//! 独立游戏运行时内核。
//!
//! 本模块读取编辑器导出的场景JSON，使用Rapier推进物理世界，
//! 并使用wgpu直接绘制场景中的矩形游戏物体。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::sync::Arc;
use std::time::Instant;

use bytemuck::{Pod, Zeroable};
use egui_wgpu::ScreenDescriptor;
use rapier2d::prelude::*;
use rodio::{Decoder, OutputStream, Sink};
use wgpu::util::DeviceExt;
use winit::dpi::LogicalSize;
use winit::event::ElementState;
use winit::event::{Event, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::WindowBuilder;

use crate::animation::{resolve_asset_path, SpriteAnimation};
use crate::app_state::{GameObject, PerformanceSettings, RuntimePerformanceReport, Scene};
use crate::blueprint::model::BlueprintNodeKind;
use crate::blueprint::vm::{
    update_blueprints_with_state, update_ui_blueprints_with_state, BlueprintInput,
    BlueprintRuntimeState, RuntimeCommand, UiCommand,
};
use crate::game_ui::{UiElement, UiElementKind};
use crate::project::{open_project, Slide2dProject, PROJECT_MAGIC};
use crate::tilemap::{TileLayerKind, TileMap, TileSet};

const RUNTIME_WIDTH: u32 = 960;
const RUNTIME_HEIGHT: u32 = 540;
const FIXED_TIME_STEP: f32 = 1.0 / 60.0;

/// GPU顶点包含二维位置和图片纹理坐标。
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    position: [f32; 2],
    texture_coordinate: [f32; 2],
}

impl Vertex {
    /// 返回wgpu读取顶点缓冲区时使用的内存布局。
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: 8,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
            ],
        }
    }
}

/// 保存一个游戏物体在runtime中使用的图片绑定组。
struct RuntimeTexture {
    bind_group: wgpu::BindGroup,
}

/// 保存一个物体在Runtime中的动画播放状态和全部帧纹理。
#[derive(Clone)]
struct RuntimeAnimation {
    source_path: String,
    animation: SpriteAnimation,
    frame_textures: Vec<Arc<RuntimeTexture>>,
    current_frame: usize,
    elapsed: f32,
}

/// 保存Runtime瓦片图集纹理和切分配置。
struct RuntimeTileSet {
    tileset: TileSet,
    texture: RuntimeTexture,
}

/// 保存Runtime顶层egui的输入、渲染器和图片面板纹理。
struct RuntimeUi {
    context: egui::Context,
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
    textures: HashMap<String, egui::TextureHandle>,
}

/// 保存一帧已经生成、等待提交给GPU的UI图元。
struct PreparedUiFrame {
    paint_jobs: Vec<egui::ClippedPrimitive>,
    textures_delta: egui::TexturesDelta,
}

/// 保存Rapier物理世界及游戏物体到刚体的对应关系。
struct PhysicsWorld {
    pipeline: PhysicsPipeline,
    gravity: Vector<Real>,
    integration_parameters: IntegrationParameters,
    island_manager: IslandManager,
    broad_phase: BroadPhase,
    narrow_phase: NarrowPhase,
    rigid_body_set: RigidBodySet,
    collider_set: ColliderSet,
    impulse_joint_set: ImpulseJointSet,
    multibody_joint_set: MultibodyJointSet,
    ccd_solver: CCDSolver,
    object_bodies: HashMap<u64, RigidBodyHandle>,
    collider_objects: HashMap<ColliderHandle, u64>,
    accumulated_time: f32,
}

impl PhysicsWorld {
    /// 根据场景中启用了碰撞体的物体创建Rapier刚体和矩形碰撞体。
    fn new(game_objects: &[GameObject]) -> Self {
        let mut world = Self {
            pipeline: PhysicsPipeline::new(),
            gravity: vector![0.0, 500.0],
            integration_parameters: IntegrationParameters {
                dt: FIXED_TIME_STEP,
                ..IntegrationParameters::default()
            },
            island_manager: IslandManager::new(),
            broad_phase: BroadPhase::new(),
            narrow_phase: NarrowPhase::new(),
            rigid_body_set: RigidBodySet::new(),
            collider_set: ColliderSet::new(),
            impulse_joint_set: ImpulseJointSet::new(),
            multibody_joint_set: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
            object_bodies: HashMap::new(),
            collider_objects: HashMap::new(),
            accumulated_time: 0.0,
        };

        for game_object in game_objects {
            let collider_config = match &game_object.collider {
                Some(config) => config,
                None => continue,
            };
            let center_x = game_object.x + game_object.width * 0.5;
            let center_y = game_object.y + game_object.height * 0.5;
            let uses_blueprint_movement = object_uses_blueprint_movement(game_object);
            let rigid_body = if collider_config.is_dynamic || uses_blueprint_movement {
                // CCD防止蓝图或键盘高速移动时穿过只有一格厚的瓦片碰撞体。
                let gravity_scale = if collider_config.is_dynamic { 1.0 } else { 0.0 };
                RigidBodyBuilder::dynamic()
                    .gravity_scale(gravity_scale)
                    .ccd_enabled(true)
            } else {
                RigidBodyBuilder::fixed()
            };
            let body_handle = world
                .rigid_body_set
                .insert(rigid_body.translation(vector![center_x, center_y]).build());
            let collider =
                ColliderBuilder::cuboid(game_object.width * 0.5, game_object.height * 0.5)
                    .friction(0.7)
                    .restitution(0.1)
                    .build();
            let collider_handle = world.collider_set.insert_with_parent(
                collider,
                body_handle,
                &mut world.rigid_body_set,
            );
            world
                .collider_objects
                .insert(collider_handle, game_object.id);
            world.object_bodies.insert(game_object.id, body_handle);
        }
        world
    }

    /// 预计算静态瓦片碰撞缓存，并把同一行连续瓦片合并成一个矩形。
    fn add_tile_colliders(&mut self, tile_map: &TileMap, tileset: &TileSet, use_cache: bool) {
        for layer in &tile_map.layers {
            let mut rows: HashMap<i32, Vec<i32>> = HashMap::new();
            for cell in &layer.cells {
                let property_collision = tileset
                    .property(cell.tile_id)
                    .map(|property| property.collision)
                    .unwrap_or(false);
                if layer.kind != TileLayerKind::Collision && !property_collision {
                    continue;
                }
                if !use_cache {
                    self.insert_tile_collider_run(tile_map, cell.y, cell.x, cell.x);
                    continue;
                }
                rows.entry(cell.y).or_default().push(cell.x);
            }
            for (row, mut columns) in rows {
                columns.sort_unstable();
                columns.dedup();
                let mut start = None;
                let mut previous = 0;
                for column in columns.into_iter().chain(std::iter::once(i32::MAX)) {
                    match start {
                        None => {
                            start = Some(column);
                            previous = column;
                        }
                        Some(_) if column == previous.saturating_add(1) => previous = column,
                        Some(run_start) => {
                            self.insert_tile_collider_run(tile_map, row, run_start, previous);
                            start = Some(column);
                            previous = column;
                        }
                    }
                }
            }
        }
    }

    /// 为一段连续横向瓦片创建单个固定刚体和矩形碰撞体。
    fn insert_tile_collider_run(
        &mut self,
        tile_map: &TileMap,
        row: i32,
        start_column: i32,
        end_column: i32,
    ) {
        if start_column == i32::MAX || end_column < start_column {
            return;
        }
        let count = end_column - start_column + 1;
        let tile_width = tile_map.tile_width as f32;
        let tile_height = tile_map.tile_height as f32;
        let center_x = (start_column as f32 + count as f32 * 0.5) * tile_width;
        let center_y = (row as f32 + 0.5) * tile_height;
        let body = self.rigid_body_set.insert(
            RigidBodyBuilder::fixed()
                .translation(vector![center_x, center_y])
                .build(),
        );
        let collider =
            ColliderBuilder::cuboid(count as f32 * tile_width * 0.5, tile_height * 0.5).build();
        self.collider_set
            .insert_with_parent(collider, body, &mut self.rigid_body_set);
    }

    /// 使用固定时间步推进物理模拟，并同步动态物体的位置。
    fn update(
        &mut self,
        elapsed_seconds: f32,
        game_objects: &mut [GameObject],
        movement_requests: &HashMap<u64, (f32, f32)>,
        viewport_width: f32,
        viewport_height: f32,
        settings: &PerformanceSettings,
    ) -> HashSet<u64> {
        self.sync_object_positions_to_bodies(
            game_objects,
            movement_requests,
            elapsed_seconds,
            viewport_width,
            viewport_height,
            settings,
        );
        self.accumulated_time += elapsed_seconds.min(0.1);
        while self.accumulated_time >= FIXED_TIME_STEP {
            self.pipeline.step(
                &self.gravity,
                &self.integration_parameters,
                &mut self.island_manager,
                &mut self.broad_phase,
                &mut self.narrow_phase,
                &mut self.rigid_body_set,
                &mut self.collider_set,
                &mut self.impulse_joint_set,
                &mut self.multibody_joint_set,
                &mut self.ccd_solver,
                None,
                &(),
                &(),
            );
            self.accumulated_time -= FIXED_TIME_STEP;
        }

        for game_object in game_objects {
            let body_handle = match self.object_bodies.get(&game_object.id) {
                Some(handle) => handle,
                None => continue,
            };
            let rigid_body = match self.rigid_body_set.get(*body_handle) {
                Some(body) if body.is_dynamic() => body,
                _ => continue,
            };
            game_object.x = rigid_body.translation().x - game_object.width * 0.5;
            game_object.y = rigid_body.translation().y - game_object.height * 0.5;
        }

        let mut collision_objects = HashSet::new();
        for contact_pair in self.narrow_phase.contact_pairs() {
            if !contact_pair.has_any_active_contact {
                continue;
            }
            if let Some(object_id) = self.collider_objects.get(&contact_pair.collider1) {
                collision_objects.insert(*object_id);
            }
            if let Some(object_id) = self.collider_objects.get(&contact_pair.collider2) {
                collision_objects.insert(*object_id);
            }
        }
        collision_objects
    }

    /// 将蓝图移动意图同步到Rapier刚体。
    ///
    /// 动态刚体必须使用速度移动，不能每帧set_translation瞬移，否则会绕过碰撞求解。
    /// 固定刚体没有运动模拟，仍可同步编辑器或其他逻辑提供的绝对坐标。
    fn sync_object_positions_to_bodies(
        &mut self,
        game_objects: &[GameObject],
        movement_requests: &HashMap<u64, (f32, f32)>,
        elapsed_seconds: f32,
        viewport_width: f32,
        viewport_height: f32,
        settings: &PerformanceSettings,
    ) {
        for game_object in game_objects {
            let body_handle = match self.object_bodies.get(&game_object.id) {
                Some(handle) => *handle,
                None => continue,
            };
            let body = match self.rigid_body_set.get_mut(body_handle) {
                Some(body) => body,
                None => continue,
            };
            if body.is_dynamic() {
                let movement = movement_requests
                    .get(&game_object.id)
                    .copied()
                    .unwrap_or((0.0, 0.0));
                let safe_delta_time = elapsed_seconds.max(0.001);
                let current_velocity = *body.linvel();
                let horizontal_velocity = movement.0 / safe_delta_time;
                let vertical_velocity = if movement.1.abs() > f32::EPSILON {
                    movement.1 / safe_delta_time
                } else {
                    current_velocity.y
                };
                let target_velocity = vector![horizontal_velocity, vertical_velocity];
                let far_away = game_object.x + game_object.width < -settings.activity_margin
                    || game_object.y + game_object.height < -settings.activity_margin
                    || game_object.x > viewport_width + settings.activity_margin
                    || game_object.y > viewport_height + settings.activity_margin;
                if settings.distant_physics_sleep
                    && far_away
                    && movement.0.abs() <= f32::EPSILON
                    && movement.1.abs() <= f32::EPSILON
                    && current_velocity.norm_squared() < 0.01
                {
                    if !body.is_sleeping() {
                        body.sleep();
                    }
                } else if (target_velocity - current_velocity).norm_squared() > 0.0001 {
                    body.set_linvel(target_velocity, true);
                }
            } else {
                let center_x = game_object.x + game_object.width * 0.5;
                let center_y = game_object.y + game_object.height * 0.5;
                body.set_translation(vector![center_x, center_y], true);
            }
        }
    }
}

/// 从JSON文件读取场景并启动独立运行时窗口。
pub fn run(scene_path: &str) -> Result<(), String> {
    apply_runtime_language_argument();
    if std::path::Path::new(scene_path)
        .extension()
        .and_then(|value| value.to_str())
        == Some("slide2d")
    {
        return run_project(scene_path);
    }
    let json_text =
        fs::read_to_string(scene_path).map_err(|error| format!("读取场景文件失败：{error}"))?;
    let scene: Scene =
        serde_json::from_str(&json_text).map_err(|error| format!("解析场景JSON失败：{error}"))?;
    let scene_directory = std::path::Path::new(scene_path)
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf();
    run_window(
        scene,
        scene_directory,
        None,
        HashMap::new(),
        HashSet::new(),
        PerformanceSettings::new(),
    )
}

/// 读取编辑器传入的语言代码，使独立Runtime沿用当前语言设置。
fn apply_runtime_language_argument() {
    let arguments: Vec<String> = std::env::args().collect();
    let code = arguments
        .windows(2)
        .find(|values| values[0] == "--locale")
        .map(|values| values[1].as_str());
    if let Some(code) = code {
        let language = if code == "en-US" {
            crate::localization::Language::English
        } else {
            crate::localization::Language::SimplifiedChinese
        };
        let _ = crate::localization::set_language(language);
    }
}

/// 读取.slide2d工程，并按照启动参数选择要运行的场景。
fn run_project(project_path: &str) -> Result<(), String> {
    let bytes = fs::read(project_path).map_err(|error| format!("读取工程文件失败：{error}"))?;
    let project: Slide2dProject =
        serde_json::from_slice(&bytes).map_err(|error| format!("解析工程文件失败：{error}"))?;
    if project.slide2d_engine != PROJECT_MAGIC {
        return Err("Runtime拒绝加载：缺少有效的Slide2D工程标识".to_owned());
    }
    let arguments: Vec<String> = std::env::args().collect();
    let requested_scene = arguments
        .windows(2)
        .find(|values| values[0] == "--scene")
        .map(|values| values[1].clone())
        .unwrap_or_else(|| project.startup_scene_name.clone());
    let scene = project
        .scenes
        .iter()
        .find(|scene| scene.name == requested_scene)
        .cloned()
        .or_else(|| project.scenes.first().cloned())
        .ok_or_else(|| "工程中没有场景".to_owned())?;
    let state = open_project(std::path::Path::new(project_path))?;
    let global_variables = arguments
        .windows(2)
        .find(|values| values[0] == "--globals")
        .and_then(|values| serde_json::from_str(&values[1]).ok())
        .unwrap_or(project.global_variables);
    let enabled_plugins = project.enabled_plugins.clone();
    let performance_settings = project.performance_settings.clone();
    run_window(
        scene,
        state.project_root,
        Some(project_path.to_owned()),
        global_variables,
        enabled_plugins,
        performance_settings,
    )
}

/// 创建运行时窗口、GPU资源和物理世界，并进入游戏循环。
fn run_window(
    mut scene: Scene,
    scene_directory: std::path::PathBuf,
    project_path: Option<String>,
    initial_global_variables: HashMap<String, f32>,
    enabled_plugins: HashSet<String>,
    performance_settings: PerformanceSettings,
) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|error| error.to_string())?;
    let window = WindowBuilder::new()
        .with_title("Slide2D Runtime")
        .with_inner_size(LogicalSize::new(RUNTIME_WIDTH, RUNTIME_HEIGHT))
        .build(&event_loop)
        .map_err(|error| error.to_string())?;
    let window = Arc::new(window);

    let instance = wgpu::Instance::default();
    let surface = instance
        .create_surface(window.clone())
        .map_err(|error| error.to_string())?;
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: Some(&surface),
        force_fallback_adapter: false,
    }))
    .ok_or("没有找到可用的图形适配器")?;
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("Slide2D Runtime Device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
        },
        None,
    ))
    .map_err(|error| error.to_string())?;

    let surface_capabilities = surface.get_capabilities(&adapter);
    let surface_format = surface_capabilities.formats[0];
    let window_size = window.inner_size();
    let mut surface_config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: surface_format,
        width: window_size.width.max(1),
        height: window_size.height.max(1),
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: surface_capabilities.alpha_modes[0],
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    surface.configure(&device, &surface_config);
    let runtime_ui_context = egui::Context::default();
    configure_runtime_chinese_fonts(&runtime_ui_context);
    let runtime_ui_state = egui_winit::State::new(
        runtime_ui_context.clone(),
        egui::ViewportId::ROOT,
        window.as_ref(),
        None,
        None,
    );
    let runtime_ui_renderer = egui_wgpu::Renderer::new(&device, surface_format, None, 1);
    let mut runtime_ui = RuntimeUi {
        context: runtime_ui_context,
        state: runtime_ui_state,
        renderer: runtime_ui_renderer,
        textures: HashMap::new(),
    };

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Slide2D Rectangle Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("rectangle.wgsl").into()),
    });
    let texture_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Slide2D Texture Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Slide2D Runtime Pipeline Layout"),
        bind_group_layouts: &[&texture_bind_group_layout],
        push_constant_ranges: &[],
    });
    let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Slide2D Runtime Pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: "vertex_main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Vertex::layout()],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: "fragment_main",
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
    });

    scene.game_objects.sort_by_key(|object| object.layer_index);
    let mut runtime_textures = load_runtime_textures(
        &device,
        &queue,
        &texture_bind_group_layout,
        &scene.game_objects,
        &scene_directory,
        performance_settings.resource_cache,
    );
    let mut runtime_animations = load_runtime_animations(
        &device,
        &queue,
        &texture_bind_group_layout,
        &scene.game_objects,
        &scene_directory,
    );
    let runtime_tileset = load_runtime_tileset(
        &device,
        &queue,
        &texture_bind_group_layout,
        &scene.tile_map,
        &scene_directory,
    );
    let mut runtime_tile_buffers = create_tile_buffers(
        &device,
        &scene.tile_map,
        runtime_tileset.as_ref(),
        &surface_config,
        &performance_settings,
    );
    // OutputStream必须在整个事件循环期间保持存活，否则音效会立即停止。
    let _audio_stream = play_scene_audio(&scene.game_objects, &scene_directory);
    let mut physics_world = PhysicsWorld::new(&scene.game_objects);
    if let Some(runtime_tileset) = &runtime_tileset {
        physics_world.add_tile_colliders(
            &scene.tile_map,
            &runtime_tileset.tileset,
            performance_settings.static_physics_cache,
        );
    }
    let mut previous_time = Instant::now();
    let mut blueprint_input = BlueprintInput::new();
    blueprint_input.enabled_plugins = enabled_plugins;
    blueprint_input.dormant_blueprints_enabled = performance_settings.dormant_blueprints;
    blueprint_input.blueprint_cache_enabled = performance_settings.blueprint_cache;
    let mut global_variables = initial_global_variables;
    let mut blueprint_runtime_state = BlueprintRuntimeState::new();
    let mut last_cursor_position = (0.0_f32, 0.0_f32);
    let mut runtime_fps = 0.0_f32;
    let mut last_report_time = Instant::now();

    event_loop
        .run(move |event, event_loop_window_target| {
            event_loop_window_target.set_control_flow(ControlFlow::Poll);
            match event {
                Event::WindowEvent { event, window_id } if window_id == window.id() => {
                    let egui_response = runtime_ui.state.on_window_event(window.as_ref(), &event);
                    if egui_response.repaint {
                        window.request_redraw();
                    }
                    match event {
                        WindowEvent::CloseRequested => event_loop_window_target.exit(),
                        WindowEvent::Resized(new_size) => {
                            if new_size.width > 0 && new_size.height > 0 {
                                surface_config.width = new_size.width;
                                surface_config.height = new_size.height;
                                surface.configure(&device, &surface_config);
                                runtime_tile_buffers = create_tile_buffers(
                                    &device,
                                    &scene.tile_map,
                                    runtime_tileset.as_ref(),
                                    &surface_config,
                                    &performance_settings,
                                );
                            }
                        }
                        WindowEvent::KeyboardInput { event, .. } => {
                            if let PhysicalKey::Code(key_code) = event.physical_key {
                                blueprint_input.set_key_down(
                                    format!("{key_code:?}"),
                                    event.state == ElementState::Pressed,
                                );
                            }
                        }
                        WindowEvent::CursorMoved { position, .. } => {
                            let scale = window.scale_factor() as f32;
                            last_cursor_position =
                                (position.x as f32 / scale, position.y as f32 / scale);
                        }
                        WindowEvent::MouseInput {
                            state: ElementState::Pressed,
                            button: winit::event::MouseButton::Left,
                            ..
                        } => {
                            let clicked = find_clicked_objects(
                                &scene.game_objects,
                                last_cursor_position.0,
                                last_cursor_position.1,
                            );
                            blueprint_input.set_clicked_object_ids(clicked);
                        }
                        WindowEvent::Focused(false) => {
                            blueprint_input.clear();
                        }
                        WindowEvent::RedrawRequested => {
                            let current_time = Instant::now();
                            let elapsed_seconds =
                                current_time.duration_since(previous_time).as_secs_f32();
                            previous_time = current_time;
                            blueprint_input.set_tiles_under_objects(detect_tiles_under_objects(
                                &scene.game_objects,
                                &scene.tile_map,
                            ));
                            let (prepared_ui, clicked_ui_ids) = prepare_runtime_ui_frame(
                                &mut runtime_ui,
                                window.as_ref(),
                                &scene.ui_elements,
                                &scene_directory,
                                &performance_settings,
                            );
                            blueprint_input.set_clicked_ui_ids(clicked_ui_ids);
                            blueprint_input.active_object_ids = active_runtime_object_ids(
                                &scene.game_objects,
                                surface_config.width as f32,
                                surface_config.height as f32,
                                performance_settings.activity_margin,
                            );
                            let positions_before_blueprints: HashMap<u64, (f32, f32)> = scene
                                .game_objects
                                .iter()
                                .map(|object| (object.id, (object.x, object.y)))
                                .collect();
                            let blueprint_started = Instant::now();
                            let mut runtime_commands = update_blueprints_with_state(
                                &mut scene.game_objects,
                                &blueprint_input,
                                elapsed_seconds.min(0.1),
                                &mut global_variables,
                                &mut blueprint_runtime_state,
                            );
                            let movement_requests = take_dynamic_movement_requests(
                                &mut scene.game_objects,
                                &positions_before_blueprints,
                            );
                            runtime_commands.extend(update_ui_blueprints_with_state(
                                &scene.ui_elements,
                                &blueprint_input,
                                elapsed_seconds.min(0.1),
                                &mut global_variables,
                                &mut blueprint_runtime_state,
                            ));
                            let blueprint_time_ms =
                                blueprint_started.elapsed().as_secs_f32() * 1000.0;
                            let mut switch_scene_name = None;
                            let mut spawn_commands = Vec::new();
                            let mut destroy_ids = Vec::new();
                            let all_ui_commands: Vec<UiCommand> = runtime_commands
                                .into_iter()
                                .filter_map(|command| match command {
                                    RuntimeCommand::Ui(command) => Some(command),
                                    RuntimeCommand::SwitchScene(name) => {
                                        switch_scene_name = Some(name);
                                        None
                                    }
                                    RuntimeCommand::SpawnObject {
                                        template_object_id,
                                        x,
                                        y,
                                    } => {
                                        spawn_commands.push((template_object_id, x, y));
                                        None
                                    }
                                    RuntimeCommand::DestroyObject(id) => {
                                        destroy_ids.push(id);
                                        None
                                    }
                                })
                                .collect();
                            apply_ui_commands(&mut scene.ui_elements, all_ui_commands);
                            let objects_changed = apply_object_commands(
                                &mut scene.game_objects,
                                spawn_commands,
                                destroy_ids,
                            );
                            if objects_changed {
                                runtime_textures = load_runtime_textures(
                                    &device,
                                    &queue,
                                    &texture_bind_group_layout,
                                    &scene.game_objects,
                                    &scene_directory,
                                    performance_settings.resource_cache,
                                );
                                runtime_animations = load_runtime_animations(
                                    &device,
                                    &queue,
                                    &texture_bind_group_layout,
                                    &scene.game_objects,
                                    &scene_directory,
                                );
                                physics_world = PhysicsWorld::new(&scene.game_objects);
                                if let Some(runtime_tileset) = &runtime_tileset {
                                    physics_world.add_tile_colliders(
                                        &scene.tile_map,
                                        &runtime_tileset.tileset,
                                        performance_settings.static_physics_cache,
                                    );
                                }
                            }
                            if let Some(scene_name) = switch_scene_name {
                                if let Some(project_path) = &project_path {
                                    if launch_switched_scene(
                                        project_path,
                                        &scene_name,
                                        &global_variables,
                                    )
                                    .is_ok()
                                    {
                                        event_loop_window_target.exit();
                                        return;
                                    }
                                }
                            }
                            blueprint_input.scene_just_loaded = false;
                            // 蓝图先提出移动需求，再让Rapier在同一帧进行瓦片碰撞求解。
                            // 渲染使用物理修正后的坐标，因此玩家不会直接穿过碰撞瓦片。
                            let physics_started = Instant::now();
                            let collision_objects = physics_world.update(
                                elapsed_seconds,
                                &mut scene.game_objects,
                                &movement_requests,
                                surface_config.width as f32,
                                surface_config.height as f32,
                                &performance_settings,
                            );
                            let physics_time_ms = physics_started.elapsed().as_secs_f32() * 1000.0;
                            blueprint_input.set_collision_objects(collision_objects);
                            update_runtime_animations(
                                &device,
                                &queue,
                                &texture_bind_group_layout,
                                &scene.game_objects,
                                &scene_directory,
                                &mut runtime_animations,
                                elapsed_seconds.min(0.1),
                            );
                            draw_runtime_frame(
                                &surface,
                                &device,
                                &queue,
                                &surface_config,
                                &render_pipeline,
                                &scene.game_objects,
                                &runtime_textures,
                                &runtime_animations,
                                runtime_tileset.as_ref(),
                                &runtime_tile_buffers,
                                &mut runtime_ui,
                                prepared_ui,
                                window.scale_factor() as f32,
                                &performance_settings,
                            );
                            let current_fps = 1.0 / elapsed_seconds.max(0.0001);
                            runtime_fps = if runtime_fps <= 0.0 {
                                current_fps
                            } else {
                                runtime_fps * 0.9 + current_fps * 0.1
                            };
                            if last_report_time.elapsed().as_millis() >= 500 {
                                let (rendered_objects, rendered_tiles) =
                                    count_visible_runtime_items(
                                        &scene.game_objects,
                                        &scene.tile_map,
                                        surface_config.width as f32,
                                        surface_config.height as f32,
                                        &performance_settings,
                                    );
                                write_runtime_performance_report(RuntimePerformanceReport {
                                    slide2d_engine: "SLIDE2D_PERFORMANCE_SYSTEM".to_owned(),
                                    frame_rate: runtime_fps,
                                    memory_bytes: estimate_runtime_cache_bytes(
                                        &scene,
                                        &runtime_animations,
                                    ),
                                    cached_assets: runtime_textures.len()
                                        + runtime_animations.len()
                                        + runtime_ui.textures.len(),
                                    rendered_objects,
                                    rendered_tiles,
                                    blueprint_time_ms,
                                    physics_time_ms,
                                });
                                last_report_time = Instant::now();
                            }
                            blueprint_input.set_clicked_ui_ids(HashSet::new());
                            blueprint_input.set_clicked_object_ids(HashSet::new());
                        }
                        _ => {}
                    }
                }
                Event::AboutToWait => window.request_redraw(),
                _ => {}
            }
        })
        .map_err(|error| error.to_string())
}

/// 提取蓝图对动态刚体产生的位移，并恢复物体位置交给Rapier安全移动。
fn take_dynamic_movement_requests(
    game_objects: &mut [GameObject],
    positions_before_blueprints: &HashMap<u64, (f32, f32)>,
) -> HashMap<u64, (f32, f32)> {
    let mut requests = HashMap::new();
    for object in game_objects {
        let is_dynamic = object
            .collider
            .as_ref()
            .map(|collider| collider.is_dynamic || object_uses_blueprint_movement(object))
            .unwrap_or(false);
        if !is_dynamic {
            continue;
        }
        let (old_x, old_y) = match positions_before_blueprints.get(&object.id) {
            Some(position) => *position,
            None => continue,
        };
        let delta_x = object.x - old_x;
        let delta_y = object.y - old_y;
        object.x = old_x;
        object.y = old_y;
        requests.insert(object.id, (delta_x, delta_y));
    }
    requests
}

/// 判断对象蓝图是否包含按帧修改位置的节点。
fn object_uses_blueprint_movement(object: &GameObject) -> bool {
    object.blueprint.nodes.iter().any(|node| {
        matches!(node.kind, BlueprintNodeKind::ModifyPosition { .. })
            || matches!(
                node.kind,
                BlueprintNodeKind::PluginNode {
                    behavior: crate::plugins::PluginBehavior::MoveHorizontal,
                    ..
                }
            )
    })
}

/// 将当前场景转换为顶点并提交一帧GPU绘制命令。
fn draw_runtime_frame(
    surface: &wgpu::Surface,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    surface_config: &wgpu::SurfaceConfiguration,
    render_pipeline: &wgpu::RenderPipeline,
    game_objects: &[GameObject],
    runtime_textures: &HashMap<u64, Arc<RuntimeTexture>>,
    runtime_animations: &HashMap<u64, RuntimeAnimation>,
    runtime_tileset: Option<&RuntimeTileSet>,
    tile_buffers: &[wgpu::Buffer],
    runtime_ui: &mut RuntimeUi,
    prepared_ui: PreparedUiFrame,
    pixels_per_point: f32,
    performance_settings: &PerformanceSettings,
) {
    let surface_texture = match surface.get_current_texture() {
        Ok(texture) => texture,
        Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
            surface.configure(device, surface_config);
            return;
        }
        Err(_) => return,
    };
    let mut object_buffers = Vec::new();
    let visible_objects: Vec<&GameObject> = game_objects
        .iter()
        .filter(|object| {
            !performance_settings.viewport_culling
                || runtime_object_visible(
                    object,
                    surface_config.width as f32,
                    surface_config.height as f32,
                )
        })
        .collect();
    for game_object in &visible_objects {
        let vertices =
            create_object_vertices(game_object, surface_config.width, surface_config.height);
        object_buffers.push(
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Slide2D Object Vertices"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
        );
    }
    let texture_view = surface_texture
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    let mut command_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Slide2D Runtime Encoder"),
    });
    {
        let mut render_pass = command_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Slide2D Runtime Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &texture_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.12,
                        g: 0.13,
                        b: 0.15,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        render_pass.set_pipeline(render_pipeline);
        if let Some(runtime_tileset) = runtime_tileset {
            render_pass.set_bind_group(0, &runtime_tileset.texture.bind_group, &[]);
            for buffer in tile_buffers {
                render_pass.set_vertex_buffer(0, buffer.slice(..));
                render_pass.draw(0..6, 0..1);
            }
        }
        for (index, game_object) in visible_objects.iter().enumerate() {
            let runtime_texture = runtime_animations
                .get(&game_object.id)
                .and_then(|animation| animation.frame_textures.get(animation.current_frame))
                .or_else(|| runtime_textures.get(&game_object.id));
            let runtime_texture = match runtime_texture {
                Some(texture) => texture,
                None => continue,
            };
            render_pass.set_bind_group(0, &runtime_texture.bind_group, &[]);
            render_pass.set_vertex_buffer(0, object_buffers[index].slice(..));
            render_pass.draw(0..6, 0..1);
        }
    }
    let screen_descriptor = ScreenDescriptor {
        size_in_pixels: [surface_config.width, surface_config.height],
        pixels_per_point,
    };
    for (texture_id, image_delta) in &prepared_ui.textures_delta.set {
        runtime_ui
            .renderer
            .update_texture(device, queue, *texture_id, image_delta);
    }
    runtime_ui.renderer.update_buffers(
        device,
        queue,
        &mut command_encoder,
        &prepared_ui.paint_jobs,
        &screen_descriptor,
    );
    {
        let mut ui_pass = command_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Slide2D Runtime UI Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &texture_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    // UI覆盖在已经完成的游戏画面上，不能再次清屏。
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        runtime_ui
            .renderer
            .render(&mut ui_pass, &prepared_ui.paint_jobs, &screen_descriptor);
    }
    queue.submit(Some(command_encoder.finish()));
    surface_texture.present();
    for texture_id in &prepared_ui.textures_delta.free {
        runtime_ui.renderer.free_texture(texture_id);
    }
}

/// 根据场景UI数据生成本帧egui图元，并返回点击的按钮ID。
fn prepare_runtime_ui_frame(
    runtime_ui: &mut RuntimeUi,
    window: &winit::window::Window,
    elements: &[UiElement],
    scene_directory: &std::path::Path,
    performance_settings: &PerformanceSettings,
) -> (PreparedUiFrame, HashSet<u64>) {
    let raw_input = runtime_ui.state.take_egui_input(window);
    let context = runtime_ui.context.clone();
    let textures = &mut runtime_ui.textures;
    if performance_settings.idle_cache_release {
        let active_paths: HashSet<String> = elements
            .iter()
            .filter_map(|element| match &element.kind {
                UiElementKind::ImagePanel { image_path } if !image_path.is_empty() => Some(
                    resolve_runtime_asset_path(scene_directory, image_path)
                        .to_string_lossy()
                        .into_owned(),
                ),
                _ => None,
            })
            .collect();
        textures.retain(|path, _| active_paths.contains(path));
    }
    let mut clicked_ids = HashSet::new();
    let mut sorted_elements: Vec<&UiElement> = elements.iter().collect();
    sorted_elements.sort_by_key(|element| element.layer_index);
    let output = context.run(raw_input, |context| {
        for element in &sorted_elements {
            if !element.visible {
                continue;
            }
            if performance_settings.viewport_culling {
                let screen = context.screen_rect();
                let outside = element.x + element.width < 0.0
                    || element.y + element.height < 0.0
                    || element.x > screen.width()
                    || element.y > screen.height();
                if outside {
                    continue;
                }
            }
            egui::Area::new(egui::Id::new(("runtime_ui", element.id)))
                .order(egui::Order::Foreground)
                .fixed_pos(egui::Pos2::new(element.x, element.y))
                .show(context, |ui| {
                    draw_runtime_ui_element(
                        ui,
                        element,
                        scene_directory,
                        textures,
                        &mut clicked_ids,
                    );
                });
        }
        draw_runtime_watermark(context);
    });
    runtime_ui
        .state
        .handle_platform_output(window, output.platform_output);
    let paint_jobs = context.tessellate(output.shapes, output.pixels_per_point);
    (
        PreparedUiFrame {
            paint_jobs,
            textures_delta: output.textures_delta,
        },
        clicked_ids,
    )
}

/// 在游戏运行窗口右下角永久绘制Slide2D半透明浅水印。
///
/// 水印属于Runtime系统图层，不保存进场景，也不会被用户蓝图删除或隐藏。
fn draw_runtime_watermark(context: &egui::Context) {
    let painter = context.layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        egui::Id::new("slide2d_runtime_watermark"),
    ));
    let position = context.screen_rect().right_bottom() - egui::Vec2::new(14.0, 10.0);
    painter.text(
        position,
        egui::Align2::RIGHT_BOTTOM,
        "Made by Slide2D",
        egui::FontId::proportional(14.0),
        egui::Color32::from_white_alpha(105),
    );
}

/// 绘制一个Runtime UI元素，并收集按钮点击事件。
fn draw_runtime_ui_element(
    ui: &mut egui::Ui,
    element: &UiElement,
    scene_directory: &std::path::Path,
    textures: &mut HashMap<String, egui::TextureHandle>,
    clicked_ids: &mut HashSet<u64>,
) {
    let size = egui::Vec2::new(element.width, element.height);
    match &element.kind {
        UiElementKind::Text {
            content,
            font_size,
            color,
        } => {
            let text = egui::RichText::new(content)
                .size(*font_size)
                .color(runtime_color(*color));
            ui.add_sized(size, egui::Label::new(text));
        }
        UiElementKind::Button { text } => {
            if ui.add_sized(size, egui::Button::new(text)).clicked() {
                clicked_ids.insert(element.id);
            }
        }
        UiElementKind::ProgressBar {
            maximum,
            value,
            background_color,
            fill_color,
        } => {
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            ui.painter()
                .rect_filled(rect, 3.0, runtime_color(*background_color));
            let ratio = if *maximum <= 0.0 {
                0.0
            } else {
                (*value / *maximum).clamp(0.0, 1.0)
            };
            let fill_rect = egui::Rect::from_min_size(
                rect.min,
                egui::Vec2::new(rect.width() * ratio, rect.height()),
            );
            ui.painter()
                .rect_filled(fill_rect, 3.0, runtime_color(*fill_color));
        }
        UiElementKind::ImagePanel { image_path } => {
            let path = resolve_runtime_asset_path(scene_directory, image_path);
            if let Some(texture) = load_runtime_egui_texture(ui.ctx(), textures, &path) {
                ui.add(egui::Image::new(&texture).fit_to_exact_size(size));
            }
        }
    }
}

/// 加载Runtime图片面板使用的egui纹理，并处理GPU尺寸上限。
fn load_runtime_egui_texture(
    context: &egui::Context,
    textures: &mut HashMap<String, egui::TextureHandle>,
    path: &std::path::Path,
) -> Option<egui::TextureHandle> {
    let key = path.to_string_lossy().into_owned();
    if let Some(texture) = textures.get(&key) {
        return Some(texture.clone());
    }
    let image = image::open(path).ok()?;
    let maximum = context.input(|input| input.max_texture_side).max(1) as u32;
    let image = if image.width() > maximum || image.height() > maximum {
        image.resize(maximum, maximum, image::imageops::FilterType::Triangle)
    } else {
        image
    };
    let image = image.to_rgba8();
    let color_image = egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    );
    let texture = context.load_texture(key.clone(), color_image, egui::TextureOptions::LINEAR);
    textures.insert(key, texture.clone());
    Some(texture)
}

/// 应用蓝图VM输出的UI命令。
fn apply_ui_commands(elements: &mut [UiElement], commands: Vec<UiCommand>) {
    for command in commands {
        match command {
            UiCommand::SetText { ui_id, content } => {
                if let Some(element) = elements.iter_mut().find(|element| element.id == ui_id) {
                    match &mut element.kind {
                        UiElementKind::Text { content: text, .. } => *text = content,
                        UiElementKind::Button { text } => *text = content,
                        _ => {}
                    }
                }
            }
            UiCommand::SetProgress { ui_id, value } => {
                if let Some(element) = elements.iter_mut().find(|element| element.id == ui_id) {
                    if let UiElementKind::ProgressBar {
                        maximum,
                        value: current,
                        ..
                    } = &mut element.kind
                    {
                        *current = value.clamp(0.0, maximum.max(0.0));
                    }
                }
            }
            UiCommand::SetVisible { ui_id, visible } => {
                if let Some(element) = elements.iter_mut().find(|element| element.id == ui_id) {
                    element.visible = visible;
                }
            }
        }
    }
}

/// 返回鼠标位置命中的最高图层物体。
fn find_clicked_objects(game_objects: &[GameObject], x: f32, y: f32) -> HashSet<u64> {
    let object = game_objects
        .iter()
        .filter(|object| {
            x >= object.x
                && x <= object.x + object.width
                && y >= object.y
                && y <= object.y + object.height
        })
        .max_by_key(|object| object.layer_index);
    object
        .map(|value| HashSet::from([value.id]))
        .unwrap_or_default()
}

/// 在一帧结束时生成或销毁物体，避免蓝图遍历期间修改数组。
fn apply_object_commands(
    game_objects: &mut Vec<GameObject>,
    spawn_commands: Vec<(u64, f32, f32)>,
    destroy_ids: Vec<u64>,
) -> bool {
    let changed = !spawn_commands.is_empty() || !destroy_ids.is_empty();
    game_objects.retain(|object| !destroy_ids.contains(&object.id));
    let mut next_id = game_objects
        .iter()
        .map(|object| object.id)
        .max()
        .unwrap_or(0)
        + 1;
    let templates = game_objects.clone();
    for (template_id, x, y) in spawn_commands {
        if let Some(mut object) = templates
            .iter()
            .find(|object| object.id == template_id)
            .cloned()
        {
            object.id = next_id;
            object.x = x;
            object.y = y;
            object.blueprint_file = format!("blueprint_{next_id}.json");
            game_objects.push(object);
            next_id += 1;
        }
    }
    changed
}

/// 启动同一工程的目标场景，旧Runtime随后安全退出。
fn launch_switched_scene(
    project_path: &str,
    scene_name: &str,
    global_variables: &HashMap<String, f32>,
) -> Result<(), String> {
    let executable =
        std::env::current_exe().map_err(|error| format!("定位Runtime失败：{error}"))?;
    std::process::Command::new(executable)
        .arg("--runtime")
        .arg(project_path)
        .arg("--scene")
        .arg(scene_name)
        .arg("--locale")
        .arg(crate::localization::current_language().code())
        .arg("--globals")
        .arg(
            serde_json::to_string(global_variables)
                .map_err(|error| format!("保存全局变量失败：{error}"))?,
        )
        .spawn()
        .map_err(|error| format!("切换场景失败：{error}"))?;
    Ok(())
}

/// 转换序列化RGBA颜色为egui颜色。
fn runtime_color(color: [u8; 4]) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3])
}

/// Runtime加载Windows中文字体，确保中文文本和按钮正常显示。
fn configure_runtime_chinese_fonts(context: &egui::Context) {
    for path in [
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\simsun.ttc",
    ] {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        let mut definitions = egui::FontDefinitions::default();
        definitions.font_data.insert(
            "runtime_chinese".to_owned(),
            egui::FontData::from_owned(bytes),
        );
        definitions
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "runtime_chinese".to_owned());
        context.set_fonts(definitions);
        return;
    }
}

/// 加载瓦片集JSON和图集纹理。
fn load_runtime_tileset(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    tile_map: &TileMap,
    scene_directory: &std::path::Path,
) -> Option<RuntimeTileSet> {
    if tile_map.tileset_path.is_empty() {
        return None;
    }
    let tileset = if let Some(tileset) = &tile_map.tileset {
        tileset.clone()
    } else {
        let tileset_path = resolve_runtime_asset_path(scene_directory, &tile_map.tileset_path);
        TileSet::load(&tileset_path).ok()?
    };
    let image_path = resolve_asset_path(scene_directory, &tileset.image_path);
    let image = image::open(image_path).ok()?.to_rgba8();
    let texture = create_runtime_texture(
        device,
        queue,
        layout,
        image.width(),
        image.height(),
        image.as_raw(),
    );
    Some(RuntimeTileSet { tileset, texture })
}

/// 为全部可见瓦片创建GPU顶点缓冲区。
fn create_tile_buffers(
    device: &wgpu::Device,
    tile_map: &TileMap,
    runtime_tileset: Option<&RuntimeTileSet>,
    surface_config: &wgpu::SurfaceConfiguration,
    performance_settings: &PerformanceSettings,
) -> Vec<wgpu::Buffer> {
    let runtime_tileset = match runtime_tileset {
        Some(tileset) => tileset,
        None => return Vec::new(),
    };
    let mut buffers = Vec::new();
    for layer in &tile_map.layers {
        if !layer.visible {
            continue;
        }
        for cell in &layer.cells {
            if performance_settings.tile_chunk_culling
                && !runtime_tile_chunk_visible(
                    cell.x,
                    cell.y,
                    tile_map,
                    surface_config.width as f32,
                    surface_config.height as f32,
                )
            {
                continue;
            }
            if runtime_tileset
                .tileset
                .property(cell.tile_id)
                .map(|property| property.transparent)
                .unwrap_or(false)
            {
                continue;
            }
            let vertices = create_tile_vertices(
                tile_map,
                &runtime_tileset.tileset,
                cell.x,
                cell.y,
                cell.tile_id,
                surface_config.width,
                surface_config.height,
            );
            buffers.push(
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Slide2D Tile Vertices"),
                    contents: bytemuck::cast_slice(&vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
            );
        }
    }
    buffers
}

/// 判断Runtime物体矩形是否与当前窗口相交。
fn runtime_object_visible(object: &GameObject, width: f32, height: f32) -> bool {
    object.x + object.width >= 0.0
        && object.y + object.height >= 0.0
        && object.x <= width
        && object.y <= height
}

/// 判断瓦片所在16x16区块是否与Runtime窗口相交。
fn runtime_tile_chunk_visible(
    tile_x: i32,
    tile_y: i32,
    tile_map: &TileMap,
    width: f32,
    height: f32,
) -> bool {
    const CHUNK_SIZE: i32 = 16;
    let left = tile_x.div_euclid(CHUNK_SIZE) as f32
        * CHUNK_SIZE as f32
        * tile_map.tile_width.max(1) as f32;
    let top = tile_y.div_euclid(CHUNK_SIZE) as f32
        * CHUNK_SIZE as f32
        * tile_map.tile_height.max(1) as f32;
    let right = left + CHUNK_SIZE as f32 * tile_map.tile_width.max(1) as f32;
    let bottom = top + CHUNK_SIZE as f32 * tile_map.tile_height.max(1) as f32;
    right >= 0.0 && bottom >= 0.0 && left <= width && top <= height
}

/// 返回Runtime活动视口和扩展边距内的对象ID集合。
fn active_runtime_object_ids(
    objects: &[GameObject],
    width: f32,
    height: f32,
    margin: f32,
) -> HashSet<u64> {
    objects
        .iter()
        .filter(|object| {
            object.x + object.width >= -margin
                && object.y + object.height >= -margin
                && object.x <= width + margin
                && object.y <= height + margin
        })
        .map(|object| object.id)
        .collect()
}

/// 统计当前优化设置下真正提交渲染的物体和瓦片数量。
fn count_visible_runtime_items(
    objects: &[GameObject],
    tile_map: &TileMap,
    width: f32,
    height: f32,
    settings: &PerformanceSettings,
) -> (usize, usize) {
    let objects_count = objects
        .iter()
        .filter(|object| {
            !settings.viewport_culling || runtime_object_visible(object, width, height)
        })
        .count();
    let tiles_count = tile_map
        .layers
        .iter()
        .filter(|layer| layer.visible)
        .flat_map(|layer| &layer.cells)
        .filter(|cell| {
            !settings.tile_chunk_culling
                || runtime_tile_chunk_visible(cell.x, cell.y, tile_map, width, height)
        })
        .count();
    (objects_count, tiles_count)
}

/// 根据场景图片尺寸和动画帧数估算Runtime资源缓存内存。
fn estimate_runtime_cache_bytes(scene: &Scene, animations: &HashMap<u64, RuntimeAnimation>) -> u64 {
    let static_estimate = scene
        .game_objects
        .iter()
        .map(|object| (object.width.max(1.0) * object.height.max(1.0) * 4.0) as u64)
        .sum::<u64>();
    let animation_estimate = animations
        .values()
        .map(|animation| animation.frame_textures.len() as u64 * 256 * 256 * 4)
        .sum::<u64>();
    static_estimate + animation_estimate
}

/// 将Runtime性能指标写入临时JSON，编辑器性能监视器可实时读取。
fn write_runtime_performance_report(report: RuntimePerformanceReport) {
    if let Ok(bytes) = serde_json::to_vec_pretty(&report) {
        let _ = fs::write(
            std::env::temp_dir().join("slide2d_runtime_performance.json"),
            bytes,
        );
    }
}

/// 将单个瓦片转换为带图集UV的两个三角形。
fn create_tile_vertices(
    tile_map: &TileMap,
    tileset: &TileSet,
    tile_x: i32,
    tile_y: i32,
    tile_id: u32,
    width: u32,
    height: u32,
) -> [Vertex; 6] {
    let x = tile_x as f32 * tile_map.tile_width as f32;
    let y = tile_y as f32 * tile_map.tile_height as f32;
    let left = pixel_to_clip_x(x, width);
    let right = pixel_to_clip_x(x + tile_map.tile_width as f32, width);
    let top = pixel_to_clip_y(y, height);
    let bottom = pixel_to_clip_y(y + tile_map.tile_height as f32, height);
    let column = tile_id % tileset.columns.max(1);
    let row = tile_id / tileset.columns.max(1);
    let u0 = column as f32 / tileset.columns.max(1) as f32;
    let v0 = row as f32 / tileset.rows.max(1) as f32;
    let u1 = (column + 1) as f32 / tileset.columns.max(1) as f32;
    let v1 = (row + 1) as f32 / tileset.rows.max(1) as f32;
    [
        Vertex {
            position: [left, top],
            texture_coordinate: [u0, v0],
        },
        Vertex {
            position: [left, bottom],
            texture_coordinate: [u0, v1],
        },
        Vertex {
            position: [right, bottom],
            texture_coordinate: [u1, v1],
        },
        Vertex {
            position: [left, top],
            texture_coordinate: [u0, v0],
        },
        Vertex {
            position: [right, bottom],
            texture_coordinate: [u1, v1],
        },
        Vertex {
            position: [right, top],
            texture_coordinate: [u1, v0],
        },
    ]
}

/// 查询每个物体脚下地面层或碰撞层的瓦片ID。
fn detect_tiles_under_objects(
    game_objects: &[GameObject],
    tile_map: &TileMap,
) -> HashMap<u64, i32> {
    let mut result = HashMap::new();
    for object in game_objects {
        let foot_x = object.x + object.width * 0.5;
        let foot_y = object.y + object.height + 1.0;
        let tile_x = (foot_x / tile_map.tile_width.max(1) as f32).floor() as i32;
        let tile_y = (foot_y / tile_map.tile_height.max(1) as f32).floor() as i32;
        let tile_id = tile_map
            .layer(TileLayerKind::Collision)
            .and_then(|layer| layer.tile_at(tile_x, tile_y))
            .or_else(|| {
                tile_map
                    .layer(TileLayerKind::Ground)
                    .and_then(|layer| layer.tile_at(tile_x, tile_y))
            })
            .map(|id| id as i32)
            .unwrap_or(-1);
        result.insert(object.id, tile_id);
    }
    result
}

/// 加载所有物体初始绑定的动画资源和序列帧纹理。
fn load_runtime_animations(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    game_objects: &[GameObject],
    scene_directory: &std::path::Path,
) -> HashMap<u64, RuntimeAnimation> {
    let mut animations = HashMap::new();
    let mut shared: HashMap<String, RuntimeAnimation> = HashMap::new();
    for object in game_objects {
        if object.animation_path.is_empty() {
            continue;
        }
        let animation = if let Some(animation) = shared.get(&object.animation_path) {
            Some(animation.clone())
        } else {
            let loaded = load_runtime_animation(
                device,
                queue,
                layout,
                scene_directory,
                &object.animation_path,
            );
            if let Some(animation) = &loaded {
                shared.insert(object.animation_path.clone(), animation.clone());
            }
            loaded
        };
        if let Some(animation) = animation {
            animations.insert(object.id, animation);
        }
    }
    animations
}

/// 每帧推进动画，并在蓝图切换动画路径后重新加载资源。
fn update_runtime_animations(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    game_objects: &[GameObject],
    scene_directory: &std::path::Path,
    animations: &mut HashMap<u64, RuntimeAnimation>,
    delta_time: f32,
) {
    for object in game_objects {
        let needs_reload = !object.animation_path.is_empty()
            && animations
                .get(&object.id)
                .map(|animation| animation.source_path != object.animation_path)
                .unwrap_or(true);
        if needs_reload {
            if let Some(animation) = load_runtime_animation(
                device,
                queue,
                layout,
                scene_directory,
                &object.animation_path,
            ) {
                animations.insert(object.id, animation);
            }
        }
        if object.animation_path.is_empty() {
            animations.remove(&object.id);
            continue;
        }
        let animation = match animations.get_mut(&object.id) {
            Some(animation) => animation,
            None => continue,
        };
        if !object.animation_playing || animation.frame_textures.is_empty() {
            continue;
        }
        animation.elapsed += delta_time;
        let frame_duration = 1.0 / animation.animation.frames_per_second.max(1.0);
        while animation.elapsed >= frame_duration {
            animation.elapsed -= frame_duration;
            if animation.current_frame + 1 < animation.frame_textures.len() {
                animation.current_frame += 1;
            } else if animation.animation.looping {
                animation.current_frame = 0;
            } else {
                animation.current_frame = animation.frame_textures.len() - 1;
                break;
            }
        }
    }
}

/// 读取一个动画JSON并将全部PNG序列帧上传到GPU。
fn load_runtime_animation(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    scene_directory: &std::path::Path,
    animation_path: &str,
) -> Option<RuntimeAnimation> {
    let path = resolve_runtime_asset_path(scene_directory, animation_path);
    let animation = match SpriteAnimation::load(&path) {
        Ok(animation) => animation,
        Err(error) => {
            eprintln!("Runtime加载动画失败：{error}");
            return None;
        }
    };
    let mut frame_textures = Vec::new();
    for frame in &animation.frames {
        let frame_path = resolve_asset_path(scene_directory, frame);
        let image = match image::open(&frame_path) {
            Ok(image) => image.to_rgba8(),
            Err(error) => {
                eprintln!(
                    "Runtime加载动画帧失败：{}，错误：{error}",
                    frame_path.display()
                );
                continue;
            }
        };
        frame_textures.push(Arc::new(create_runtime_texture(
            device,
            queue,
            layout,
            image.width(),
            image.height(),
            image.as_raw(),
        )));
    }
    Some(RuntimeAnimation {
        source_path: animation_path.to_owned(),
        animation,
        frame_textures,
        current_frame: 0,
        elapsed: 0.0,
    })
}

/// 将一个矩形物体转换为带完整UV坐标的两个三角形。
fn create_object_vertices(game_object: &GameObject, width: u32, height: u32) -> [Vertex; 6] {
    let left = pixel_to_clip_x(game_object.x, width);
    let right = pixel_to_clip_x(game_object.x + game_object.width, width);
    let top = pixel_to_clip_y(game_object.y, height);
    let bottom = pixel_to_clip_y(game_object.y + game_object.height, height);
    [
        Vertex {
            position: [left, top],
            texture_coordinate: [0.0, 0.0],
        },
        Vertex {
            position: [left, bottom],
            texture_coordinate: [0.0, 1.0],
        },
        Vertex {
            position: [right, bottom],
            texture_coordinate: [1.0, 1.0],
        },
        Vertex {
            position: [left, top],
            texture_coordinate: [0.0, 0.0],
        },
        Vertex {
            position: [right, bottom],
            texture_coordinate: [1.0, 1.0],
        },
        Vertex {
            position: [right, top],
            texture_coordinate: [1.0, 0.0],
        },
    ]
}

/// 为场景中的每个物体加载图片；没有图片或加载失败时使用蓝色回退纹理。
fn load_runtime_textures(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    game_objects: &[GameObject],
    scene_directory: &std::path::Path,
    use_cache: bool,
) -> HashMap<u64, Arc<RuntimeTexture>> {
    let mut textures = HashMap::new();
    let mut shared: HashMap<String, Arc<RuntimeTexture>> = HashMap::new();
    let fallback = Arc::new(create_runtime_texture(
        device,
        queue,
        layout,
        1,
        1,
        &[64, 140, 217, 255],
    ));
    for game_object in game_objects {
        if !game_object.audio_path.is_empty() {
            continue;
        }
        let image_path = resolve_runtime_asset_path(scene_directory, &game_object.image_path);
        let key = image_path.to_string_lossy().into_owned();
        let runtime_texture = if game_object.image_path.is_empty() {
            fallback.clone()
        } else if use_cache && shared.contains_key(&key) {
            shared.get(&key).expect("已确认纹理缓存键存在").clone()
        } else {
            let texture = match image::open(&image_path) {
                Ok(image) => {
                    let image = image.to_rgba8();
                    Arc::new(create_runtime_texture(
                        device,
                        queue,
                        layout,
                        image.width(),
                        image.height(),
                        image.as_raw(),
                    ))
                }
                Err(error) => {
                    eprintln!(
                        "runtime加载图片失败：{}，错误：{error}",
                        image_path.display()
                    );
                    fallback.clone()
                }
            };
            if use_cache {
                shared.insert(key, texture.clone());
            }
            texture
        };
        textures.insert(game_object.id, runtime_texture);
    }
    textures
}

/// 播放场景中的全部音效物体，并保持系统音频输出流存活。
fn play_scene_audio(
    game_objects: &[GameObject],
    scene_directory: &std::path::Path,
) -> Option<OutputStream> {
    let (stream, stream_handle) = match OutputStream::try_default() {
        Ok(value) => value,
        Err(error) => {
            eprintln!("无法打开默认音频设备：{error}");
            return None;
        }
    };

    for game_object in game_objects {
        if game_object.audio_path.is_empty() {
            continue;
        }
        let audio_path = resolve_runtime_asset_path(scene_directory, &game_object.audio_path);
        let file = match std::fs::File::open(&audio_path) {
            Ok(file) => file,
            Err(error) => {
                eprintln!("打开音效失败：{}，错误：{error}", audio_path.display());
                continue;
            }
        };
        let decoder = match Decoder::new(std::io::BufReader::new(file)) {
            Ok(decoder) => decoder,
            Err(error) => {
                eprintln!("解码音效失败：{}，错误：{error}", audio_path.display());
                continue;
            }
        };
        let sink = match Sink::try_new(&stream_handle) {
            Ok(sink) => sink,
            Err(error) => {
                eprintln!("创建音效播放器失败：{error}");
                continue;
            }
        };
        sink.append(decoder);
        sink.detach();
    }
    Some(stream)
}

/// 将场景中的相对图片路径解析为相对于scenes.json的路径。
fn resolve_runtime_asset_path(
    scene_directory: &std::path::Path,
    asset_path: &str,
) -> std::path::PathBuf {
    let path = std::path::Path::new(asset_path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        scene_directory.join(path)
    }
}

/// 将RGBA像素上传到GPU，并创建着色器需要的纹理和采样器绑定组。
fn create_runtime_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    width: u32,
    height: u32,
    rgba_pixels: &[u8],
) -> RuntimeTexture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Slide2D Runtime Texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba_pixels,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(4 * width),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Slide2D Runtime Sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Slide2D Runtime Texture Bind Group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&texture_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });
    RuntimeTexture { bind_group }
}

/// 将水平像素坐标转换为wgpu使用的-1到1裁剪空间坐标。
fn pixel_to_clip_x(pixel_x: f32, window_width: u32) -> f32 {
    pixel_x / window_width.max(1) as f32 * 2.0 - 1.0
}

/// 将向下增长的垂直像素坐标转换为wgpu裁剪空间坐标。
fn pixel_to_clip_y(pixel_y: f32, window_height: u32) -> f32 {
    1.0 - pixel_y / window_height.max(1) as f32 * 2.0
}
