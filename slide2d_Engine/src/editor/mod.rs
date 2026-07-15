use std::collections::HashMap;
use std::sync::Arc;
use std::{fs, process::Command};

use egui::{Color32, CursorIcon, PointerButton, Pos2, Rect, Stroke, Vec2};
use egui_wgpu::ScreenDescriptor;
use winit::dpi::LogicalSize;
use winit::event::{Event, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::{WindowBuilder, WindowLevel};

use crate::animation::{relative_to_project, resolve_asset_path, AnimationFrameDragPayload};
use crate::app_state::{
    AppState, BlueprintEditorMode, ColliderConfig, GameObject, ObjectInteraction,
    PendingEditorAction, PendingImageDrop, ResizeHandle,
};
use crate::assets::{
    delete_resource, duplicate_resource, rename_resource, scan_plugin_resource, ActorAsset,
    ActorAssetDragPayload, AnimationAssetDragPayload, AssetCategory, AssetEntry, AssetKind,
    AssetLibrary, AudioAssetDragPayload, ImageAssetDragPayload,
};
use crate::blueprint::editor::draw_blueprint_contents;
use crate::blueprint::model::Blueprint;
use crate::game_ui::{UiDragPayload, UiElementKind, UiTemplate};
use crate::localization::{
    current_language, localize_message, revision as language_revision, set_language, tr, tr_args,
    Language,
};
use crate::plugins::{delete_plugin, import_plugin_folder, PluginBehavior};
use crate::project::{
    ensure_project_extension, open_project_folder, remember_recent_project, save_project,
    save_project_folder,
};
use crate::tilemap::{TileLayerKind, TileSet, TileTool};

const MIN_OBJECT_SIZE: f32 = 20.0;
const RESIZE_HANDLE_SIZE: f32 = 10.0;
const MIN_VIEW_ZOOM: f32 = 0.25;
const MAX_VIEW_ZOOM: f32 = 4.0;

/// 缓存编辑器已经上传到GPU的图片纹理，避免每一帧重复读取文件。
struct EditorTextures {
    textures: HashMap<String, egui::TextureHandle>,
    /// 每个纹理最后一次请求时间，用于释放闲置缓存。
    last_used: HashMap<String, std::time::Instant>,
    /// 每个纹理RGBA解码后的估算内存字节数。
    estimated_bytes: HashMap<String, u64>,
    /// 上一次执行闲置缓存清理的时间。
    last_cleanup: std::time::Instant,
}

/// 区分辅助OS窗口显示的是设置还是蓝图。
#[derive(Clone, Copy)]
enum AuxiliaryWindowKind {
    Settings,
    Blueprint,
    Animation,
    Tilemap,
}

impl AuxiliaryWindowKind {
    /// 返回辅助窗口标题的本地化键。
    fn title_key(self) -> &'static str {
        match self {
            Self::Settings => "window.settings",
            Self::Blueprint => "window.blueprint",
            Self::Animation => "window.animation",
            Self::Tilemap => "window.tilemap",
        }
    }
}

/// 保存一个真正独立的winit窗口及其egui、wgpu渲染资源。
struct AuxiliaryWindow {
    kind: AuxiliaryWindowKind,
    window: Arc<winit::window::Window>,
    surface: wgpu::Surface<'static>,
    surface_config: wgpu::SurfaceConfiguration,
    egui_context: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    textures: HashMap<String, egui::TextureHandle>,
}

impl EditorTextures {
    /// 创建空的图片纹理缓存。
    fn new() -> Self {
        Self {
            textures: HashMap::new(),
            last_used: HashMap::new(),
            estimated_bytes: HashMap::new(),
            last_cleanup: std::time::Instant::now(),
        }
    }

    /// 返回当前纹理缓存估算内存。
    fn memory_bytes(&self) -> u64 {
        self.estimated_bytes.values().sum()
    }

    /// 每五秒检查一次，并释放超过三十秒没有使用的纹理。
    fn release_idle(&mut self, enabled: bool) {
        if !enabled || self.last_cleanup.elapsed().as_secs() < 5 {
            return;
        }
        self.last_cleanup = std::time::Instant::now();
        let now = std::time::Instant::now();
        let expired: Vec<String> = self
            .last_used
            .iter()
            .filter(|(_, used)| now.duration_since(**used).as_secs() >= 30)
            .map(|(key, _)| key.clone())
            .collect();
        for key in expired {
            self.textures.remove(&key);
            self.last_used.remove(&key);
            self.estimated_bytes.remove(&key);
        }
    }

    /// 关闭资源缓存时清空全部GPU纹理句柄，下一次使用会重新加载。
    fn apply_cache_switch(&mut self, enabled: bool) {
        if enabled {
            return;
        }
        self.textures.clear();
        self.last_used.clear();
        self.estimated_bytes.clear();
    }
}

/// 启动编辑器窗口，并持续处理窗口事件和绘制界面。
pub fn run(mut app_state: AppState) -> Result<(), String> {
    let event_loop = EventLoop::new().map_err(|error| error.to_string())?;
    let window = WindowBuilder::new()
        .with_title("Slide2D Engine")
        .with_inner_size(LogicalSize::new(1280.0, 720.0))
        .with_min_inner_size(LogicalSize::new(900.0, 560.0))
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
            label: Some("Slide2D GPU Device"),
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

    let egui_context = egui::Context::default();
    configure_chinese_fonts(&egui_context);
    let mut egui_state = egui_winit::State::new(
        egui_context.clone(),
        egui::ViewportId::ROOT,
        window.as_ref(),
        None,
        None,
    );
    let mut egui_renderer = egui_wgpu::Renderer::new(&device, surface_format, None, 1);
    let mut editor_textures = EditorTextures::new();
    let project_root = std::env::current_dir().map_err(|error| error.to_string())?;
    let mut asset_library = AssetLibrary::new(project_root)?;
    let mut applied_language_revision = language_revision();
    app_state.mark_project_saved();
    let mut last_cursor_position = Pos2::ZERO;
    let mut auxiliary_windows: Vec<AuxiliaryWindow> = Vec::new();

    event_loop
        .run(move |event, event_loop_window_target| {
            event_loop_window_target.set_control_flow(ControlFlow::Wait);

            match event {
                Event::WindowEvent { event, window_id } if window_id == window.id() => {
                    // DroppedFile不包含鼠标坐标，因此始终记录最近一次光标位置。
                    if let WindowEvent::CursorMoved { position, .. } = &event {
                        let scale_factor = window.scale_factor() as f32;
                        last_cursor_position = Pos2::new(
                            position.x as f32 / scale_factor,
                            position.y as f32 / scale_factor,
                        );
                    }
                    if let WindowEvent::DroppedFile(path) = &event {
                        eprintln!("收到拖放文件：{}", path.display());
                        match asset_library.import_file(path) {
                            Ok(imported_asset) => match imported_asset.kind {
                                AssetKind::Image => {
                                    app_state.pending_image_drops.push(PendingImageDrop {
                                        path: imported_asset.path,
                                        screen_x: last_cursor_position.x,
                                        screen_y: last_cursor_position.y,
                                    });
                                }
                                AssetKind::Audio => {
                                    app_state.status_message =
                                        format!("音效已导入：{}", imported_asset.path.display());
                                }
                                AssetKind::Animation => {
                                    app_state.status_message = format!(
                                        "动画资源已导入：{}",
                                        imported_asset.path.display()
                                    );
                                }
                                AssetKind::Tileset => {
                                    app_state.status_message =
                                        format!("瓦片集已导入：{}", imported_asset.path.display());
                                }
                            },
                            Err(error) => app_state.status_message = error,
                        }
                        asset_library.refresh();
                        app_state.is_file_hovering = false;
                        window.request_redraw();
                    }
                    if let WindowEvent::HoveredFile(path) = &event {
                        eprintln!("文件正在拖入窗口：{}", path.display());
                        app_state.is_file_hovering = true;
                        window.request_redraw();
                    }
                    if matches!(&event, WindowEvent::HoveredFileCancelled) {
                        app_state.is_file_hovering = false;
                        window.request_redraw();
                    }

                    let response = egui_state.on_window_event(window.as_ref(), &event);
                    if response.repaint {
                        window.request_redraw();
                    }
                    if response.consumed {
                        return;
                    }

                    match event {
                        WindowEvent::CloseRequested => {
                            if app_state.has_unsaved_changes() {
                                app_state.pending_editor_action =
                                    Some(PendingEditorAction::ExitEditor);
                            } else {
                                event_loop_window_target.exit();
                            }
                            window.request_redraw();
                        }
                        WindowEvent::Resized(new_size) => {
                            resize_surface(&surface, &device, &mut surface_config, new_size);
                            window.request_redraw();
                        }
                        WindowEvent::RedrawRequested => {
                            draw_frame(
                                window.as_ref(),
                                &surface,
                                &device,
                                &queue,
                                &surface_config,
                                &egui_context,
                                &mut egui_state,
                                &mut egui_renderer,
                                &mut app_state,
                                &mut editor_textures,
                                &mut asset_library,
                            );
                        }
                        _ => {}
                    }
                }
                Event::WindowEvent { event, window_id } => {
                    if let Some(index) = auxiliary_windows
                        .iter()
                        .position(|auxiliary| auxiliary.window.id() == window_id)
                    {
                        let should_close = handle_auxiliary_event(
                            &mut auxiliary_windows[index],
                            &event,
                            &device,
                            &queue,
                            &mut app_state,
                        );
                        if should_close {
                            let closed_kind = auxiliary_windows[index].kind;
                            auxiliary_windows.remove(index);
                            match closed_kind {
                                AuxiliaryWindowKind::Settings => {
                                    app_state.settings_window_open = false;
                                }
                                AuxiliaryWindowKind::Blueprint => {
                                    app_state.close_blueprint_owner();
                                }
                                AuxiliaryWindowKind::Animation => {
                                    app_state.animation_editor.window_open = false;
                                }
                                AuxiliaryWindowKind::Tilemap => {
                                    app_state.tile_editor.window_open = false;
                                    app_state.tile_editor.tool = TileTool::Select;
                                }
                            }
                        }
                    }
                }
                Event::AboutToWait => {
                    let current_revision = language_revision();
                    if current_revision != applied_language_revision {
                        applied_language_revision = current_revision;
                        window.set_title("Slide2D Engine");
                        for auxiliary in &auxiliary_windows {
                            auxiliary.window.set_title(&tr(auxiliary.kind.title_key()));
                            auxiliary.window.request_redraw();
                        }
                    }
                    if app_state.exit_requested {
                        event_loop_window_target.exit();
                        return;
                    }
                    let settings_exists = auxiliary_windows
                        .iter()
                        .any(|window| matches!(window.kind, AuxiliaryWindowKind::Settings));
                    if app_state.settings_window_open && !settings_exists {
                        match create_auxiliary_window(
                            event_loop_window_target,
                            &instance,
                            &adapter,
                            &device,
                            AuxiliaryWindowKind::Settings,
                        ) {
                            Ok(auxiliary) => auxiliary_windows.push(auxiliary),
                            Err(error) => {
                                app_state.status_message = error;
                                app_state.settings_window_open = false;
                            }
                        }
                    }

                    let blueprint_exists = auxiliary_windows
                        .iter()
                        .any(|window| matches!(window.kind, AuxiliaryWindowKind::Blueprint));
                    let blueprint_should_open = app_state.blueprint_owner_is_open()
                        && app_state.editor_settings.blueprint_editor_mode
                            == BlueprintEditorMode::SeparateWindow;
                    if !blueprint_should_open {
                        auxiliary_windows.retain(|window| {
                            !matches!(window.kind, AuxiliaryWindowKind::Blueprint)
                        });
                    }
                    if blueprint_should_open && !blueprint_exists {
                        match create_auxiliary_window(
                            event_loop_window_target,
                            &instance,
                            &adapter,
                            &device,
                            AuxiliaryWindowKind::Blueprint,
                        ) {
                            Ok(auxiliary) => auxiliary_windows.push(auxiliary),
                            Err(error) => app_state.status_message = error,
                        }
                    }

                    let animation_exists = auxiliary_windows
                        .iter()
                        .any(|window| matches!(window.kind, AuxiliaryWindowKind::Animation));
                    if app_state.animation_editor.window_open && !animation_exists {
                        match create_auxiliary_window(
                            event_loop_window_target,
                            &instance,
                            &adapter,
                            &device,
                            AuxiliaryWindowKind::Animation,
                        ) {
                            Ok(auxiliary) => auxiliary_windows.push(auxiliary),
                            Err(error) => app_state.status_message = error,
                        }
                    }
                    if !app_state.animation_editor.window_open {
                        auxiliary_windows.retain(|window| {
                            !matches!(window.kind, AuxiliaryWindowKind::Animation)
                        });
                    }
                    let tilemap_exists = auxiliary_windows
                        .iter()
                        .any(|window| matches!(window.kind, AuxiliaryWindowKind::Tilemap));
                    if app_state.tile_editor.window_open && !tilemap_exists {
                        match create_auxiliary_window(
                            event_loop_window_target,
                            &instance,
                            &adapter,
                            &device,
                            AuxiliaryWindowKind::Tilemap,
                        ) {
                            Ok(auxiliary) => auxiliary_windows.push(auxiliary),
                            Err(error) => app_state.status_message = error,
                        }
                    }
                    if !app_state.tile_editor.window_open {
                        auxiliary_windows
                            .retain(|window| !matches!(window.kind, AuxiliaryWindowKind::Tilemap));
                    }

                    window.request_redraw();
                    for auxiliary in &auxiliary_windows {
                        auxiliary.window.request_redraw();
                    }
                }
                _ => {}
            }
        })
        .map_err(|error| error.to_string())
}

/// 创建设置或蓝图使用的真正独立Windows原生窗口。
fn create_auxiliary_window(
    event_loop: &winit::event_loop::EventLoopWindowTarget<()>,
    instance: &wgpu::Instance,
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    kind: AuxiliaryWindowKind,
) -> Result<AuxiliaryWindow, String> {
    let (width, height) = match kind {
        AuxiliaryWindowKind::Settings => (520.0, 620.0),
        AuxiliaryWindowKind::Blueprint => (1200.0, 760.0),
        AuxiliaryWindowKind::Animation => (1100.0, 720.0),
        AuxiliaryWindowKind::Tilemap => (950.0, 720.0),
    };
    let title = tr(kind.title_key());
    let mut window_builder = WindowBuilder::new()
        .with_title(title)
        .with_inner_size(LogicalSize::new(width, height));
    if matches!(kind, AuxiliaryWindowKind::Tilemap) {
        // 瓦片工具窗口始终位于其他编辑器窗口上方，绘图时不会被设置或蓝图窗口遮挡。
        window_builder = window_builder.with_window_level(WindowLevel::AlwaysOnTop);
    }
    let window = window_builder
        .build(event_loop)
        .map_err(|error| format!("创建独立窗口失败：{error}"))?;
    let window = Arc::new(window);
    let surface = instance
        .create_surface(window.clone())
        .map_err(|error| format!("创建独立窗口GPU表面失败：{error}"))?;
    let capabilities = surface.get_capabilities(adapter);
    let surface_format = capabilities.formats[0];
    let size = window.inner_size();
    let surface_config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: surface_format,
        width: size.width.max(1),
        height: size.height.max(1),
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: capabilities.alpha_modes[0],
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    surface.configure(device, &surface_config);

    let egui_context = egui::Context::default();
    configure_chinese_fonts(&egui_context);
    let egui_state = egui_winit::State::new(
        egui_context.clone(),
        egui::ViewportId::ROOT,
        window.as_ref(),
        None,
        None,
    );
    let egui_renderer = egui_wgpu::Renderer::new(device, surface_format, None, 1);
    Ok(AuxiliaryWindow {
        kind,
        window,
        surface,
        surface_config,
        egui_context,
        egui_state,
        egui_renderer,
        textures: HashMap::new(),
    })
}

/// 处理独立设置窗口或蓝图窗口收到的winit事件。
fn handle_auxiliary_event(
    auxiliary: &mut AuxiliaryWindow,
    event: &WindowEvent,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    app_state: &mut AppState,
) -> bool {
    let response = auxiliary
        .egui_state
        .on_window_event(auxiliary.window.as_ref(), event);
    if response.repaint {
        auxiliary.window.request_redraw();
    }

    match event {
        WindowEvent::CloseRequested => return true,
        WindowEvent::Resized(new_size) => {
            resize_surface(
                &auxiliary.surface,
                device,
                &mut auxiliary.surface_config,
                *new_size,
            );
        }
        WindowEvent::RedrawRequested => {
            draw_auxiliary_frame(auxiliary, device, queue, app_state);
        }
        _ => {}
    }
    false
}

/// 绘制一帧独立设置窗口或蓝图窗口。
fn draw_auxiliary_frame(
    auxiliary: &mut AuxiliaryWindow,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    app_state: &mut AppState,
) {
    let surface_texture = match auxiliary.surface.get_current_texture() {
        Ok(texture) => texture,
        Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
            auxiliary
                .surface
                .configure(device, &auxiliary.surface_config);
            return;
        }
        Err(_) => return,
    };
    let raw_input = auxiliary
        .egui_state
        .take_egui_input(auxiliary.window.as_ref());
    apply_editor_theme(
        &auxiliary.egui_context,
        app_state.editor_settings.dark_theme,
    );
    let output = auxiliary.egui_context.run(raw_input, |context| {
        egui::CentralPanel::default()
            .frame(
                if matches!(
                    auxiliary.kind,
                    AuxiliaryWindowKind::Blueprint
                        | AuxiliaryWindowKind::Animation
                        | AuxiliaryWindowKind::Tilemap
                ) {
                    egui::Frame::none().inner_margin(0.0)
                } else {
                    egui::Frame::central_panel(&context.style())
                },
            )
            .show(context, |ui| match auxiliary.kind {
                AuxiliaryWindowKind::Settings => draw_settings_contents(ui, app_state),
                AuxiliaryWindowKind::Blueprint => draw_blueprint_contents(ui, app_state),
                AuxiliaryWindowKind::Animation => {
                    draw_animation_editor(ui, app_state, &mut auxiliary.textures)
                }
                AuxiliaryWindowKind::Tilemap => {
                    draw_tilemap_editor(ui, app_state, &mut auxiliary.textures)
                }
            });
    });
    auxiliary
        .egui_state
        .handle_platform_output(auxiliary.window.as_ref(), output.platform_output);
    let paint_jobs = auxiliary
        .egui_context
        .tessellate(output.shapes, output.pixels_per_point);
    let descriptor = ScreenDescriptor {
        size_in_pixels: [
            auxiliary.surface_config.width,
            auxiliary.surface_config.height,
        ],
        pixels_per_point: auxiliary.window.scale_factor() as f32,
    };
    for (texture_id, image_delta) in &output.textures_delta.set {
        auxiliary
            .egui_renderer
            .update_texture(device, queue, *texture_id, image_delta);
    }
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Slide2D Auxiliary Window Encoder"),
    });
    auxiliary
        .egui_renderer
        .update_buffers(device, queue, &mut encoder, &paint_jobs, &descriptor);
    let view = surface_texture
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Slide2D Auxiliary Window Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        auxiliary
            .egui_renderer
            .render(&mut pass, &paint_jobs, &descriptor);
    }
    queue.submit(Some(encoder.finish()));
    surface_texture.present();
    for texture_id in &output.textures_delta.free {
        auxiliary.egui_renderer.free_texture(texture_id);
    }
}

/// 从Windows字体目录加载中文字体，确保面板中的中文不会显示成方框。
fn configure_chinese_fonts(egui_context: &egui::Context) {
    let font_paths = [
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\simsun.ttc",
    ];

    for font_path in font_paths {
        let font_data = match std::fs::read(font_path) {
            Ok(data) => data,
            Err(_) => continue,
        };

        let mut font_definitions = egui::FontDefinitions::default();
        font_definitions.font_data.insert(
            "chinese_font".to_owned(),
            egui::FontData::from_owned(font_data),
        );
        font_definitions
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "chinese_font".to_owned());
        egui_context.set_fonts(font_definitions);
        return;
    }

    eprintln!("未找到Windows中文字体，界面中的中文可能无法正确显示");
}

/// 在窗口尺寸改变后，使用新尺寸重新配置GPU表面。
fn resize_surface(
    surface: &wgpu::Surface,
    device: &wgpu::Device,
    surface_config: &mut wgpu::SurfaceConfiguration,
    new_size: winit::dpi::PhysicalSize<u32>,
) {
    if new_size.width == 0 || new_size.height == 0 {
        return;
    }

    surface_config.width = new_size.width;
    surface_config.height = new_size.height;
    surface.configure(device, surface_config);
}

/// 绘制一帧编辑器画面，并将egui生成的图形提交给GPU显示。
#[allow(clippy::too_many_arguments)]
fn draw_frame(
    window: &winit::window::Window,
    surface: &wgpu::Surface,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    surface_config: &wgpu::SurfaceConfiguration,
    egui_context: &egui::Context,
    egui_state: &mut egui_winit::State,
    egui_renderer: &mut egui_wgpu::Renderer,
    app_state: &mut AppState,
    editor_textures: &mut EditorTextures,
    asset_library: &mut AssetLibrary,
) {
    let surface_texture = match surface.get_current_texture() {
        Ok(texture) => texture,
        Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
            surface.configure(device, surface_config);
            return;
        }
        Err(wgpu::SurfaceError::OutOfMemory) => {
            eprintln!("GPU显存不足，无法继续绘制");
            return;
        }
        Err(wgpu::SurfaceError::Timeout) => return,
    };

    let raw_input = egui_state.take_egui_input(window);
    let full_output = egui_context.run(raw_input, |context| {
        draw_editor_ui(context, app_state, editor_textures, asset_library);
    });
    egui_state.handle_platform_output(window, full_output.platform_output);

    let paint_jobs = egui_context.tessellate(full_output.shapes, full_output.pixels_per_point);
    let screen_descriptor = ScreenDescriptor {
        size_in_pixels: [surface_config.width, surface_config.height],
        pixels_per_point: window.scale_factor() as f32,
    };
    for (texture_id, image_delta) in &full_output.textures_delta.set {
        egui_renderer.update_texture(device, queue, *texture_id, image_delta);
    }

    let mut command_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Slide2D Render Encoder"),
    });
    egui_renderer.update_buffers(
        device,
        queue,
        &mut command_encoder,
        &paint_jobs,
        &screen_descriptor,
    );

    let texture_view = surface_texture
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    {
        let mut render_pass = command_encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Slide2D Egui Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &texture_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.08,
                        g: 0.08,
                        b: 0.09,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        egui_renderer.render(&mut render_pass, &paint_jobs, &screen_descriptor);
    }

    queue.submit(Some(command_encoder.finish()));
    surface_texture.present();
    for texture_id in &full_output.textures_delta.free {
        egui_renderer.free_texture(texture_id);
    }
}

/// 使用egui划分资源面板、场景画布和属性面板三个区域。
fn draw_editor_ui(
    context: &egui::Context,
    app_state: &mut AppState,
    editor_textures: &mut EditorTextures,
    asset_library: &mut AssetLibrary,
) {
    app_state.performance_metrics.update_editor_frame_rate();
    editor_textures.apply_cache_switch(app_state.performance_settings.resource_cache);
    editor_textures.release_idle(app_state.performance_settings.idle_cache_release);
    app_state.performance_metrics.cached_assets = editor_textures.textures.len();
    app_state.performance_metrics.memory_bytes = editor_textures.memory_bytes();
    apply_editor_theme(context, app_state.editor_settings.dark_theme);
    draw_project_menu(context, app_state, asset_library);
    draw_plugin_toolbar(context, app_state);
    draw_performance_status_bar(context, app_state);
    // IDE模式打开蓝图标签后，隐藏只服务于场景编辑的左右面板。
    // 这样节点画布可以使用完整窗口宽度，返回“场景”标签后面板会自动恢复。
    let blueprint_uses_full_workspace = app_state.editor_settings.blueprint_editor_mode
        == BlueprintEditorMode::IdeTabs
        && app_state.blueprint_tab_active
        && app_state.blueprint_owner_is_open();

    if !blueprint_uses_full_workspace {
        egui::SidePanel::left("resource_panel")
            .exact_width(220.0)
            .resizable(false)
            .show(context, |ui| {
                ui.heading(tr("panel.resources"));
                ui.separator();
                // 固定操作按钮必须放在资源树滚动区域之前，否则资源条目较多时
                // ScrollArea会占满剩余高度，把设置和运行按钮挤到面板外面。
                ui.horizontal_wrapped(|ui| {
                    if ui.button(tr("toolbar.add_block")).clicked() {
                        app_state.add_test_object();
                    }
                    if ui.button(tr("toolbar.export_scene")).clicked() {
                        match export_scene(app_state) {
                            Ok(scene_path) => {
                                app_state.status_message =
                                    format!("场景已导出：{}", scene_path.display());
                            }
                            Err(error) => app_state.status_message = error,
                        }
                    }
                    if ui.button(tr("toolbar.run_game")).clicked() {
                        match launch_runtime(app_state) {
                            Ok(()) => app_state.status_message = "游戏运行时已启动".to_owned(),
                            Err(error) => app_state.status_message = error,
                        }
                    }
                    if ui.button(tr("toolbar.settings")).clicked() {
                        app_state.settings_window_open = true;
                    }
                });
                if !app_state.status_message.is_empty() {
                    ui.label(
                        egui::RichText::new(localize_message(&app_state.status_message))
                            .small()
                            .color(ui.visuals().weak_text_color()),
                    );
                }
                ui.separator();
                draw_asset_library(ui, app_state, editor_textures, asset_library);
            });

        egui::SidePanel::right("property_panel")
            .exact_width(260.0)
            .resizable(false)
            .show(context, |ui| {
                ui.heading(tr("panel.ui_library"));
                draw_ui_component_library(ui, app_state);
                ui.separator();
                ui.heading(tr("panel.properties"));
                ui.separator();
                draw_property_panel(ui, app_state);
            });
    }

    let central_frame = if blueprint_uses_full_workspace {
        // 蓝图模式移除CentralPanel默认内边距，让网格延伸到窗口四周。
        egui::Frame::none().inner_margin(0.0)
    } else {
        egui::Frame::central_panel(&context.style())
    };
    egui::CentralPanel::default()
        .frame(central_frame)
        .show(context, |ui| {
            if app_state.editor_settings.blueprint_editor_mode == BlueprintEditorMode::IdeTabs {
                draw_workspace_tabs(ui, app_state, editor_textures);
            } else {
                ui.heading(tr("panel.game_scene"));
                ui.add_space(6.0);
                let canvas_rect = ui.available_rect_before_wrap();
                draw_canvas(ui, canvas_rect, app_state, editor_textures);
            }
        });

    // 设置窗口和独立蓝图窗口由run函数中的真正OS窗口管理，这里不再绘制egui子窗口。
}

/// 在全局菜单下方绘制插件声明的工具栏按钮，禁用插件后按钮立即消失。
fn draw_plugin_toolbar(context: &egui::Context, app_state: &mut AppState) {
    let tools: Vec<(String, String, String)> = app_state
        .plugin_registry
        .installed
        .iter()
        .filter(|plugin| plugin.enabled && plugin.load_error.is_none())
        .flat_map(|plugin| {
            plugin
                .manifest
                .editor_tools
                .iter()
                .map(|tool| {
                    (
                        plugin.manifest.plugin_id.clone(),
                        tool.tool_id.clone(),
                        tool.display_name.clone(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect();
    if tools.is_empty() {
        return;
    }
    egui::TopBottomPanel::top("slide2d_plugin_toolbar").show(context, |ui| {
        ui.horizontal(|ui| {
            ui.small("Slide2D Plugin System");
            for (plugin_id, tool_id, label) in tools {
                if ui.button(label).clicked() {
                    app_state.open_plugin_tool = Some((plugin_id, tool_id));
                }
            }
        });
    });
}

/// 绘制工程文件和多场景管理菜单。
fn draw_project_menu(
    context: &egui::Context,
    app_state: &mut AppState,
    asset_library: &mut AssetLibrary,
) {
    if context.input(|input| input.modifiers.ctrl && input.key_pressed(egui::Key::S)) {
        save_current_project(app_state, asset_library);
    }
    egui::TopBottomPanel::top("project_menu_bar").show(context, |ui| {
        egui::menu::bar(ui, |ui| {
            ui.menu_button(tr("menu.file"), |ui| {
                if ui.button(tr("file.new")).clicked() {
                    if app_state.has_unsaved_changes() {
                        app_state.pending_editor_action = Some(PendingEditorAction::NewProject);
                    } else if let Err(error) = create_new_project(app_state, asset_library) {
                        app_state.status_message = error;
                    }
                    ui.close_menu();
                }
                if ui.button(tr("file.open")).clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title(tr("dialog.open_project"))
                        .pick_folder()
                    {
                        open_project_from_folder(&path, app_state, asset_library);
                    }
                    ui.close_menu();
                }
                if ui.button(tr("file.save")).clicked() {
                    save_current_project(app_state, asset_library);
                    ui.close_menu();
                }
                if ui.button(tr("file.save_as")).clicked() {
                    app_state.status_message = match export_project_package(app_state) {
                        Ok(path) => format!("Slide2D工程已另存为：{}", path.display()),
                        Err(error) => error,
                    };
                    ui.close_menu();
                }
                ui.separator();
                if ui.button(tr("file.export_scene")).clicked() {
                    app_state.status_message = match export_scene(app_state) {
                        Ok(path) => format!("Slide2D场景已导出：{}", path.display()),
                        Err(error) => error,
                    };
                    ui.close_menu();
                }
                if ui.button(tr("file.export_assets")).clicked() {
                    app_state.status_message = match export_all_assets(app_state) {
                        Ok(path) => format!("Slide2D全部素材已导出：{}", path.display()),
                        Err(error) => error,
                    };
                    ui.close_menu();
                }
                ui.separator();
                ui.menu_button(tr("file.recent"), |ui| {
                    if app_state.recent_projects.is_empty() {
                        ui.label(tr("file.no_recent"));
                    }
                    let recent = app_state.recent_projects.clone();
                    for path in recent {
                        if ui.button(path.display().to_string()).clicked() {
                            open_project_from_folder(&path, app_state, asset_library);
                            ui.close_menu();
                        }
                    }
                });
                ui.separator();
                if ui.button(tr("file.exit")).clicked() {
                    if app_state.has_unsaved_changes() {
                        app_state.pending_editor_action = Some(PendingEditorAction::ExitEditor);
                    } else {
                        app_state.exit_requested = true;
                    }
                    ui.close_menu();
                }
            });
            ui.menu_button(tr("menu.edit"), |ui| {
                ui.label(tr("edit.visual_only"));
                ui.label(tr("edit.no_script"));
            });
            ui.menu_button(tr("menu.view"), |ui| {
                ui.checkbox(
                    &mut app_state.editor_settings.show_grid,
                    tr("view.show_grid"),
                );
                if ui.button(tr("view.performance")).clicked() {
                    app_state.performance_monitor_open = true;
                    ui.close_menu();
                }
                if ui.button(tr("view.reset")).clicked() {
                    app_state.view_offset_x = 0.0;
                    app_state.view_offset_y = 0.0;
                    app_state.view_zoom = 1.0;
                    ui.close_menu();
                }
            });
            ui.menu_button(tr("menu.tools"), |ui| {
                if ui.button(tr("tools.settings")).clicked() {
                    app_state.settings_window_open = true;
                    ui.close_menu();
                }
                if ui.button(tr("tools.animation")).clicked() {
                    app_state.animation_editor.window_open = true;
                    ui.close_menu();
                }
                if ui.button(tr("tools.tilemap")).clicked() {
                    app_state.tile_editor.window_open = true;
                    ui.close_menu();
                }
                if ui.button(tr("tools.plugins")).clicked() {
                    app_state.plugin_manager_open = true;
                    ui.close_menu();
                }
                if ui.button(tr("language.settings")).clicked() {
                    app_state.language_settings_open = true;
                    ui.close_menu();
                }
                if ui.button(tr("tools.assistant")).clicked() {
                    app_state.assistant_toolkit_open = true;
                    ui.close_menu();
                }
                let plugin_tools: Vec<(String, String, String)> = app_state
                    .plugin_registry
                    .installed
                    .iter()
                    .filter(|plugin| plugin.enabled && plugin.load_error.is_none())
                    .flat_map(|plugin| {
                        plugin
                            .manifest
                            .editor_tools
                            .iter()
                            .map(|tool| {
                                (
                                    plugin.manifest.plugin_id.clone(),
                                    tool.tool_id.clone(),
                                    tool.display_name.clone(),
                                )
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect();
                if !plugin_tools.is_empty() {
                    ui.menu_button(tr("tools.plugin_tools"), |ui| {
                        for (plugin_id, tool_id, display_name) in &plugin_tools {
                            if ui.button(display_name).clicked() {
                                app_state.open_plugin_tool =
                                    Some((plugin_id.clone(), tool_id.clone()));
                                ui.close_menu();
                            }
                        }
                    });
                }
                ui.separator();
                if ui.button(tr("tools.new_scene")).clicked() {
                    app_state.add_scene(String::new());
                    ui.close_menu();
                }
                ui.separator();
                let scene_names: Vec<String> = app_state
                    .project_scenes
                    .iter()
                    .map(|scene| scene.name.clone())
                    .collect();
                for (index, name) in scene_names.iter().enumerate() {
                    if ui
                        .selectable_label(
                            index == app_state.active_scene_index,
                            tr_args("status.switch_scene", &[("value", name.clone())]),
                        )
                        .clicked()
                    {
                        app_state.switch_scene(index);
                        ui.close_menu();
                    }
                }
                ui.separator();
                ui.label(tr("tools.startup_scene"));
                for name in scene_names {
                    ui.radio_value(&mut app_state.startup_scene_name, name.clone(), name);
                }
            });
            ui.menu_button(tr("menu.help"), |ui| {
                ui.strong(tr("help.title"));
                ui.label(tr("help.blueprint_only"));
                ui.label(tr("help.watermark"));
            });
            ui.separator();
            let project_name = app_state
                .project_file_path
                .as_ref()
                .and_then(|path| path.file_name())
                .and_then(|value| value.to_str())
                .map(str::to_owned)
                .unwrap_or_else(|| tr("common.unsaved"));
            ui.label(tr_args("status.project", &[("value", project_name)]));
            ui.label(tr_args(
                "status.scene",
                &[("value", app_state.active_scene_name().to_owned())],
            ));
        });
    });
    draw_unsaved_changes_dialog(context, app_state, asset_library);
    draw_save_notice_dialog(context, app_state);
    draw_plugin_manager(context, app_state, asset_library);
    draw_plugin_tool_window(context, app_state);
    draw_performance_monitor(context, app_state);
    draw_language_settings(context, app_state);
    draw_assistant_toolkit(context, app_state, asset_library);
}

/// 绘制Slide2D辅助开发工具总面板，全部功能通过可视化按钮和勾选项操作。
fn draw_assistant_toolkit(
    context: &egui::Context,
    app_state: &mut AppState,
    asset_library: &mut AssetLibrary,
) {
    if !app_state.assistant_toolkit_open { return; }
    let mut open = true;
    egui::Window::new(tr("window.assistant"))
        .open(&mut open)
        .default_size(Vec2::new(640.0, 720.0))
        .show(context, |ui| {
            ui.heading(tr("assistant.grid_ruler"));
            if ui.add(egui::Slider::new(&mut app_state.assistant_settings.grid_size, 4.0..=256.0).text(tr("assistant.grid_size"))).changed() {
                app_state.grid_size = app_state.assistant_settings.grid_size;
            }
            ui.checkbox(&mut app_state.assistant_settings.snap_to_grid, tr("assistant.snap"));
            ui.checkbox(&mut app_state.assistant_settings.show_rulers, tr("assistant.rulers"));
            ui.separator();

            ui.heading(tr("assistant.selection"));
            ui.label(tr("assistant.selection_help"));
            ui.label(tr_args("assistant.selected_count", &[("value", app_state.selected_object_ids.len().to_string())]));
            ui.horizontal_wrapped(|ui| {
                if ui.button(tr("assistant.align_left")).clicked() { align_selected_objects(app_state, AlignOperation::Left); }
                if ui.button(tr("assistant.align_center")).clicked() { align_selected_objects(app_state, AlignOperation::Center); }
                if ui.button(tr("assistant.align_top")).clicked() { align_selected_objects(app_state, AlignOperation::Top); }
                if ui.button(tr("assistant.distribute")).clicked() { distribute_selected_objects(app_state); }
                if ui.button(tr("assistant.duplicate_group")).clicked() { duplicate_selected_objects(app_state); }
                if ui.button(tr("assistant.center_canvas")).clicked() { center_selected_objects_in_canvas(app_state); }
                if ui.button(tr("assistant.clear_scene")).clicked() {
                    app_state.clear_scene_objects();
                    app_state.status_message = tr("assistant.cleared");
                }
            });
            ui.separator();

            ui.heading(tr("assistant.colliders"));
            ui.checkbox(&mut app_state.assistant_settings.show_object_colliders, tr("assistant.object_colliders"));
            ui.checkbox(&mut app_state.assistant_settings.show_tile_colliders, tr("assistant.tile_colliders"));
            ui.checkbox(&mut app_state.assistant_settings.show_static_colliders, tr("assistant.static_colliders"));
            ui.checkbox(&mut app_state.assistant_settings.show_dynamic_colliders, tr("assistant.dynamic_colliders"));
            ui.separator();

            ui.heading(tr("assistant.assets"));
            if ui.button(tr("assistant.select_assets")).clicked() {
                app_state.assistant_selected_assets = collect_batch_assets(asset_library);
            }
            ui.label(tr_args("assistant.asset_count", &[("value", app_state.assistant_selected_assets.len().to_string())]));
            ui.label(tr("assistant.batch_name"));
            ui.text_edit_singleline(&mut app_state.assistant_batch_name);
            if ui.button(tr("assistant.batch_rename")).clicked() {
                app_state.status_message = batch_rename_assets(app_state, asset_library).unwrap_or_else(|error| error);
            }
            ui.add(egui::Slider::new(&mut app_state.assistant_texture_max_size, 128..=4096).text(tr("assistant.texture_size")));
            if ui.button(tr("assistant.compress")).clicked() {
                app_state.status_message = batch_resize_textures(app_state).unwrap_or_else(|error| error);
            }
            if ui.button(tr("assistant.export_frames")).clicked() {
                app_state.status_message = export_selected_animation_frames(app_state).unwrap_or_else(|error| error);
            }
            ui.separator();

            ui.heading(tr("assistant.bookmarks"));
            if ui.button(tr("assistant.save_bookmark")).clicked() { save_camera_bookmark(app_state); }
            let bookmarks = app_state.assistant_settings.camera_bookmarks.clone();
            for (index, bookmark) in bookmarks.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.button(&bookmark.name).clicked() {
                        app_state.view_offset_x = bookmark.offset_x;
                        app_state.view_offset_y = bookmark.offset_y;
                        app_state.view_zoom = bookmark.zoom;
                    }
                    if ui.small_button(tr("common.delete")).clicked() {
                        app_state.assistant_settings.camera_bookmarks.remove(index);
                    }
                });
            }
            ui.separator();

            ui.heading(tr("assistant.screenshot"));
            ui.checkbox(&mut app_state.assistant_settings.transparent_screenshot, tr("assistant.transparent"));
            if ui.button(tr("assistant.capture")).clicked() {
                app_state.status_message = export_canvas_screenshot(app_state).unwrap_or_else(|error| error);
            }
            ui.separator();
            ui.small("Slide2D Assistant Toolkit");
            ui.small(tr("localization.no_code"));
        });
    app_state.assistant_toolkit_open = open;
}

/// 对齐命令类型。
#[derive(Clone, Copy)]
enum AlignOperation { Left, Center, Top }

/// 将多选对象按左边、中心或顶部统一对齐。
fn align_selected_objects(app_state: &mut AppState, operation: AlignOperation) {
    let selected: Vec<&GameObject> = app_state.game_objects.iter()
        .filter(|object| app_state.selected_object_ids.contains(&object.id)).collect();
    if selected.len() < 2 { app_state.status_message = tr("assistant.no_selection"); return; }
    let target = match operation {
        AlignOperation::Left => selected.iter().map(|object| object.x).fold(f32::INFINITY, f32::min),
        AlignOperation::Center => selected.iter().map(|object| object.x + object.width * 0.5).sum::<f32>() / selected.len() as f32,
        AlignOperation::Top => selected.iter().map(|object| object.y).fold(f32::INFINITY, f32::min),
    };
    for object in app_state.game_objects.iter_mut().filter(|object| app_state.selected_object_ids.contains(&object.id)) {
        match operation {
            AlignOperation::Left => object.x = target,
            AlignOperation::Center => object.x = target - object.width * 0.5,
            AlignOperation::Top => object.y = target,
        }
    }
}

/// 按对象中心从左到右排序，并让中间对象保持相同水平中心间距。
fn distribute_selected_objects(app_state: &mut AppState) {
    let mut ids: Vec<u64> = app_state.selected_object_ids.clone();
    ids.sort_by(|left, right| {
        let left_x = app_state.game_objects.iter().find(|object| object.id == *left).map(|object| object.x + object.width * 0.5).unwrap_or(0.0);
        let right_x = app_state.game_objects.iter().find(|object| object.id == *right).map(|object| object.x + object.width * 0.5).unwrap_or(0.0);
        left_x.total_cmp(&right_x)
    });
    if ids.len() < 3 { app_state.status_message = tr("assistant.no_selection"); return; }
    let first = app_state.game_objects.iter().find(|object| object.id == ids[0]).map(|object| object.x + object.width * 0.5).unwrap_or(0.0);
    let last = app_state.game_objects.iter().find(|object| object.id == *ids.last().unwrap()).map(|object| object.x + object.width * 0.5).unwrap_or(first);
    let spacing = (last - first) / (ids.len() - 1) as f32;
    for (index, id) in ids.iter().enumerate().skip(1).take(ids.len() - 2) {
        if let Some(object) = app_state.game_objects.iter_mut().find(|object| object.id == *id) {
            object.x = first + spacing * index as f32 - object.width * 0.5;
        }
    }
}

/// 复制全部选中对象，分配新ID、蓝图文件名和轻微偏移。
fn duplicate_selected_objects(app_state: &mut AppState) {
    let sources: Vec<GameObject> = app_state.game_objects.iter()
        .filter(|object| app_state.selected_object_ids.contains(&object.id)).cloned().collect();
    let mut new_ids = Vec::new();
    for mut object in sources {
        object.id = app_state.next_object_id;
        object.layer_index = app_state.next_layer_index;
        object.x += app_state.assistant_settings.grid_size;
        object.y += app_state.assistant_settings.grid_size;
        object.blueprint_file = format!("blueprint_{}.json", object.id);
        new_ids.push(object.id);
        app_state.next_object_id += 1;
        app_state.next_layer_index += 1;
        app_state.game_objects.push(object);
    }
    app_state.selected_object_ids = new_ids;
    app_state.selected_object_id = app_state.selected_object_ids.last().copied();
}

/// 调整场景相机，使所选对象包围盒中心落在最近画布中心。
fn center_selected_objects_in_canvas(app_state: &mut AppState) {
    let selected: Vec<&GameObject> = app_state.game_objects.iter()
        .filter(|object| app_state.selected_object_ids.contains(&object.id)).collect();
    if selected.is_empty() { return; }
    let left = selected.iter().map(|object| object.x).fold(f32::INFINITY, f32::min);
    let right = selected.iter().map(|object| object.x + object.width).fold(f32::NEG_INFINITY, f32::max);
    let top = selected.iter().map(|object| object.y).fold(f32::INFINITY, f32::min);
    let bottom = selected.iter().map(|object| object.y + object.height).fold(f32::NEG_INFINITY, f32::max);
    app_state.view_offset_x = app_state.last_canvas_width as f32 * 0.5 - (left + right) * 0.5 * app_state.view_zoom;
    app_state.view_offset_y = app_state.last_canvas_height as f32 * 0.5 - (top + bottom) * 0.5 * app_state.view_zoom;
}

/// 保存当前画布平移和缩放为工程级摄像机书签。
fn save_camera_bookmark(app_state: &mut AppState) {
    let index = app_state.assistant_settings.camera_bookmarks.len() + 1;
    app_state.assistant_settings.camera_bookmarks.push(crate::app_state::CameraBookmark {
        name: format!("Camera {index}"),
        offset_x: app_state.view_offset_x,
        offset_y: app_state.view_offset_y,
        zoom: app_state.view_zoom,
    });
}

/// 收集资源库中全部PNG和动画文件，供批处理窗口选择。
fn collect_batch_assets(asset_library: &AssetLibrary) -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    collect_asset_entry_paths(asset_library.scan_images(), &mut paths);
    collect_asset_entry_paths(asset_library.scan_animations(), &mut paths);
    paths.sort();
    paths.dedup();
    paths
}

/// 递归收集资源树文件路径。
fn collect_asset_entry_paths(entries: &[AssetEntry], paths: &mut Vec<std::path::PathBuf>) {
    for entry in entries {
        match entry {
            AssetEntry::Folder { children, .. } => collect_asset_entry_paths(children, paths),
            AssetEntry::File { path, .. } => paths.push(path.clone()),
        }
    }
}

/// 将所选素材按路径排序后使用基础名称和三位序号重命名。
fn batch_rename_assets(app_state: &mut AppState, asset_library: &mut AssetLibrary) -> Result<String, String> {
    let base = app_state.assistant_batch_name.trim();
    if base.is_empty() { return Err("Slide2D Assistant Toolkit: empty batch name".to_owned()); }
    let mut selected = app_state.assistant_selected_assets.clone();
    selected.sort();
    let mut renamed = Vec::new();
    for (index, path) in selected.iter().enumerate() {
        if !path.is_file() { continue; }
        let extension = path.extension().and_then(|value| value.to_str()).unwrap_or("");
        let new_name = format!("{}_{:03}.{}", base, index + 1, extension);
        renamed.push(crate::assets::rename_resource(path, &new_name)?);
    }
    app_state.assistant_selected_assets = renamed;
    asset_library.refresh();
    Ok(format!("Slide2D Assistant Toolkit: renamed {} assets", app_state.assistant_selected_assets.len()))
}

/// 批量缩小所选PNG并使用PNG编码保存，透明通道保持不变。
fn batch_resize_textures(app_state: &AppState) -> Result<String, String> {
    use image::GenericImageView;
    let mut count = 0;
    for path in &app_state.assistant_selected_assets {
        if path.extension().and_then(|value| value.to_str()).map(|value| value.eq_ignore_ascii_case("png")) != Some(true) { continue; }
        let image = image::open(path).map_err(|error| format!("Slide2D Assistant Toolkit: {error}"))?;
        let (width, height) = image.dimensions();
        let maximum = app_state.assistant_texture_max_size.max(1);
        let output = if width > maximum || height > maximum {
            let scale = maximum as f32 / width.max(height) as f32;
            image.resize((width as f32 * scale) as u32, (height as f32 * scale) as u32, image::imageops::FilterType::Lanczos3)
        } else { image };
        output.save_with_format(path, image::ImageFormat::Png).map_err(|error| format!("Slide2D Assistant Toolkit: {error}"))?;
        count += 1;
    }
    Ok(format!("Slide2D Assistant Toolkit: processed {count} textures"))
}

/// 将所选.s2anim全部序列帧复制到工程AssistantExports目录并生成标识清单。
fn export_selected_animation_frames(app_state: &AppState) -> Result<String, String> {
    let root = app_state.project_root.join("AssistantExports/AnimationFrames");
    fs::create_dir_all(&root).map_err(|error| format!("Slide2D Assistant Toolkit: {error}"))?;
    let mut exported = 0;
    for path in &app_state.assistant_selected_assets {
        if path.extension().and_then(|value| value.to_str()) != Some("s2anim") { continue; }
        let animation = crate::animation::SpriteAnimation::load(path)?;
        let target = root.join(path.file_stem().and_then(|value| value.to_str()).unwrap_or("animation"));
        fs::create_dir_all(&target).map_err(|error| format!("Slide2D Assistant Toolkit: {error}"))?;
        let mut files = Vec::new();
        for (index, frame) in animation.frames.iter().enumerate() {
            let source = resolve_asset_path(&app_state.project_root, frame);
            let output = target.join(format!("frame_{:04}.png", index + 1));
            fs::copy(source, &output).map_err(|error| format!("Slide2D Assistant Toolkit: {error}"))?;
            files.push(output.file_name().unwrap().to_string_lossy().into_owned());
            exported += 1;
        }
        let manifest = serde_json::json!({
            "slide2d_engine": "SLIDE2D_ANIMATION_FRAME_EXPORT",
            "animation": animation.name,
            "frames_per_second": animation.frames_per_second,
            "looping": animation.looping,
            "frames": files
        });
        fs::write(target.join("slide2d.frames.json"), serde_json::to_vec_pretty(&manifest).unwrap_or_default())
            .map_err(|error| format!("Slide2D Assistant Toolkit: {error}"))?;
    }
    Ok(format!("Slide2D Assistant Toolkit: exported {exported} frames"))
}

/// 将当前画布视角合成为PNG并保存到工程Screenshots目录。
fn export_canvas_screenshot(app_state: &AppState) -> Result<String, String> {
    use image::GenericImageView;
    let width = app_state.last_canvas_width.max(1);
    let height = app_state.last_canvas_height.max(1);
    let background = if app_state.assistant_settings.transparent_screenshot {
        image::Rgba([0, 0, 0, 0])
    } else {
        let color = app_state.editor_settings.canvas_background;
        image::Rgba([color[0], color[1], color[2], 255])
    };
    let mut canvas = image::RgbaImage::from_pixel(width, height, background);

    if let Some(tileset) = &app_state.tile_map.tileset {
        let atlas_path = resolve_asset_path(&app_state.project_root, &tileset.image_path);
        if let Ok(atlas) = image::open(atlas_path) {
            for layer in app_state.tile_map.layers.iter().filter(|layer| layer.visible) {
                for cell in &layer.cells {
                    let columns = (atlas.width() / tileset.tile_width.max(1)).max(1);
                    let source_x = cell.tile_id % columns * tileset.tile_width;
                    let source_y = cell.tile_id / columns * tileset.tile_height;
                    let frame = atlas.crop_imm(source_x, source_y, tileset.tile_width, tileset.tile_height);
                    let screen_x = app_state.view_offset_x + cell.x as f32 * app_state.tile_map.tile_width as f32 * app_state.view_zoom;
                    let screen_y = app_state.view_offset_y + cell.y as f32 * app_state.tile_map.tile_height as f32 * app_state.view_zoom;
                    let output_width = (app_state.tile_map.tile_width as f32 * app_state.view_zoom).max(1.0) as u32;
                    let output_height = (app_state.tile_map.tile_height as f32 * app_state.view_zoom).max(1.0) as u32;
                    let frame = frame.resize_exact(output_width, output_height, image::imageops::FilterType::Nearest).to_rgba8();
                    image::imageops::overlay(&mut canvas, &frame, screen_x.round() as i64, screen_y.round() as i64);
                }
            }
        }
    }

    let mut objects: Vec<&GameObject> = app_state.game_objects.iter().collect();
    objects.sort_by_key(|object| object.layer_index);
    for object in objects {
        let x = app_state.view_offset_x + object.x * app_state.view_zoom;
        let y = app_state.view_offset_y + object.y * app_state.view_zoom;
        let object_width = (object.width * app_state.view_zoom).max(1.0) as u32;
        let object_height = (object.height * app_state.view_zoom).max(1.0) as u32;
        if !object.image_path.is_empty() {
            let path = resolve_asset_path(&app_state.project_root, &object.image_path);
            if let Ok(image) = image::open(path) {
                let image = image.resize_exact(object_width, object_height, image::imageops::FilterType::Lanczos3).to_rgba8();
                image::imageops::overlay(&mut canvas, &image, x.round() as i64, y.round() as i64);
                continue;
            }
        }
        fill_rgba_rectangle(&mut canvas, x, y, object_width, object_height, image::Rgba([64, 140, 217, 255]));
    }

    for element in app_state.ui_elements.iter().filter(|element| element.visible) {
        let color = match &element.kind {
            UiElementKind::ImagePanel { image_path } if !image_path.is_empty() => {
                let path = resolve_asset_path(&app_state.project_root, image_path);
                if let Ok(image) = image::open(path) {
                    let image = image.resize_exact(element.width.max(1.0) as u32, element.height.max(1.0) as u32, image::imageops::FilterType::Lanczos3).to_rgba8();
                    image::imageops::overlay(&mut canvas, &image, element.x.round() as i64, element.y.round() as i64);
                    continue;
                }
                image::Rgba([90, 90, 100, 220])
            }
            UiElementKind::Text { color, .. } => image::Rgba(*color),
            UiElementKind::Button { .. } => image::Rgba([50, 110, 185, 230]),
            UiElementKind::ProgressBar { fill_color, .. } => image::Rgba(*fill_color),
            UiElementKind::ImagePanel { .. } => image::Rgba([90, 90, 100, 220]),
        };
        fill_rgba_rectangle(&mut canvas, element.x, element.y, element.width.max(1.0) as u32, element.height.max(1.0) as u32, color);
    }

    let directory = app_state.project_root.join("Screenshots");
    fs::create_dir_all(&directory).map_err(|error| format!("Slide2D Assistant Toolkit: {error}"))?;
    let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs()).unwrap_or(0);
    let path = directory.join(format!("Slide2D_Canvas_{timestamp}.png"));
    canvas.save_with_format(&path, image::ImageFormat::Png)
        .map_err(|error| format!("Slide2D Assistant Toolkit: {error}"))?;
    Ok(tr_args("assistant.screenshot_saved", &[("value", path.display().to_string())]))
}

/// 在截图像素缓冲区中填充一个裁剪后的矩形。
fn fill_rgba_rectangle(
    image: &mut image::RgbaImage,
    x: f32,
    y: f32,
    width: u32,
    height: u32,
    color: image::Rgba<u8>,
) {
    let start_x = x.max(0.0) as u32;
    let start_y = y.max(0.0) as u32;
    let end_x = start_x.saturating_add(width).min(image.width());
    let end_y = start_y.saturating_add(height).min(image.height());
    for pixel_y in start_y..end_y {
        for pixel_x in start_x..end_x { image.put_pixel(pixel_x, pixel_y, color); }
    }
}

/// 绘制Slide2D语言设置弹窗，选择后立即切换全部系统界面。
fn draw_language_settings(context: &egui::Context, app_state: &mut AppState) {
    if !app_state.language_settings_open {
        return;
    }
    let mut open = true;
    let mut selected = current_language();
    egui::Window::new(tr("window.language"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .show(context, |ui| {
            ui.heading(tr("language.settings"));
            let chinese_changed = ui
                .radio_value(
                    &mut selected,
                    Language::SimplifiedChinese,
                    tr("language.chinese"),
                )
                .changed();
            let english_changed = ui
                .radio_value(&mut selected, Language::English, tr("language.english"))
                .changed();
            if chinese_changed || english_changed {
                if let Err(error) = set_language(selected) {
                    app_state.status_message = error;
                }
                context.request_repaint();
            }
            ui.separator();
            ui.label(tr("localization.description"));
            ui.small(tr("localization.no_code"));
            ui.separator();
            ui.small("Slide2D Localization System");
        });
    app_state.language_settings_open = open;
}

/// 在编辑器底部实时显示缓存数量、估算内存和当前渲染统计。
fn draw_performance_status_bar(context: &egui::Context, app_state: &AppState) {
    egui::TopBottomPanel::bottom("slide2d_performance_status").show(context, |ui| {
        ui.horizontal(|ui| {
            ui.small("Slide2D Performance System");
            ui.separator();
            ui.small(format!(
                "FPS {:.1}",
                app_state.performance_metrics.frame_rate
            ));
            ui.small(format!(
                "缓存 {} 项",
                app_state.performance_metrics.cached_assets
            ));
            ui.small(format!(
                "内存 {:.2} MB",
                app_state.performance_metrics.memory_bytes as f64 / 1_048_576.0
            ));
            ui.small(format!(
                "渲染物体 {}，瓦片 {}",
                app_state.performance_metrics.rendered_objects,
                app_state.performance_metrics.rendered_tiles
            ));
        });
    });
}

/// 绘制Slide2D性能监视器及全部低配设备优化开关。
fn draw_performance_monitor(context: &egui::Context, app_state: &mut AppState) {
    if !app_state.performance_monitor_open {
        return;
    }
    update_runtime_performance_metrics(app_state);
    let mut open = true;
    egui::Window::new(tr("window.performance"))
        .open(&mut open)
        .default_size(Vec2::new(560.0, 560.0))
        .show(context, |ui| {
            ui.heading(tr("performance.realtime"));
            ui.label(tr_args(
                "performance.editor_fps",
                &[(
                    "value",
                    format!("{:.1}", app_state.performance_metrics.frame_rate),
                )],
            ));
            ui.label(tr_args(
                "performance.runtime_fps",
                &[(
                    "value",
                    format!("{:.1}", app_state.performance_metrics.runtime_frame_rate),
                )],
            ));
            ui.label(format!(
                "素材缓存：{} 项",
                app_state.performance_metrics.cached_assets
            ));
            ui.label(format!(
                "估算内存：{:.2} MB",
                app_state.performance_metrics.memory_bytes as f64 / 1_048_576.0
            ));
            ui.label(format!(
                "渲染物体：{}",
                app_state.performance_metrics.rendered_objects
            ));
            ui.label(format!(
                "渲染瓦片：{}",
                app_state.performance_metrics.rendered_tiles
            ));
            ui.label(format!(
                "蓝图执行：{:.3} ms",
                app_state.performance_metrics.blueprint_time_ms
            ));
            ui.label(format!(
                "物理计算：{:.3} ms",
                app_state.performance_metrics.physics_time_ms
            ));
            ui.separator();
            ui.heading(tr("performance.options"));
            ui.checkbox(
                &mut app_state.performance_settings.viewport_culling,
                tr("performance.viewport"),
            );
            ui.checkbox(
                &mut app_state.performance_settings.tile_chunk_culling,
                tr("performance.tile_chunks"),
            );
            ui.checkbox(
                &mut app_state.performance_settings.resource_cache,
                tr("performance.resource_cache"),
            );
            ui.checkbox(
                &mut app_state.performance_settings.idle_cache_release,
                tr("performance.idle_release"),
            );
            ui.checkbox(
                &mut app_state.performance_settings.blueprint_cache,
                tr("performance.blueprint_cache"),
            );
            ui.checkbox(
                &mut app_state.performance_settings.dormant_blueprints,
                tr("performance.dormant"),
            );
            ui.checkbox(
                &mut app_state.performance_settings.static_physics_cache,
                tr("performance.static_physics"),
            );
            ui.checkbox(
                &mut app_state.performance_settings.distant_physics_sleep,
                tr("performance.distant_physics"),
            );
            ui.checkbox(
                &mut app_state.performance_settings.automatic_image_compression,
                tr("performance.image_compression"),
            );
            ui.add(
                egui::Slider::new(
                    &mut app_state.performance_settings.activity_margin,
                    64.0..=2048.0,
                )
                .text(tr("performance.margin")),
            );
            ui.separator();
            ui.small("Slide2D Performance System");
            ui.small(tr("performance.no_code"));
        });
    app_state.performance_monitor_open = open;
}

/// 从Runtime临时指标JSON读取游戏帧率、蓝图耗时和物理耗时。
fn update_runtime_performance_metrics(app_state: &mut AppState) {
    let path = std::env::temp_dir().join("slide2d_runtime_performance.json");
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(_) => return,
    };
    let report = match serde_json::from_slice::<crate::app_state::RuntimePerformanceReport>(&bytes)
    {
        Ok(report) if report.slide2d_engine == "SLIDE2D_PERFORMANCE_SYSTEM" => report,
        _ => return,
    };
    app_state.performance_metrics.runtime_frame_rate = report.frame_rate;
    if report.frame_rate > 0.0 {
        app_state.performance_metrics.memory_bytes = report.memory_bytes;
        app_state.performance_metrics.cached_assets = report.cached_assets;
        app_state.performance_metrics.rendered_objects = report.rendered_objects;
        app_state.performance_metrics.rendered_tiles = report.rendered_tiles;
    }
    app_state.performance_metrics.blueprint_time_ms = report.blueprint_time_ms;
    app_state.performance_metrics.physics_time_ms = report.physics_time_ms;
}

/// 绘制Slide2D Plugin System插件管理器，支持热启停、导入和删除。
fn draw_plugin_manager(
    context: &egui::Context,
    app_state: &mut AppState,
    asset_library: &mut AssetLibrary,
) {
    if !app_state.plugin_manager_open {
        return;
    }
    let mut open = true;
    let mut enable_change = None;
    let mut delete_directory = None;
    let mut refresh = false;
    egui::Window::new(tr("window.plugin"))
        .open(&mut open)
        .default_size(Vec2::new(720.0, 520.0))
        .show(context, |ui| {
            ui.horizontal(|ui| {
                if ui.button(tr("plugin.import")).clicked() {
                    if let Some(folder) = rfd::FileDialog::new()
                        .set_title(tr("dialog.import_plugin"))
                        .pick_folder()
                    {
                        app_state.status_message =
                            match import_plugin_folder(&folder, &app_state.project_root) {
                                Ok(path) => {
                                    refresh = true;
                                    format!("插件已导入：{}", path.display())
                                }
                                Err(error) => error,
                            };
                    }
                }
                if ui.button(tr("plugin.refresh")).clicked() {
                    refresh = true;
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for plugin in &app_state.plugin_registry.installed {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let mut enabled = plugin.enabled;
                            if ui.checkbox(&mut enabled, tr("plugin.enabled")).changed() {
                                enable_change = Some((plugin.manifest.plugin_id.clone(), enabled));
                            }
                            ui.strong(&plugin.manifest.name);
                            ui.label(format!("v{}", plugin.manifest.version));
                            ui.label(tr_args(
                                "plugin.author",
                                &[("value", plugin.manifest.author.clone())],
                            ));
                        });
                        ui.label(&plugin.manifest.description);
                        ui.small(tr_args(
                            "plugin.id",
                            &[("value", plugin.manifest.plugin_id.clone())],
                        ));
                        ui.small(tr_args(
                            "plugin.counts",
                            &[
                                ("nodes", plugin.manifest.nodes.len().to_string()),
                                ("resources", plugin.manifest.resources.len().to_string()),
                                ("tools", plugin.manifest.editor_tools.len().to_string()),
                                (
                                    "runtime",
                                    plugin.manifest.runtime_capabilities.len().to_string(),
                                ),
                            ],
                        ));
                        if let Some(error) = &plugin.load_error {
                            ui.colored_label(Color32::RED, error);
                        }
                        let is_official =
                            plugin.manifest.plugin_id == crate::plugins::OFFICIAL_PICKUP_PLUGIN_ID;
                        if ui
                            .add_enabled(!is_official, egui::Button::new(tr("plugin.delete")))
                            .clicked()
                        {
                            delete_directory = Some(plugin.directory.clone());
                        }
                        if is_official {
                            ui.small(tr("plugin.official_notice"));
                        }
                    });
                    ui.add_space(6.0);
                }
            });
            ui.separator();
            ui.small(tr("plugin.no_code"));
            ui.small("Made by Slide2D");
        });
    app_state.plugin_manager_open = open;
    if let Some((plugin_id, enabled)) = enable_change {
        app_state.plugin_registry.set_enabled(&plugin_id, enabled);
        app_state.status_message = if enabled {
            "插件已启用，扩展立即生效"
        } else {
            "插件已禁用，扩展已立即移除"
        }
        .to_owned();
        asset_library.refresh();
    }
    if let Some(directory) = delete_directory {
        app_state.status_message = match delete_plugin(&directory, &app_state.project_root) {
            Ok(()) => {
                refresh = true;
                "本地插件已删除".to_owned()
            }
            Err(error) => error,
        };
    }
    if refresh {
        app_state.plugin_registry.refresh();
        asset_library.refresh();
    }
}

/// 绘制插件注册的声明式独立工具窗口，不提供代码或脚本输入区域。
fn draw_plugin_tool_window(context: &egui::Context, app_state: &mut AppState) {
    let (plugin_id, tool_id) = match app_state.open_plugin_tool.clone() {
        Some(value) => value,
        None => return,
    };
    let plugin = match app_state
        .plugin_registry
        .installed
        .iter()
        .find(|plugin| plugin.enabled && plugin.manifest.plugin_id == plugin_id)
    {
        Some(value) => value,
        None => {
            app_state.open_plugin_tool = None;
            return;
        }
    };
    let tool = match plugin
        .manifest
        .editor_tools
        .iter()
        .find(|tool| tool.tool_id == tool_id)
    {
        Some(value) => value,
        None => {
            app_state.open_plugin_tool = None;
            return;
        }
    };
    let mut open = true;
    egui::Window::new(format!("Slide2D Plugin System - {}", tool.display_name))
        .open(&mut open)
        .show(context, |ui| {
            ui.heading(&plugin.manifest.name);
            ui.label(&tool.description);
            ui.separator();
            ui.label(tr("plugin.registered_nodes"));
            for node in &plugin.manifest.nodes {
                ui.label(format!("- {}：{}", node.display_name, node.description));
            }
            ui.label(tr("plugin.runtime_capabilities"));
            for behavior in &plugin.manifest.runtime_capabilities {
                ui.label(format!("- {}", plugin_behavior_label(behavior)));
            }
            ui.separator();
            ui.small("Slide2D Plugin System | Made by Slide2D");
        });
    if !open {
        app_state.open_plugin_tool = None;
    }
}

/// 返回插件Runtime白名单能力的中文说明。
fn plugin_behavior_label(behavior: &PluginBehavior) -> &'static str {
    match behavior {
        PluginBehavior::SceneLoadedEvent => "场景加载事件",
        PluginBehavior::ObjectClickedEvent => "物体点击事件",
        PluginBehavior::PickupCheck => "道具拾取判定",
        PluginBehavior::SetObjectVariable => "物体变量赋值",
        PluginBehavior::SetGlobalVariable => "全局变量赋值",
        PluginBehavior::MoveHorizontal => "物理横向移动",
        PluginBehavior::NumberVariable => "数值变量",
    }
}

/// 选择另一个文件夹，并将工程清单、场景和素材保存到该目录。
fn save_project_as(
    app_state: &mut AppState,
    asset_library: &mut AssetLibrary,
) -> Result<(), String> {
    let path = rfd::FileDialog::new()
        .set_title(tr("dialog.save_folder"))
        .pick_folder()
        .ok_or_else(|| "已取消另存为".to_owned())?;
    let enabled_plugins = app_state.plugin_registry.enabled_ids();
    save_project_folder(app_state, &path)?;
    app_state.plugin_registry =
        crate::plugins::PluginRegistry::load(path.clone(), &enabled_plugins);
    std::env::set_current_dir(&path)
        .map_err(|error| format!("切换到另存为工程目录失败：{error}"))?;
    *asset_library = AssetLibrary::new(path)?;
    Ok(())
}

/// 保存当前工程、更新最近列表，并显示带Slide2D品牌标注的完成弹窗。
fn save_current_project(app_state: &mut AppState, asset_library: &mut AssetLibrary) {
    let result = if let Some(path) = app_state.project_file_path.clone() {
        save_project_folder(app_state, &path)
    } else {
        save_project_as(app_state, asset_library)
    };
    match result {
        Ok(()) => {
            app_state.mark_project_saved();
            app_state.save_notice_open = true;
            app_state.status_message = "Slide2D工程已完整保存".to_owned();
            if let Some(path) = app_state.project_file_path.clone() {
                let _ = remember_recent_project(&mut app_state.recent_projects, &path);
            }
        }
        Err(error) => app_state.status_message = error,
    }
}

/// 打开指定工程文件夹并更新最近打开工程列表。
fn open_project_from_folder(
    path: &std::path::Path,
    app_state: &mut AppState,
    asset_library: &mut AssetLibrary,
) {
    match open_project_folder(path) {
        Ok(mut state) => {
            let _ = std::env::set_current_dir(&state.project_root);
            match AssetLibrary::new(state.project_root.clone()) {
                Ok(library) => {
                    state.status_message = format!("Slide2D工程已打开：{}", path.display());
                    state.recent_projects = app_state.recent_projects.clone();
                    let _ = remember_recent_project(&mut state.recent_projects, path);
                    state.mark_project_saved();
                    *asset_library = library;
                    *app_state = state;
                }
                Err(error) => app_state.status_message = error,
            }
        }
        Err(error) => app_state.status_message = error,
    }
}

/// 绘制未保存修改确认框，并在用户选择后继续新建或退出动作。
fn draw_unsaved_changes_dialog(
    context: &egui::Context,
    app_state: &mut AppState,
    asset_library: &mut AssetLibrary,
) {
    let action = match app_state.pending_editor_action {
        Some(action) => action,
        None => return,
    };
    egui::Window::new(tr("dialog.unsaved.title"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .show(context, |ui| {
            ui.label(tr("dialog.unsaved.message"));
            ui.horizontal(|ui| {
                if ui.button(tr("dialog.unsaved.save_continue")).clicked() {
                    save_current_project(app_state, asset_library);
                    if !app_state.has_unsaved_changes() {
                        app_state.pending_editor_action = None;
                        continue_pending_editor_action(action, app_state, asset_library);
                    }
                }
                if ui.button(tr("dialog.unsaved.discard")).clicked() {
                    app_state.pending_editor_action = None;
                    continue_pending_editor_action(action, app_state, asset_library);
                }
                if ui.button(tr("common.cancel")).clicked() {
                    app_state.pending_editor_action = None;
                }
            });
            ui.separator();
            ui.small(tr("help.title"));
        });
}

/// 在未保存确认完成后执行原定的新建或退出操作。
fn continue_pending_editor_action(
    action: PendingEditorAction,
    app_state: &mut AppState,
    asset_library: &mut AssetLibrary,
) {
    match action {
        PendingEditorAction::NewProject => {
            if let Err(error) = create_new_project(app_state, asset_library) {
                app_state.status_message = error;
            }
        }
        PendingEditorAction::ExitEditor => app_state.exit_requested = true,
    }
}

/// 绘制工程保存完成提示，底部永久标注Slide2D品牌。
fn draw_save_notice_dialog(context: &egui::Context, app_state: &mut AppState) {
    if !app_state.save_notice_open {
        return;
    }
    egui::Window::new(tr("dialog.save.title"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .show(context, |ui| {
            ui.label(tr("dialog.save.message"));
            if ui.button(tr("common.confirm")).clicked() {
                app_state.save_notice_open = false;
            }
            ui.separator();
            ui.small(tr("help.title"));
        });
}

/// 选择工程文件夹，并在其中创建工程清单、素材目录和初始场景。
fn create_new_project(
    app_state: &mut AppState,
    asset_library: &mut AssetLibrary,
) -> Result<(), String> {
    let project_root = match rfd::FileDialog::new()
        .set_title(tr("dialog.new_project"))
        .pick_folder()
    {
        Some(path) => path,
        None => return Ok(()),
    };
    let mut new_state = AppState::new();
    new_state.project_root = project_root.clone();
    new_state.project_file_path = Some(project_root.clone());
    new_state.plugin_registry =
        crate::plugins::PluginRegistry::load_new_project(project_root.clone());

    let new_asset_library = AssetLibrary::new(project_root.clone())?;
    save_project_folder(&mut new_state, &project_root)?;
    std::env::set_current_dir(&project_root)
        .map_err(|error| format!("切换到新工程目录失败：{error}"))?;
    new_state.status_message = format!("新工程文件夹已创建：{}", project_root.display());
    new_state.recent_projects = app_state.recent_projects.clone();
    let _ = remember_recent_project(&mut new_state.recent_projects, &project_root);
    new_state.mark_project_saved();
    *asset_library = new_asset_library;
    *app_state = new_state;
    Ok(())
}

/// 弹出文件保存对话框，将整个工程打包为单个.slide2d文件。
fn export_project_package(app_state: &mut AppState) -> Result<std::path::PathBuf, String> {
    let path = rfd::FileDialog::new()
        .set_title(tr("dialog.export_project"))
        .add_filter("Slide2D项目包", &["slide2d"])
        .set_file_name("导出项目.slide2d")
        .save_file()
        .ok_or_else(|| "已取消导出项目".to_owned())?;
    let path = ensure_project_extension(path);
    let project_folder = app_state.project_file_path.clone();
    save_project(app_state, &path)?;
    app_state.project_file_path = project_folder;
    Ok(path)
}

/// 绘制可以直接拖入画布的四种游戏UI组件。
fn draw_ui_component_library(ui: &mut egui::Ui, app_state: &mut AppState) {
    ui.horizontal_wrapped(|ui| {
        for (template, name) in [
            (UiTemplate::Text, tr("ui_template.text")),
            (UiTemplate::Button, tr("ui_template.button")),
            (UiTemplate::ProgressBar, tr("ui_template.progress")),
            (UiTemplate::ImagePanel, tr("ui_template.image")),
        ] {
            let response = ui.dnd_drag_source(
                egui::Id::new(("ui_template", name.clone())),
                UiDragPayload { template },
                |ui| {
                    let _ = ui.button(name);
                },
            );
            if response.response.dragged_by(egui::PointerButton::Primary) {
                app_state.dragging_ui_template = Some(template);
            }
        }
    });
}

/// 绘制IDE风格的顶部标签栏，并在场景和蓝图工作区之间切换。
fn draw_workspace_tabs(
    ui: &mut egui::Ui,
    app_state: &mut AppState,
    editor_textures: &mut EditorTextures,
) {
    ui.horizontal(|ui| {
        if ui
            .selectable_label(!app_state.blueprint_tab_active, "场景")
            .clicked()
        {
            app_state.blueprint_tab_active = false;
            app_state.pending_blueprint_output = None;
        }

        if app_state.blueprint_owner_is_open() {
            let tab_title = if let Some(ui_id) = app_state.blueprint_ui_id {
                format!("蓝图 - UI {ui_id}")
            } else {
                format!(
                    "蓝图 - 物体 {}",
                    app_state.blueprint_object_id.unwrap_or_default()
                )
            };
            if ui
                .selectable_label(app_state.blueprint_tab_active, tab_title)
                .clicked()
            {
                app_state.blueprint_tab_active = true;
            }
            if ui.small_button("×").clicked() {
                app_state.close_blueprint_owner();
                app_state.blueprint_tab_active = false;
                app_state.pending_blueprint_output = None;
                app_state.selected_blueprint_node_id = None;
            }
        }
    });

    ui.separator();

    if app_state.blueprint_tab_active && app_state.blueprint_owner_is_open() {
        draw_blueprint_contents(ui, app_state);
    } else {
        let canvas_rect = ui.available_rect_before_wrap();
        draw_canvas(ui, canvas_rect, app_state, editor_textures);
    }
}

/// 递归收集animations目录中的所有.s2anim文件。
fn collect_animation_files(directory: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) => return files,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(collect_animation_files(&path));
        } else if path
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("s2anim"))
        {
            files.push(path);
        }
    }
    files.sort();
    files
}

/// 绘制左侧资源素材管理器，包括导入、文件夹和树形资源列表。
fn draw_asset_library(
    ui: &mut egui::Ui,
    app_state: &mut AppState,
    editor_textures: &mut EditorTextures,
    asset_library: &mut AssetLibrary,
) {
    asset_library.automatic_image_compression =
        app_state.performance_settings.automatic_image_compression;
    if app_state.asset_refresh_requested {
        asset_library.refresh();
        app_state.asset_refresh_requested = false;
    }
    if asset_library.refresh_if_due() {
        app_state.status_message = "资源库已自动扫描并刷新".to_owned();
    }
    ui.horizontal(|ui| {
        if ui.button(tr("resource.refresh")).clicked() {
            asset_library.refresh();
            app_state.status_message = "资源库已手动刷新".to_owned();
        }
        ui.label(egui::RichText::new(tr("resource.auto_scan")).small());
    });
    if ui.button(tr("resource.create_animation")).clicked() {
        app_state.animation_editor.create_new();
    }
    if ui.button(tr("resource.open_tile_editor")).clicked() {
        app_state.tile_editor.window_open = true;
    }
    ui.horizontal_wrapped(|ui| {
        if ui.button(tr("resource.save_actor")).clicked() {
            match save_selected_actor_asset(app_state, asset_library) {
                Ok(path) => app_state.status_message = format!("Actor已保存：{}", path.display()),
                Err(error) => app_state.status_message = error,
            }
            asset_library.refresh();
        }
        if ui.button(tr("resource.save_blueprint")).clicked() {
            match save_selected_blueprint_asset(app_state, asset_library) {
                Ok(path) => {
                    app_state.status_message = format!("蓝图模板已保存：{}", path.display())
                }
                Err(error) => app_state.status_message = error,
            }
            asset_library.refresh();
        }
    });
    if ui.button(tr("resource.import_png")).clicked() {
        match asset_library.import_png_images() {
            Ok(imported_assets) => {
                for imported_asset in imported_assets {
                    let _ = ensure_editor_texture(ui.ctx(), editor_textures, &imported_asset.path);
                    app_state.status_message =
                        format!("PNG图片已导入：{}", imported_asset.path.display());
                }
                asset_library.refresh();
            }
            Err(error) => app_state.status_message = error,
        }
    }
    if ui.button(tr("resource.import_file")).clicked() {
        match asset_library.import_files() {
            Ok(imported_assets) => {
                for imported_asset in imported_assets {
                    match imported_asset.kind {
                        AssetKind::Image => {
                            let _ = ensure_editor_texture(
                                ui.ctx(),
                                editor_textures,
                                &imported_asset.path,
                            );
                            app_state.status_message =
                                format!("图片已导入：{}", imported_asset.path.display());
                        }
                        AssetKind::Audio => {
                            app_state.status_message =
                                format!("音效已导入：{}", imported_asset.path.display());
                        }
                        AssetKind::Animation => {
                            app_state.status_message =
                                format!("动画已导入：{}", imported_asset.path.display());
                        }
                        AssetKind::Tileset => {
                            app_state.status_message =
                                format!("瓦片集已导入：{}", imported_asset.path.display());
                        }
                    }
                }
                asset_library.refresh();
            }
            Err(error) => app_state.status_message = error,
        }
    }

    ui.collapsing("新建文件夹", |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.radio_value(
                &mut asset_library.new_folder_category,
                AssetCategory::Actor,
                tr("resource.actor"),
            );
            ui.radio_value(
                &mut asset_library.new_folder_category,
                AssetCategory::Blueprint,
                tr("resource.blueprint"),
            );
            ui.radio_value(
                &mut asset_library.new_folder_category,
                AssetCategory::Animation,
                tr("resource.animation"),
            );
            ui.radio_value(
                &mut asset_library.new_folder_category,
                AssetCategory::AssetsTextures,
                tr("resource.textures"),
            );
            ui.radio_value(
                &mut asset_library.new_folder_category,
                AssetCategory::AssetsAudio,
                tr("resource.audio"),
            );
            ui.radio_value(
                &mut asset_library.new_folder_category,
                AssetCategory::AssetsTilesets,
                tr("resource.tilesets"),
            );
            ui.radio_value(
                &mut asset_library.new_folder_category,
                AssetCategory::AssetsUi,
                tr("resource.ui"),
            );
        });
        if let Some(parent) = &asset_library.new_folder_parent {
            ui.label(tr_args(
                "resource.parent",
                &[("value", parent.display().to_string())],
            ));
            if ui.small_button(tr("resource.restore_root")).clicked() {
                asset_library.new_folder_parent = None;
            }
        }
        ui.text_edit_singleline(&mut asset_library.new_folder_name);
        if ui.button(tr("resource.create_folder")).clicked() {
            if let Err(error) = asset_library.create_folder() {
                app_state.status_message = error;
            }
        }
    });

    ui.separator();
    let mut resource_action = None;
    egui::ScrollArea::vertical()
        // 资源树只能纵向滚动，不能被长文件名撑到面板右侧。
        .auto_shrink([true, false])
        .show(ui, |ui| {
            ui.set_max_width(ui.available_width());
            egui::CollapsingHeader::new(tr("resource.actor"))
                .default_open(true)
                .show(ui, |ui| {
                    draw_actor_asset_entries(
                        ui,
                        asset_library.scan_actors(),
                        app_state,
                        &mut resource_action,
                    );
                });
            egui::CollapsingHeader::new(tr("resource.blueprint"))
                .default_open(true)
                .show(ui, |ui| {
                    draw_blueprint_asset_entries(
                        ui,
                        asset_library.scan_blueprints(),
                        app_state,
                        &mut resource_action,
                    );
                });
            egui::CollapsingHeader::new(tr("resource.animation"))
                .default_open(true)
                .show(ui, |ui| {
                    draw_animation_asset_entries(
                        ui,
                        asset_library.scan_animations(),
                        app_state,
                        &mut resource_action,
                    );
                });
            egui::CollapsingHeader::new(tr("resource.assets"))
                .default_open(true)
                .show(ui, |ui| {
                    egui::CollapsingHeader::new(tr("resource.textures")).show(ui, |ui| {
                        draw_image_asset_entries(
                            ui,
                            asset_library.scan_images(),
                            app_state,
                            editor_textures,
                            &mut resource_action,
                        );
                    });
                    egui::CollapsingHeader::new(tr("resource.audio")).show(ui, |ui| {
                        draw_audio_asset_entries(
                            ui,
                            asset_library.scan_audio(),
                            app_state,
                            &mut resource_action,
                        );
                    });
                    egui::CollapsingHeader::new(tr("resource.tilesets")).show(ui, |ui| {
                        draw_tileset_asset_entries(
                            ui,
                            asset_library.scan_tilesets(),
                            app_state,
                            &mut resource_action,
                        );
                    });
                    egui::CollapsingHeader::new(tr("resource.ui")).show(ui, |ui| {
                        draw_image_asset_entries(
                            ui,
                            asset_library.scan_ui(),
                            app_state,
                            editor_textures,
                            &mut resource_action,
                        );
                    });
                    egui::CollapsingHeader::new(tr("resource.plugin")).show(ui, |ui| {
                        let plugin_resources: Vec<_> = app_state
                            .plugin_registry
                            .installed
                            .iter()
                            .filter(|plugin| plugin.enabled && plugin.load_error.is_none())
                            .flat_map(|plugin| plugin.manifest.resources.iter().cloned())
                            .collect();
                        if plugin_resources.is_empty() {
                            ui.label(tr("resource.no_plugin"));
                        }
                        for definition in plugin_resources {
                            egui::CollapsingHeader::new(&definition.display_name).show(ui, |ui| {
                                let entries =
                                    scan_plugin_resource(&app_state.project_root, &definition);
                                draw_plugin_resource_entries(ui, &entries, &mut resource_action);
                            });
                        }
                    });
                });
        });
    apply_resource_action(resource_action, app_state, asset_library);
    draw_resource_rename_window(ui.ctx(), app_state, asset_library);
}

/// 递归绘制插件注册的自定义资源类型及统一右键菜单。
fn draw_plugin_resource_entries(
    ui: &mut egui::Ui,
    entries: &[AssetEntry],
    action: &mut Option<ResourceAction>,
) {
    for entry in entries {
        match entry {
            AssetEntry::Folder {
                name,
                path,
                children,
            } => {
                let folder = egui::CollapsingHeader::new(name)
                    .id_source(("plugin_resource_folder", path))
                    .show(ui, |ui| draw_plugin_resource_entries(ui, children, action));
                add_resource_context_menu(&folder.header_response, path, action);
            }
            AssetEntry::File { name, path } => {
                let response = ui.selectable_label(false, name);
                add_resource_context_menu(&response, path, action);
            }
        }
    }
}

/// 资源树右键菜单在递归绘制结束后执行的文件动作。
enum ResourceAction {
    Open(std::path::PathBuf),
    Delete(std::path::PathBuf),
    Duplicate(std::path::PathBuf),
    Rename(std::path::PathBuf),
    NewFolder(std::path::PathBuf),
}

/// 给任意资源或文件夹添加统一右键菜单。
fn add_resource_context_menu(
    response: &egui::Response,
    path: &std::path::Path,
    action: &mut Option<ResourceAction>,
) {
    response.context_menu(|ui| {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        if path.is_file()
            && matches!(extension, "s2anim" | "s2tileset" | "s2blueprint" | "json")
            && ui.button(tr("resource.open_editor")).clicked()
        {
            *action = Some(ResourceAction::Open(path.to_path_buf()));
            ui.close_menu();
        }
        if path.is_dir() && ui.button(tr("resource.folder_here")).clicked() {
            *action = Some(ResourceAction::NewFolder(path.to_path_buf()));
            ui.close_menu();
        }
        if ui.button(tr("common.rename")).clicked() {
            *action = Some(ResourceAction::Rename(path.to_path_buf()));
            ui.close_menu();
        }
        if ui.button(tr("common.copy")).clicked() {
            *action = Some(ResourceAction::Duplicate(path.to_path_buf()));
            ui.close_menu();
        }
        if ui.button(tr("common.delete")).clicked() {
            *action = Some(ResourceAction::Delete(path.to_path_buf()));
            ui.close_menu();
        }
    });
}

/// 执行右键资源动作并刷新资源树。
fn apply_resource_action(
    action: Option<ResourceAction>,
    app_state: &mut AppState,
    asset_library: &mut AssetLibrary,
) {
    let action = match action {
        Some(value) => value,
        None => return,
    };
    let result = match action {
        ResourceAction::Open(path) => open_resource_editor(&path, app_state),
        ResourceAction::Delete(path) => delete_resource(&path).map(|_| "资源已删除".to_owned()),
        ResourceAction::Duplicate(path) => {
            duplicate_resource(&path).map(|output| format!("资源已复制：{}", output.display()))
        }
        ResourceAction::Rename(path) => {
            asset_library.rename_text = path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("")
                .to_owned();
            asset_library.rename_target = Some(path);
            Ok("请输入新的资源名称".to_owned())
        }
        ResourceAction::NewFolder(path) => {
            asset_library.new_folder_parent = Some(path);
            Ok("请在上方输入文件夹名称".to_owned())
        }
    };
    app_state.status_message = result.unwrap_or_else(|error| error);
    asset_library.refresh();
}

/// 根据资源扩展名打开动画、蓝图或瓦片编辑器。
fn open_resource_editor(
    path: &std::path::Path,
    app_state: &mut AppState,
) -> Result<String, String> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    match extension {
        "s2anim" => {
            app_state.animation_editor.open(path.to_path_buf())?;
            Ok("已打开Slide2D动画编辑器".to_owned())
        }
        "s2tileset" => {
            let tileset = TileSet::load(path)?;
            app_state.tile_map.tileset_path = project_relative_asset_path(path);
            app_state.tile_map.tile_width = tileset.tile_width;
            app_state.tile_map.tile_height = tileset.tile_height;
            app_state.tile_editor.selected_tileset = Some(tileset.clone());
            app_state.tile_map.tileset = Some(tileset);
            app_state.tile_editor.selected_tileset_path = Some(path.to_path_buf());
            app_state.tile_editor.window_open = true;
            Ok("已打开Slide2D瓦片编辑器".to_owned())
        }
        "s2blueprint" | "json" => {
            open_blueprint_asset(path, app_state);
            Ok("已打开Slide2D蓝图编辑器".to_owned())
        }
        _ => Err("此资源没有对应的Slide2D编辑器".to_owned()),
    }
}

/// 绘制资源重命名小窗口，确认后保留原文件扩展名。
fn draw_resource_rename_window(
    context: &egui::Context,
    app_state: &mut AppState,
    asset_library: &mut AssetLibrary,
) {
    let target = match asset_library.rename_target.clone() {
        Some(path) => path,
        None => return,
    };
    egui::Window::new(tr("dialog.rename_resource"))
        .collapsible(false)
        .resizable(false)
        .show(context, |ui| {
            ui.label(target.display().to_string());
            ui.text_edit_singleline(&mut asset_library.rename_text);
            ui.horizontal(|ui| {
                if ui.button(tr("common.confirm")).clicked() {
                    app_state.status_message =
                        match rename_resource(&target, &asset_library.rename_text) {
                            Ok(path) => format!("资源已重命名：{}", path.display()),
                            Err(error) => error,
                        };
                    asset_library.rename_target = None;
                    asset_library.refresh();
                }
                if ui.button(tr("common.cancel")).clicked() {
                    asset_library.rename_target = None;
                }
            });
        });
}

/// 将当前选中物体保存为可重复放置的.s2actor资源。
fn save_selected_actor_asset(
    app_state: &AppState,
    asset_library: &AssetLibrary,
) -> Result<std::path::PathBuf, String> {
    let object_id = app_state
        .selected_object_id
        .ok_or("请先在画布选中一个物体")?;
    let object = app_state
        .game_objects
        .iter()
        .find(|object| object.id == object_id)
        .ok_or("选中的物体不存在")?;
    let name = format!("Actor_{object_id}");
    let path = unique_named_resource(&asset_library.actor_root(), &name, "s2actor");
    ActorAsset::from_game_object(name, object).save(&path)?;
    Ok(path)
}

/// 将当前选中物体的蓝图保存为独立.s2blueprint模板。
fn save_selected_blueprint_asset(
    app_state: &AppState,
    asset_library: &AssetLibrary,
) -> Result<std::path::PathBuf, String> {
    let object_id = app_state
        .selected_object_id
        .ok_or("请先在画布选中一个物体")?;
    let object = app_state
        .game_objects
        .iter()
        .find(|object| object.id == object_id)
        .ok_or("选中的物体不存在")?;
    let path = unique_named_resource(
        &asset_library.blueprint_root(),
        &format!("Blueprint_{object_id}"),
        "s2blueprint",
    );
    let bytes = serde_json::to_vec_pretty(&object.blueprint)
        .map_err(|error| format!("生成Slide2D蓝图模板失败：{error}"))?;
    fs::write(&path, bytes).map_err(|error| format!("保存Slide2D蓝图模板失败：{error}"))?;
    Ok(path)
}

/// 双击蓝图模板时，将模板复制到当前物体并打开蓝图编辑器。
fn open_blueprint_asset(path: &std::path::Path, app_state: &mut AppState) {
    let object_id = match app_state.selected_object_id {
        Some(id) => id,
        None => {
            app_state.status_message = "请先选择要绑定蓝图模板的Actor".to_owned();
            return;
        }
    };
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            app_state.status_message = format!("读取Slide2D蓝图模板失败：{error}");
            return;
        }
    };
    let blueprint: Blueprint = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(error) => {
            app_state.status_message = format!("解析Slide2D蓝图模板失败：{error}");
            return;
        }
    };
    if let Some(object) = app_state
        .game_objects
        .iter_mut()
        .find(|object| object.id == object_id)
    {
        object.blueprint = blueprint;
    }
    app_state.blueprint_object_id = Some(object_id);
    app_state.blueprint_ui_id = None;
    app_state.blueprint_tab_active =
        app_state.editor_settings.blueprint_editor_mode == BlueprintEditorMode::IdeTabs;
    app_state.status_message = format!("蓝图模板已绑定：{}", path.display());
}

/// 在资源目录中生成不会覆盖现有文件的名称。
fn unique_named_resource(
    directory: &std::path::Path,
    stem: &str,
    extension: &str,
) -> std::path::PathBuf {
    let initial = directory.join(format!("{stem}.{extension}"));
    if !initial.exists() {
        return initial;
    }
    let mut number = 1;
    loop {
        let path = directory.join(format!("{stem}_{number}.{extension}"));
        if !path.exists() {
            return path;
        }
        number += 1;
    }
}

/// 递归绘制Actor资源，拖到画布后生成完整实体物体。
fn draw_actor_asset_entries(
    ui: &mut egui::Ui,
    entries: &[AssetEntry],
    app_state: &mut AppState,
    action: &mut Option<ResourceAction>,
) {
    for entry in entries {
        match entry {
            AssetEntry::Folder {
                name,
                path,
                children,
            } => {
                let response = egui::CollapsingHeader::new(name)
                    .id_source(("actor_folder", path))
                    .show(ui, |ui| {
                        draw_actor_asset_entries(ui, children, app_state, action)
                    })
                    .header_response;
                add_resource_context_menu(&response, path, action);
            }
            AssetEntry::File { name, path } => {
                let drag = ui.dnd_drag_source(
                    egui::Id::new(("actor_asset", path)),
                    ActorAssetDragPayload { path: path.clone() },
                    |ui| ui.selectable_label(false, format!("Actor：{name}")),
                );
                if drag.response.dragged_by(egui::PointerButton::Primary) {
                    app_state.dragging_actor_asset = Some(path.clone());
                }
                add_resource_context_menu(&drag.response, path, action);
            }
        }
    }
}

/// 递归绘制蓝图模板，双击后绑定到当前选中物体并打开蓝图编辑器。
fn draw_blueprint_asset_entries(
    ui: &mut egui::Ui,
    entries: &[AssetEntry],
    app_state: &mut AppState,
    action: &mut Option<ResourceAction>,
) {
    for entry in entries {
        match entry {
            AssetEntry::Folder {
                name,
                path,
                children,
            } => {
                let response = egui::CollapsingHeader::new(name)
                    .id_source(("blueprint_folder", path))
                    .show(ui, |ui| {
                        draw_blueprint_asset_entries(ui, children, app_state, action)
                    })
                    .header_response;
                add_resource_context_menu(&response, path, action);
            }
            AssetEntry::File { name, path } => {
                let response = ui.selectable_label(false, format!("蓝图：{name}"));
                if response.double_clicked() {
                    open_blueprint_asset(path, app_state);
                }
                add_resource_context_menu(&response, path, action);
            }
        }
    }
}

/// 递归绘制动画资源，支持拖拽绑定和双击打开动画编辑器。
fn draw_animation_asset_entries(
    ui: &mut egui::Ui,
    entries: &[AssetEntry],
    app_state: &mut AppState,
    action: &mut Option<ResourceAction>,
) {
    for entry in entries {
        match entry {
            AssetEntry::Folder {
                name,
                path,
                children,
            } => {
                let response = egui::CollapsingHeader::new(name)
                    .id_source(("animation_folder", path))
                    .show(ui, |ui| {
                        draw_animation_asset_entries(ui, children, app_state, action)
                    });
                add_resource_context_menu(&response.header_response, path, action);
            }
            AssetEntry::File { name, path } => {
                let drag = ui.dnd_drag_source(
                    egui::Id::new(("animation_asset", path)),
                    AnimationAssetDragPayload { path: path.clone() },
                    |ui| ui.selectable_label(false, format!("动画：{name}")),
                );
                if drag.response.dragged_by(egui::PointerButton::Primary) {
                    app_state.dragging_animation_asset = Some(path.clone());
                }
                if drag.response.double_clicked() {
                    if let Err(error) = app_state.animation_editor.open(path.clone()) {
                        app_state.status_message = error;
                    }
                }
                add_resource_context_menu(&drag.response, path, action);
            }
        }
    }
}

/// 递归绘制图片文件夹和可拖拽图片缩略图。
fn draw_image_asset_entries(
    ui: &mut egui::Ui,
    entries: &[AssetEntry],
    app_state: &mut AppState,
    editor_textures: &mut EditorTextures,
    action: &mut Option<ResourceAction>,
) {
    for entry in entries {
        match entry {
            AssetEntry::Folder {
                name,
                path,
                children,
            } => {
                let response = egui::CollapsingHeader::new(name)
                    .id_source(("image_folder", path))
                    .show(ui, |ui| {
                        draw_image_asset_entries(ui, children, app_state, editor_textures, action)
                    });
                add_resource_context_menu(&response.header_response, path, action);
            }
            AssetEntry::File { name, path } => {
                let texture = ensure_editor_texture(ui.ctx(), editor_textures, path).ok();
                let drag_response = ui.dnd_drag_source(
                    egui::Id::new(("image_asset", path)),
                    ImageAssetDragPayload { path: path.clone() },
                    |ui| {
                        ui.horizontal(|ui| {
                            if let Some(texture) = &texture {
                                ui.add(
                                    egui::Image::new(texture)
                                        .fit_to_exact_size(Vec2::new(48.0, 48.0)),
                                );
                            }
                            // 截断过长文件名，避免拖拽行扩大资源面板内容宽度。
                            ui.add_sized(
                                [ui.available_width(), 20.0],
                                egui::Label::new(name).truncate(),
                            );
                        });
                    },
                );
                if drag_response
                    .response
                    .dragged_by(egui::PointerButton::Primary)
                {
                    app_state.dragging_image_asset = Some(path.clone());
                }
                add_resource_context_menu(&drag_response.response, path, action);
            }
        }
    }
}

/// 递归绘制音效文件夹和音效文件名称。
fn draw_audio_asset_entries(
    ui: &mut egui::Ui,
    entries: &[AssetEntry],
    app_state: &mut AppState,
    action: &mut Option<ResourceAction>,
) {
    for entry in entries {
        match entry {
            AssetEntry::Folder {
                name,
                path,
                children,
            } => {
                let response = egui::CollapsingHeader::new(name)
                    .id_source(("audio_folder", path))
                    .show(ui, |ui| {
                        draw_audio_asset_entries(ui, children, app_state, action)
                    });
                add_resource_context_menu(&response.header_response, path, action);
            }
            AssetEntry::File { name, path } => {
                let drag_response = ui.dnd_drag_source(
                    egui::Id::new(("audio_asset", path)),
                    AudioAssetDragPayload { path: path.clone() },
                    |ui| {
                        ui.horizontal(|ui| {
                            ui.label("♪");
                            ui.add_sized(
                                [ui.available_width(), 20.0],
                                egui::Label::new(name).truncate(),
                            );
                        });
                    },
                );
                if drag_response
                    .response
                    .dragged_by(egui::PointerButton::Primary)
                {
                    app_state.dragging_audio_asset = Some(path.clone());
                }
                add_resource_context_menu(&drag_response.response, path, action);
            }
        }
    }
}

/// 确保指定图片已经上传为egui纹理，并返回纹理句柄。
fn ensure_editor_texture(
    context: &egui::Context,
    editor_textures: &mut EditorTextures,
    image_path: &std::path::Path,
) -> Result<egui::TextureHandle, String> {
    let path_key = image_path.to_string_lossy().into_owned();
    if let Some(texture) = editor_textures.textures.get(&path_key) {
        editor_textures
            .last_used
            .insert(path_key.clone(), std::time::Instant::now());
        return Ok(texture.clone());
    }
    let image = image::open(image_path)
        .map_err(|error| format!("加载图片{}失败：{error}", image_path.display()))?;
    let color_image = image_to_egui_texture(context, image);
    let memory_bytes = color_image.size[0] as u64 * color_image.size[1] as u64 * 4;
    let texture = context.load_texture(path_key.clone(), color_image, egui::TextureOptions::LINEAR);
    editor_textures
        .textures
        .insert(path_key.clone(), texture.clone());
    editor_textures
        .last_used
        .insert(path_key.clone(), std::time::Instant::now());
    editor_textures
        .estimated_bytes
        .insert(path_key, memory_bytes);
    Ok(texture)
}

/// 绘制设置窗口内部控件，并让选项立即应用。
fn draw_settings_contents(ui: &mut egui::Ui, app_state: &mut AppState) {
    let mut restore_defaults = false;
    let context = ui.ctx().clone();
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.heading(tr("settings.interface"));
        if ui
            .checkbox(
                &mut app_state.editor_settings.dark_theme,
                tr("settings.dark"),
            )
            .changed()
        {
            if app_state.editor_settings.dark_theme {
                context.set_visuals(egui::Visuals::dark());
            } else {
                context.set_visuals(egui::Visuals::light());
            }
        }

        ui.add_space(10.0);
        ui.separator();
        ui.heading(tr("settings.canvas"));
        ui.checkbox(
            &mut app_state.editor_settings.show_grid,
            tr("view.show_grid"),
        );
        ui.horizontal(|ui| {
            ui.label(tr("settings.grid_spacing"));
            ui.add(
                egui::DragValue::new(&mut app_state.grid_size)
                    .speed(1.0)
                    .range(8.0..=200.0),
            );
        });
        ui.horizontal(|ui| {
            ui.label(tr("settings.background"));
            ui.color_edit_button_srgb(&mut app_state.editor_settings.canvas_background);
        });

        ui.add_space(10.0);
        ui.separator();
        ui.heading(tr("settings.blueprint"));
        let ide_mode_changed = ui
            .radio_value(
                &mut app_state.editor_settings.blueprint_editor_mode,
                BlueprintEditorMode::IdeTabs,
                tr("settings.ide_tabs"),
            )
            .changed();
        let separate_mode_changed = ui
            .radio_value(
                &mut app_state.editor_settings.blueprint_editor_mode,
                BlueprintEditorMode::SeparateWindow,
                tr("settings.separate_window"),
            )
            .changed();
        if ide_mode_changed && app_state.blueprint_owner_is_open() {
            app_state.blueprint_tab_active = true;
        }
        if separate_mode_changed {
            app_state.blueprint_tab_active = false;
        }
        ui.label(tr("settings.ide_help"));

        ui.add_space(10.0);
        ui.separator();
        ui.heading(tr("settings.image_import"));
        ui.horizontal(|ui| {
            ui.label(tr("settings.max_image_size"));
            ui.add(
                egui::DragValue::new(&mut app_state.editor_settings.maximum_imported_image_size)
                    .speed(10.0)
                    .range(64.0..=4096.0),
            );
        });
        ui.label(tr("settings.image_resize_help"));

        ui.add_space(16.0);
        if ui.button(tr("settings.restore")).clicked() {
            restore_defaults = true;
        }
    });

    if restore_defaults {
        app_state.grid_size = 24.0;
        app_state.editor_settings = crate::app_state::EditorSettings::new();
        context.set_visuals(egui::Visuals::dark());
    }
}

/// 绘制独立精灵动画编辑器的全部内容。
fn draw_animation_editor(
    ui: &mut egui::Ui,
    app_state: &mut AppState,
    textures: &mut HashMap<String, egui::TextureHandle>,
) {
    draw_auxiliary_editor_watermark(ui, "slide2d_animation_watermark");
    let project_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    ui.horizontal(|ui| {
        ui.label(tr("animation.name"));
        ui.text_edit_singleline(&mut app_state.animation_editor.animation.name);
        if ui.button(tr("animation.import_frames")).clicked() {
            if let Some(paths) = rfd::FileDialog::new()
                .set_title(tr("dialog.import_frames"))
                .add_filter("PNG序列帧", &["png"])
                .pick_files()
            {
                for source_path in paths {
                    match copy_animation_frame_to_assets(&project_root, &source_path) {
                        Ok(frame_path) => app_state
                            .animation_editor
                            .animation
                            .frames
                            .push(relative_to_project(&project_root, &frame_path)),
                        Err(error) => app_state.status_message = error,
                    }
                }
                app_state.animation_editor.preview_started = std::time::Instant::now();
            }
        }
        if ui.button(tr("animation.save")).clicked() {
            match save_animation_editor_asset(&project_root, &mut app_state.animation_editor) {
                Ok(path) => {
                    app_state.status_message = format!("动画已保存：{}", path.display());
                    app_state.asset_refresh_requested = true;
                }
                Err(error) => app_state.status_message = error,
            }
        }
    });
    ui.horizontal(|ui| {
        ui.label(tr("animation.fps"));
        ui.add(
            egui::DragValue::new(&mut app_state.animation_editor.animation.frames_per_second)
                .speed(1.0)
                .range(1.0..=120.0),
        );
        ui.checkbox(
            &mut app_state.animation_editor.animation.looping,
            tr("animation.loop"),
        );
        if ui
            .button(if app_state.animation_editor.preview_playing {
                tr("animation.pause")
            } else {
                tr("animation.play")
            })
            .clicked()
        {
            app_state.animation_editor.preview_playing =
                !app_state.animation_editor.preview_playing;
            app_state.animation_editor.preview_started = std::time::Instant::now();
        }
    });
    ui.separator();

    ui.columns(2, |columns| {
        draw_animation_preview(&mut columns[0], app_state, textures, &project_root);
        draw_animation_frame_list(&mut columns[1], app_state, textures, &project_root);
    });
}

/// 绘制动画当前帧的实时预览。
fn draw_animation_preview(
    ui: &mut egui::Ui,
    app_state: &AppState,
    textures: &mut HashMap<String, egui::TextureHandle>,
    project_root: &std::path::Path,
) {
    ui.heading(tr("animation.preview"));
    let preview_size = Vec2::new(ui.available_width(), 420.0);
    let (rect, _) = ui.allocate_exact_size(preview_size, egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 4.0, Color32::from_rgb(35, 38, 44));
    let frame_index = match app_state.animation_editor.preview_frame_index() {
        Some(index) => index,
        None => {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "请导入PNG序列帧",
                egui::FontId::proportional(20.0),
                Color32::LIGHT_GRAY,
            );
            return;
        }
    };
    let frame_path = resolve_asset_path(
        project_root,
        &app_state.animation_editor.animation.frames[frame_index],
    );
    if let Ok(texture) = ensure_context_texture(ui.ctx(), textures, &frame_path) {
        let image_rect = fit_texture_in_rect(rect.shrink(12.0), &texture);
        ui.painter().image(
            texture.id(),
            image_rect,
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
    }
}

/// 绘制可拖拽排序和删除的序列帧列表。
fn draw_animation_frame_list(
    ui: &mut egui::Ui,
    app_state: &mut AppState,
    textures: &mut HashMap<String, egui::TextureHandle>,
    project_root: &std::path::Path,
) {
    ui.heading(tr("animation.frames"));
    let mut frame_to_delete = None;
    let mut move_request = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        let frames = app_state.animation_editor.animation.frames.clone();
        for (index, frame) in frames.iter().enumerate() {
            let frame_path = resolve_asset_path(project_root, frame);
            let texture = ensure_context_texture(ui.ctx(), textures, &frame_path).ok();
            let row = ui.dnd_drag_source(
                egui::Id::new(("animation_frame", index, frame)),
                AnimationFrameDragPayload { index },
                |ui| {
                    ui.horizontal(|ui| {
                        ui.label(format!("{}", index + 1));
                        if let Some(texture) = &texture {
                            ui.add(
                                egui::Image::new(texture).fit_to_exact_size(Vec2::new(64.0, 64.0)),
                            );
                        }
                        let name = frame_path
                            .file_name()
                            .and_then(|value| value.to_str())
                            .unwrap_or("帧");
                        if ui
                            .selectable_label(
                                app_state.animation_editor.selected_frame == Some(index),
                                name,
                            )
                            .clicked()
                        {
                            app_state.animation_editor.selected_frame = Some(index);
                        }
                        if ui.button(tr("common.delete")).clicked() {
                            frame_to_delete = Some(index);
                        }
                    });
                },
            );
            if let Some(payload) = row
                .response
                .dnd_release_payload::<AnimationFrameDragPayload>()
            {
                move_request = Some((payload.index, index));
            }
        }
    });
    if let Some((from, to)) = move_request {
        if from < app_state.animation_editor.animation.frames.len()
            && to < app_state.animation_editor.animation.frames.len()
            && from != to
        {
            let frame = app_state.animation_editor.animation.frames.remove(from);
            app_state
                .animation_editor
                .animation
                .frames
                .insert(to, frame);
        }
    }
    if let Some(index) = frame_to_delete {
        if index < app_state.animation_editor.animation.frames.len() {
            app_state.animation_editor.animation.frames.remove(index);
            app_state.animation_editor.selected_frame = None;
        }
    }
}

/// 将导入的动画帧复制到assets/images/animation_frames目录。
fn copy_animation_frame_to_assets(
    project_root: &std::path::Path,
    source: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let target_directory = project_root.join("assets/images/animation_frames");
    std::fs::create_dir_all(&target_directory)
        .map_err(|error| format!("创建动画帧目录失败：{error}"))?;
    let file_name = source.file_name().ok_or("无法读取序列帧文件名")?;
    let mut destination = target_directory.join(file_name);
    let stem = source
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("frame");
    let mut number = 1;
    while destination.exists() {
        destination = target_directory.join(format!("{stem}_{number}.png"));
        number += 1;
    }
    std::fs::copy(source, &destination).map_err(|error| format!("复制动画帧失败：{error}"))?;
    Ok(destination)
}

/// 保存动画编辑器当前内容，并返回动画文件路径。
fn save_animation_editor_asset(
    project_root: &std::path::Path,
    state: &mut crate::animation::AnimationEditorState,
) -> Result<std::path::PathBuf, String> {
    let animation_directory = project_root.join("assets/animations");
    std::fs::create_dir_all(&animation_directory)
        .map_err(|error| format!("创建动画目录失败：{error}"))?;
    let path = state.animation_path.clone().unwrap_or_else(|| {
        let safe_name = state.animation.name.trim().replace(['/', '\\'], "_");
        animation_directory.join(format!(
            "{}.s2anim",
            if safe_name.is_empty() {
                "新动画"
            } else {
                &safe_name
            }
        ))
    });
    state.animation.save(&path)?;
    state.animation_path = Some(path.clone());
    Ok(path)
}

/// 为当前辅助窗口加载一张图片纹理。
fn ensure_context_texture(
    context: &egui::Context,
    textures: &mut HashMap<String, egui::TextureHandle>,
    path: &std::path::Path,
) -> Result<egui::TextureHandle, String> {
    let key = path.to_string_lossy().into_owned();
    if let Some(texture) = textures.get(&key) {
        return Ok(texture.clone());
    }
    let image = image::open(path).map_err(|error| format!("加载动画帧失败：{error}"))?;
    let color_image = image_to_egui_texture(context, image);
    let texture = context.load_texture(key.clone(), color_image, egui::TextureOptions::LINEAR);
    textures.insert(key, texture.clone());
    Ok(texture)
}

/// 在给定区域内按比例居中显示纹理。
fn fit_texture_in_rect(rect: Rect, texture: &egui::TextureHandle) -> Rect {
    let size = texture.size_vec2();
    let scale = (rect.width() / size.x).min(rect.height() / size.y);
    Rect::from_center_size(rect.center(), size * scale)
}

/// 绘制独立瓦片集和瓦片地图工具面板。
fn draw_tilemap_editor(
    ui: &mut egui::Ui,
    app_state: &mut AppState,
    textures: &mut HashMap<String, egui::TextureHandle>,
) {
    draw_auxiliary_editor_watermark(ui, "slide2d_tilemap_watermark");
    let project_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    ui.horizontal(|ui| {
        if ui.button(tr("tile.create")).clicked() {
            if let Some(image_path) = rfd::FileDialog::new()
                .set_title(tr("dialog.select_tileset"))
                .add_filter("PNG图集", &["png"])
                .pick_file()
            {
                match create_tileset_from_image(&project_root, &image_path, app_state) {
                    Ok(path) => {
                        app_state.status_message = format!("瓦片集已创建：{}", path.display());
                        app_state.asset_refresh_requested = true;
                    }
                    Err(error) => app_state.status_message = error,
                }
            }
        }
        ui.label(tr("tile.width"));
        ui.add(egui::DragValue::new(&mut app_state.tile_editor.new_tile_width).range(1..=512));
        ui.label(tr("tile.height"));
        ui.add(egui::DragValue::new(&mut app_state.tile_editor.new_tile_height).range(1..=512));
    });
    ui.horizontal(|ui| {
        ui.selectable_value(
            &mut app_state.tile_editor.tool,
            TileTool::Select,
            tr("tile.select"),
        );
        ui.selectable_value(
            &mut app_state.tile_editor.tool,
            TileTool::Brush,
            tr("tile.brush"),
        );
        ui.selectable_value(
            &mut app_state.tile_editor.tool,
            TileTool::Eraser,
            tr("tile.eraser"),
        );
        ui.selectable_value(
            &mut app_state.tile_editor.tool,
            TileTool::Fill,
            tr("tile.fill"),
        );
        ui.selectable_value(
            &mut app_state.tile_editor.tool,
            TileTool::Rectangle,
            tr("tile.rectangle"),
        );
        ui.separator();
        for layer in [
            TileLayerKind::Ground,
            TileLayerKind::Decoration,
            TileLayerKind::Collision,
        ] {
            ui.selectable_value(
                &mut app_state.tile_editor.active_layer,
                layer,
                layer.display_name(),
            );
        }
    });
    ui.horizontal(|ui| {
        ui.label(tr("tile.map_width"));
        ui.add(egui::DragValue::new(&mut app_state.tile_map.map_width).range(1..=2000));
        ui.label(tr("tile.map_height"));
        ui.add(egui::DragValue::new(&mut app_state.tile_map.map_height).range(1..=2000));
        if ui.button(tr("tile.fill_all")).clicked() {
            fill_entire_tile_layer(app_state);
        }
    });
    ui.horizontal(|ui| {
        ui.label(tr("tile.layers_visible"));
        for layer in &mut app_state.tile_map.layers {
            ui.checkbox(&mut layer.visible, layer.kind.display_name());
        }
    });
    ui.separator();

    let tileset = match &mut app_state.tile_editor.selected_tileset {
        Some(tileset) => tileset,
        None => {
            ui.centered_and_justified(|ui| ui.label(tr("tile.empty")));
            return;
        }
    };
    let image_path = resolve_asset_path(&project_root, &tileset.image_path);
    let texture = match ensure_context_texture(ui.ctx(), textures, &image_path) {
        Ok(texture) => texture,
        Err(error) => {
            ui.label(error);
            return;
        }
    };
    let selected_id = app_state.tile_editor.selected_tile_id;
    let mut save_tileset = false;
    if let Some(property) = tileset
        .properties
        .iter_mut()
        .find(|property| property.tile_id == selected_id)
    {
        ui.horizontal(|ui| {
            ui.label(tr_args(
                "tile.current",
                &[("value", selected_id.to_string())],
            ));
            ui.checkbox(&mut property.collision, tr("tile.trigger_collision"));
            ui.checkbox(&mut property.transparent, tr("tile.transparent"));
            if ui.button(tr("tile.save_properties")).clicked() {
                save_tileset = true;
            }
        });
    }
    // 属性变化立即进入场景内嵌副本，导出scenes.json时不会因忘记保存资源而丢失。
    app_state.tile_map.tileset = Some(tileset.clone());
    if save_tileset {
        if let Some(path) = &app_state.tile_editor.selected_tileset_path {
            if let Err(error) = tileset.save(path) {
                app_state.status_message = error;
            }
        }
    }

    egui::ScrollArea::both().show(ui, |ui| {
        let display_tile_size = 64.0;
        for row in 0..tileset.rows {
            ui.horizontal(|ui| {
                for column in 0..tileset.columns {
                    let tile_id = row * tileset.columns + column;
                    let uv = tile_uv_rect(tileset, tile_id);
                    let image = egui::Image::new(&texture)
                        .uv(uv)
                        .fit_to_exact_size(Vec2::splat(display_tile_size));
                    let response =
                        ui.add(egui::ImageButton::new(image).selected(tile_id == selected_id));
                    if response.clicked() {
                        app_state.tile_editor.selected_tile_id = tile_id;
                    }
                }
            });
        }
    });
}

/// 在动画和瓦片等独立编辑器画布右下角绘制Slide2D淡水印。
fn draw_auxiliary_editor_watermark(ui: &egui::Ui, id: &str) {
    let painter = ui
        .ctx()
        .layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new(id)));
    painter.text(
        ui.max_rect().right_bottom() - Vec2::new(12.0, 10.0),
        egui::Align2::RIGHT_BOTTOM,
        "Made by Slide2D",
        egui::FontId::proportional(13.0),
        Color32::from_white_alpha(70),
    );
}

/// 递归绘制瓦片集资源，双击后载入独立瓦片地图面板。
fn draw_tileset_asset_entries(
    ui: &mut egui::Ui,
    entries: &[AssetEntry],
    app_state: &mut AppState,
    action: &mut Option<ResourceAction>,
) {
    for entry in entries {
        match entry {
            AssetEntry::Folder {
                name,
                path,
                children,
            } => {
                let folder = egui::CollapsingHeader::new(name)
                    .id_source(("tileset_folder", path))
                    .show(ui, |ui| {
                        draw_tileset_asset_entries(ui, children, app_state, action)
                    });
                add_resource_context_menu(&folder.header_response, path, action);
            }
            AssetEntry::File { name, path } => {
                let response = ui.selectable_label(false, format!("瓦片集：{name}"));
                if response.double_clicked() {
                    match TileSet::load(path) {
                        Ok(tileset) => {
                            app_state.tile_map.tileset_path = project_relative_asset_path(path);
                            app_state.tile_map.tile_width = tileset.tile_width;
                            app_state.tile_map.tile_height = tileset.tile_height;
                            app_state.tile_editor.selected_tileset = Some(tileset);
                            app_state.tile_map.tileset =
                                app_state.tile_editor.selected_tileset.clone();
                            app_state.tile_editor.selected_tileset_path = Some(path.clone());
                            app_state.tile_editor.window_open = true;
                        }
                        Err(error) => app_state.status_message = error,
                    }
                }
                add_resource_context_menu(&response, path, action);
            }
        }
    }
}

/// 复制图集图片并根据统一格子尺寸生成瓦片集JSON。
fn create_tileset_from_image(
    project_root: &std::path::Path,
    source_path: &std::path::Path,
    app_state: &mut AppState,
) -> Result<std::path::PathBuf, String> {
    let image = image::open(source_path).map_err(|error| format!("读取图集失败：{error}"))?;
    let image_directory = project_root.join("assets/images/tilesets");
    let tileset_directory = project_root.join("assets/tilesets");
    std::fs::create_dir_all(&image_directory).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(&tileset_directory).map_err(|error| error.to_string())?;
    let file_name = source_path.file_name().ok_or("无法读取图集文件名")?;
    let target_image = image_directory.join(file_name);
    if source_path != target_image {
        std::fs::copy(source_path, &target_image)
            .map_err(|error| format!("复制图集失败：{error}"))?;
    }
    let name = source_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("tileset")
        .to_owned();
    let tileset = TileSet::new(
        name.clone(),
        relative_to_project(project_root, &target_image),
        image.width(),
        image.height(),
        app_state.tile_editor.new_tile_width,
        app_state.tile_editor.new_tile_height,
    );
    let tileset_path = tileset_directory.join(format!("{name}.s2tileset"));
    tileset.save(&tileset_path)?;
    app_state.tile_map.tileset_path = relative_to_project(project_root, &tileset_path);
    app_state.tile_map.tile_width = tileset.tile_width;
    app_state.tile_map.tile_height = tileset.tile_height;
    app_state.tile_map.tileset = Some(tileset.clone());
    app_state.tile_editor.selected_tileset = Some(tileset);
    app_state.tile_editor.selected_tileset_path = Some(tileset_path.clone());
    Ok(tileset_path)
}

/// 计算瓦片在图集纹理中的归一化UV矩形。
fn tile_uv_rect(tileset: &TileSet, tile_id: u32) -> Rect {
    let column = tile_id % tileset.columns.max(1);
    let row = tile_id / tileset.columns.max(1);
    let u0 = column as f32 / tileset.columns.max(1) as f32;
    let v0 = row as f32 / tileset.rows.max(1) as f32;
    let u1 = (column + 1) as f32 / tileset.columns.max(1) as f32;
    let v1 = (row + 1) as f32 / tileset.rows.max(1) as f32;
    Rect::from_min_max(Pos2::new(u0, v0), Pos2::new(u1, v1))
}

/// 将图片转换为egui纹理数据；图片超过GPU限制时只缩小内存预览副本。
///
/// 磁盘上的原始PNG不会被修改，Runtime仍然可以读取原始分辨率图片。
fn image_to_egui_texture(context: &egui::Context, image: image::DynamicImage) -> egui::ColorImage {
    let maximum_side = context.input(|input| input.max_texture_side).max(1) as u32;
    let resized_image = if image.width() > maximum_side || image.height() > maximum_side {
        image.resize(
            maximum_side,
            maximum_side,
            image::imageops::FilterType::Triangle,
        )
    } else {
        image
    };
    let rgba_image = resized_image.to_rgba8();
    egui::ColorImage::from_rgba_unmultiplied(
        [rgba_image.width() as usize, rgba_image.height() as usize],
        rgba_image.as_raw(),
    )
}

/// 根据设置为当前egui Context应用统一的深色或浅色主题。
fn apply_editor_theme(context: &egui::Context, dark_theme: bool) {
    if dark_theme {
        context.set_visuals(egui::Visuals::dark());
    } else {
        context.set_visuals(egui::Visuals::light());
    }
}

/// 绘制选中物体的属性，并允许用户直接修改位置和大小。
fn draw_property_panel(ui: &mut egui::Ui, app_state: &mut AppState) {
    if app_state.selected_ui_id.is_some() {
        draw_ui_element_properties(ui, app_state);
        return;
    }
    let selected_object = match app_state.selected_object_mut() {
        Some(game_object) => game_object,
        None => {
            ui.label(tr("property.empty"));
            return;
        }
    };

    ui.label(tr_args(
        "property.object_id",
        &[("value", selected_object.id.to_string())],
    ));
    ui.label(tr_args(
        "property.layer",
        &[("value", selected_object.layer_index.to_string())],
    ));
    ui.add_space(8.0);

    ui.horizontal(|ui| {
        ui.label("X：");
        ui.add(egui::DragValue::new(&mut selected_object.x).speed(1.0));
    });
    ui.horizontal(|ui| {
        ui.label("Y：");
        ui.add(egui::DragValue::new(&mut selected_object.y).speed(1.0));
    });
    ui.horizontal(|ui| {
        ui.label(tr("property.width"));
        ui.add(
            egui::DragValue::new(&mut selected_object.width)
                .speed(1.0)
                .range(MIN_OBJECT_SIZE..=10000.0),
        );
    });
    ui.horizontal(|ui| {
        ui.label(tr("property.height"));
        ui.add(
            egui::DragValue::new(&mut selected_object.height)
                .speed(1.0)
                .range(MIN_OBJECT_SIZE..=10000.0),
        );
    });

    ui.add_space(12.0);
    ui.separator();
    ui.label(tr("property.animation"));
    let animation_files = collect_animation_files(
        &std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join("assets/animations"),
    );
    let current_animation_name = std::path::Path::new(&selected_object.animation_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("未绑定");
    egui::ComboBox::from_id_source(("object_animation", selected_object.id))
        .selected_text(current_animation_name)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut selected_object.animation_path, String::new(), "未绑定");
            for animation_path in &animation_files {
                let relative = project_relative_asset_path(animation_path);
                let name = animation_path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .unwrap_or("动画");
                ui.selectable_value(&mut selected_object.animation_path, relative, name);
            }
        });

    ui.add_space(12.0);
    ui.separator();
    ui.label(tr("property.collider"));

    let mut collider_enabled = selected_object.collider.is_some();
    if ui
        .checkbox(&mut collider_enabled, "启用矩形碰撞体")
        .changed()
    {
        if collider_enabled {
            selected_object.collider = Some(ColliderConfig { is_dynamic: false });
        } else {
            selected_object.collider = None;
        }
    }

    if let Some(collider) = &mut selected_object.collider {
        ui.checkbox(&mut collider.is_dynamic, tr("property.dynamic_body"));
    }
}

/// 绘制选中UI元素的位置、尺寸和类型专属参数。
fn draw_ui_element_properties(ui: &mut egui::Ui, app_state: &mut AppState) {
    let element = match app_state.selected_ui_mut() {
        Some(element) => element,
        None => return,
    };
    ui.label(format!("UI ID：{}", element.id));
    ui.checkbox(&mut element.visible, tr("property.ui_visible"));
    ui.horizontal(|ui| {
        ui.label("X：");
        ui.add(egui::DragValue::new(&mut element.x));
        ui.label("Y：");
        ui.add(egui::DragValue::new(&mut element.y));
    });
    ui.horizontal(|ui| {
        ui.label(tr("property.short_width"));
        ui.add(egui::DragValue::new(&mut element.width).range(1.0..=4000.0));
        ui.label(tr("property.short_height"));
        ui.add(egui::DragValue::new(&mut element.height).range(1.0..=4000.0));
    });
    ui.separator();
    match &mut element.kind {
        UiElementKind::Text {
            content,
            font_size,
            color,
        } => {
            ui.label(tr("property.text_content"));
            ui.text_edit_multiline(content);
            ui.add(egui::Slider::new(font_size, 8.0..=128.0).text(tr("property.font_size")));
            ui.label(tr("property.text_color"));
            ui.color_edit_button_srgba_unmultiplied(color);
        }
        UiElementKind::Button { text } => {
            ui.label(tr("property.button_text"));
            ui.text_edit_singleline(text);
            ui.label(tr("property.button_blueprint_help"));
        }
        UiElementKind::ProgressBar {
            maximum,
            value,
            background_color,
            fill_color,
        } => {
            ui.add(
                egui::DragValue::new(maximum)
                    .speed(1.0)
                    .prefix(tr("property.maximum")),
            );
            ui.add(
                egui::DragValue::new(value)
                    .speed(1.0)
                    .prefix(tr("property.current_value")),
            );
            *value = value.clamp(0.0, maximum.max(0.0));
            ui.label(tr("property.background"));
            ui.color_edit_button_srgba_unmultiplied(background_color);
            ui.label(tr("property.fill_color"));
            ui.color_edit_button_srgba_unmultiplied(fill_color);
        }
        UiElementKind::ImagePanel { image_path } => {
            ui.label(tr("property.image_path"));
            ui.text_edit_singleline(image_path);
            if ui.button(tr("property.choose_png")).clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title(tr("dialog.select_ui_image"))
                    .add_filter("PNG图片", &["png"])
                    .pick_file()
                {
                    *image_path = project_relative_asset_path(&path);
                }
            }
        }
    }
}

/// 选择导出位置，并将带SLIDE2D_SCENE标识的当前场景写为JSON。
fn export_scene(app_state: &AppState) -> Result<std::path::PathBuf, String> {
    let scene = app_state.create_scene();
    let scene_path = rfd::FileDialog::new()
        .set_title(tr("dialog.export_scene"))
        .add_filter("Slide2D场景JSON", &["json"])
        .set_file_name(format!("{}.slide2d.scene.json", scene.name))
        .save_file()
        .ok_or_else(|| "已取消导出当前场景".to_owned())?;
    let scene_directory = scene_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    for game_object in &scene.game_objects {
        let blueprint_file_name = if game_object.blueprint_file.is_empty() {
            format!("blueprint_{}.json", game_object.id)
        } else {
            game_object.blueprint_file.clone()
        };
        let blueprint_path = scene_directory.join(blueprint_file_name);
        let blueprint_json = serde_json::to_string_pretty(&game_object.blueprint)
            .map_err(|error| format!("保存物体{}蓝图失败：{error}", game_object.id))?;
        fs::write(blueprint_path, blueprint_json)
            .map_err(|error| format!("写入物体{}蓝图文件失败：{error}", game_object.id))?;
    }
    let json_text = serde_json::to_string_pretty(&scene)
        .map_err(|error| format!("生成场景JSON失败：{error}"))?;
    fs::write(&scene_path, json_text).map_err(|error| format!("导出场景失败：{error}"))?;
    Ok(scene_path)
}

/// 选择目标目录，导出Content和旧版assets，并写入Slide2D素材清单。
fn export_all_assets(app_state: &AppState) -> Result<std::path::PathBuf, String> {
    let destination = rfd::FileDialog::new()
        .set_title(tr("dialog.export_assets"))
        .pick_folder()
        .ok_or_else(|| "已取消导出全部素材".to_owned())?;
    let output = destination.join("Slide2D_Exported_Assets");
    fs::create_dir_all(&output).map_err(|error| format!("创建素材导出目录失败：{error}"))?;
    copy_export_directory(
        &app_state.project_root.join("Content"),
        &output.join("Content"),
    )?;
    copy_export_directory(
        &app_state.project_root.join("assets"),
        &output.join("assets"),
    )?;
    let manifest = serde_json::json!({
        "slide2d_engine": "SLIDE2D_ASSET_EXPORT",
        "format_version": 1,
        "source_project": app_state.project_root.to_string_lossy(),
        "notice": "Slide2D 2D零代码游戏引擎"
    });
    let bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("生成Slide2D素材清单失败：{error}"))?;
    fs::write(output.join("slide2d.assets.json"), bytes)
        .map_err(|error| format!("写入Slide2D素材清单失败：{error}"))?;
    Ok(output)
}

/// 递归复制素材目录，并保留用户创建的空文件夹层级。
fn copy_export_directory(
    source: &std::path::Path,
    destination: &std::path::Path,
) -> Result<(), String> {
    if !source.exists() {
        return Ok(());
    }
    fs::create_dir_all(destination).map_err(|error| format!("创建导出目录失败：{error}"))?;
    for entry in fs::read_dir(source).map_err(|error| format!("读取素材目录失败：{error}"))?
    {
        let entry = entry.map_err(|error| format!("读取素材条目失败：{error}"))?;
        let target = destination.join(entry.file_name());
        if entry.path().is_dir() {
            copy_export_directory(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target).map_err(|error| format!("导出素材失败：{error}"))?;
        }
    }
    Ok(())
}

/// 先导出当前场景，再启动一个独立进程运行游戏内核。
fn launch_runtime(app_state: &mut AppState) -> Result<(), String> {
    let runtime_path = if app_state.project_file_path.is_some() {
        let project_folder = app_state.project_root.clone();
        save_project_folder(app_state, &project_folder)?;
        let runtime_package = project_folder.join(".slide2d_runtime.slide2d");
        let project_identity = app_state.project_file_path.clone();
        save_project(app_state, &runtime_package)?;
        app_state.project_file_path = project_identity;
        runtime_package
    } else {
        export_scene(app_state)?
    };
    let executable_path =
        std::env::current_exe().map_err(|error| format!("查找当前程序路径失败：{error}"))?;

    Command::new(executable_path)
        .arg("--runtime")
        .arg(runtime_path)
        .arg("--locale")
        .arg(current_language().code())
        .spawn()
        .map_err(|error| format!("启动游戏运行时失败：{error}"))?;
    Ok(())
}

/// 绘制画布，并处理选择、拖动、缩放、视图平移和视图缩放。
fn draw_canvas(
    ui: &mut egui::Ui,
    canvas_rect: Rect,
    app_state: &mut AppState,
    editor_textures: &mut EditorTextures,
) {
    let image_paths: Vec<std::path::PathBuf> = app_state
        .game_objects
        .iter()
        .filter(|object| {
            !app_state.performance_settings.viewport_culling
                || object_screen_rect(object, canvas_rect, app_state).intersects(canvas_rect)
        })
        .filter(|object| !object.image_path.is_empty())
        .map(|object| std::path::PathBuf::from(&object.image_path))
        .collect();
    for image_path in image_paths {
        let _ = ensure_editor_texture(ui.ctx(), editor_textures, &image_path);
    }
    let ui_image_paths: Vec<std::path::PathBuf> = app_state
        .ui_elements
        .iter()
        .filter(|element| {
            !app_state.performance_settings.viewport_culling
                || Rect::from_min_size(
                    canvas_rect.min + Vec2::new(element.x, element.y),
                    Vec2::new(element.width, element.height),
                )
                .intersects(canvas_rect)
        })
        .filter_map(|element| match &element.kind {
            UiElementKind::ImagePanel { image_path } if !image_path.is_empty() => {
                Some(std::path::PathBuf::from(image_path))
            }
            _ => None,
        })
        .collect();
    for image_path in ui_image_paths {
        let _ = ensure_editor_texture(ui.ctx(), editor_textures, &image_path);
    }
    let canvas_response = ui.allocate_rect(canvas_rect, egui::Sense::click_and_drag());
    app_state.last_canvas_width = canvas_rect.width().max(1.0) as u32;
    app_state.last_canvas_height = canvas_rect.height().max(1.0) as u32;
    let painter = ui.painter_at(canvas_rect);
    let background = app_state.editor_settings.canvas_background;
    painter.rect_filled(
        canvas_rect,
        0.0,
        Color32::from_rgb(background[0], background[1], background[2]),
    );
    draw_editor_canvas_watermark(&painter, canvas_rect);

    process_image_drops(ui.ctx(), canvas_rect, app_state, editor_textures);
    let released_inside =
        canvas_response.contains_pointer() && ui.input(|input| input.pointer.any_released());

    let released_actor = if released_inside
        && egui::DragAndDrop::has_payload_of_type::<ActorAssetDragPayload>(ui.ctx())
    {
        egui::DragAndDrop::take_payload::<ActorAssetDragPayload>(ui.ctx())
    } else {
        None
    };
    let actor_path = released_actor
        .map(|payload| payload.path.clone())
        .or_else(|| {
            if released_inside {
                app_state.dragging_actor_asset.clone()
            } else {
                None
            }
        });
    if let Some(path) = actor_path {
        let position = canvas_response
            .interact_pointer_pos()
            .unwrap_or(canvas_rect.center());
        app_state.status_message =
            match instantiate_actor_asset(&path, position, canvas_rect, app_state) {
                Ok(id) => format!("Actor资源已放置，物体ID：{id}"),
                Err(error) => error,
            };
    }

    let released_animation = if released_inside
        && egui::DragAndDrop::has_payload_of_type::<AnimationAssetDragPayload>(ui.ctx())
    {
        egui::DragAndDrop::take_payload::<AnimationAssetDragPayload>(ui.ctx())
    } else {
        None
    };
    let animation_path = released_animation
        .map(|payload| payload.path.clone())
        .or_else(|| {
            if released_inside {
                app_state.dragging_animation_asset.clone()
            } else {
                None
            }
        });
    if let Some(path) = animation_path {
        let position = canvas_response
            .interact_pointer_pos()
            .unwrap_or(canvas_rect.center());
        app_state.status_message = match apply_animation_asset_drop(
            ui.ctx(),
            &path,
            position,
            canvas_rect,
            app_state,
            editor_textures,
        ) {
            Ok(id) => format!("动画已绑定到Actor {id}"),
            Err(error) => error,
        };
    }

    // egui的take_payload会先清空全局payload，再尝试类型转换。
    // 必须先检查类型，否则图片分支会错误吞掉UI或音效payload。
    let released_asset = if released_inside
        && egui::DragAndDrop::has_payload_of_type::<ImageAssetDragPayload>(ui.ctx())
    {
        egui::DragAndDrop::take_payload::<ImageAssetDragPayload>(ui.ctx())
    } else {
        None
    };
    let manually_dragged_path = app_state.dragging_image_asset.clone();
    let released_path = released_asset
        .map(|payload| payload.path.clone())
        .or_else(|| {
            if released_inside {
                manually_dragged_path
            } else {
                None
            }
        });
    if let Some(image_path) = released_path {
        let drop_position = canvas_response
            .interact_pointer_pos()
            .unwrap_or(canvas_rect.center());
        if let Err(error) = create_image_object_from_asset(
            ui.ctx(),
            &image_path,
            drop_position,
            canvas_rect,
            app_state,
            editor_textures,
        ) {
            app_state.status_message = error;
        }
    }
    let released_audio_payload = if released_inside
        && egui::DragAndDrop::has_payload_of_type::<AudioAssetDragPayload>(ui.ctx())
    {
        egui::DragAndDrop::take_payload::<AudioAssetDragPayload>(ui.ctx())
    } else {
        None
    };
    let released_audio_path = released_audio_payload
        .map(|payload| payload.path.clone())
        .or_else(|| {
            if released_inside {
                app_state.dragging_audio_asset.clone()
            } else {
                None
            }
        });
    if let Some(audio_path) = released_audio_path {
        let drop_position = canvas_response
            .interact_pointer_pos()
            .unwrap_or(canvas_rect.center());
        let scene_position = screen_to_scene(drop_position, canvas_rect, app_state);
        app_state.add_audio_object(
            project_relative_asset_path(&audio_path),
            scene_position.x - 24.0,
            scene_position.y - 24.0,
        );
        app_state.status_message = format!("已创建音效对象：{}", audio_path.display());
    }
    let released_ui_payload =
        if released_inside && egui::DragAndDrop::has_payload_of_type::<UiDragPayload>(ui.ctx()) {
            egui::DragAndDrop::take_payload::<UiDragPayload>(ui.ctx())
        } else {
            None
        };
    let released_ui_template = released_ui_payload
        .map(|payload| payload.template)
        .or_else(|| {
            if released_inside {
                app_state.dragging_ui_template
            } else {
                None
            }
        });
    if let Some(template) = released_ui_template {
        let pointer = canvas_response
            .interact_pointer_pos()
            .unwrap_or(canvas_rect.center());
        app_state.add_ui_element(
            template,
            pointer.x - canvas_rect.left(),
            pointer.y - canvas_rect.top(),
        );
        app_state.status_message = "UI组件已添加到场景画布".to_owned();
    }
    // 每次释放都结束当前资源拖动，下一次拖同一张图时会重新建立状态。
    if ui.input(|input| input.pointer.any_released()) {
        app_state.dragging_image_asset = None;
        app_state.dragging_audio_asset = None;
        app_state.dragging_actor_asset = None;
        app_state.dragging_animation_asset = None;
        app_state.dragging_ui_template = None;
    }

    if app_state.is_file_hovering {
        painter.rect_stroke(
            canvas_rect.shrink(4.0),
            4.0,
            Stroke::new(3.0_f32, Color32::from_rgb(45, 125, 230)),
        );
        painter.text(
            canvas_rect.center(),
            egui::Align2::CENTER_CENTER,
            "松开鼠标，自动识别并导入素材",
            egui::FontId::proportional(24.0),
            Color32::from_rgb(25, 85, 170),
        );
    }

    handle_view_navigation(ui, canvas_rect, app_state);
    draw_grid(&painter, canvas_rect, app_state);
    draw_canvas_rulers(ui, &painter, canvas_rect, app_state);
    ensure_selected_tileset(app_state);
    app_state.performance_metrics.rendered_objects = 0;
    app_state.performance_metrics.rendered_tiles = 0;
    draw_tile_map(ui.ctx(), &painter, canvas_rect, app_state, editor_textures);
    let tile_tool_active = app_state.tile_editor.tool != TileTool::Select;
    if tile_tool_active {
        handle_tilemap_interaction(ui, canvas_rect, app_state);
    }

    // 使用画布控件自己的Response识别双击，比直接组合鼠标按下和双击状态更可靠。
    // 双击要在普通拖拽之前处理，避免第二次点击再次启动物体拖动。
    if !tile_tool_active {
        let ui_consumed = handle_ui_interaction(ui, canvas_rect, app_state);
        if !ui_consumed {
            let opened_blueprint =
                handle_object_double_click(ui, &canvas_response, canvas_rect, app_state);
            if !opened_blueprint {
                handle_object_interaction(ui, canvas_rect, app_state);
            }
        }
    }
    draw_game_objects(&painter, canvas_rect, app_state, editor_textures);
    draw_editor_ui_elements(ui.ctx(), canvas_rect, app_state, editor_textures);
    if canvas_response.hovered() && ui.input(|input| input.key_pressed(egui::Key::F)) {
        center_selected_objects_in_canvas(app_state);
    }

    if canvas_response.hovered() && ui.input(|input| input.pointer.secondary_down()) {
        ui.output_mut(|output| output.cursor_icon = CursorIcon::Grabbing);
    }
}

/// 在编辑器画布右下角绘制不参与场景保存的Slide2D淡水印。
fn draw_editor_canvas_watermark(painter: &egui::Painter, canvas_rect: Rect) {
    painter.text(
        canvas_rect.right_bottom() - Vec2::new(12.0, 10.0),
        egui::Align2::RIGHT_BOTTOM,
        "Made by Slide2D",
        egui::FontId::proportional(13.0),
        Color32::from_white_alpha(70),
    );
}

/// 从.s2actor模板实例化场景物体，并由当前场景分配新ID和图层。
fn instantiate_actor_asset(
    path: &std::path::Path,
    drop_position: Pos2,
    canvas_rect: Rect,
    app_state: &mut AppState,
) -> Result<u64, String> {
    let actor = ActorAsset::load(path)?;
    let scene = screen_to_scene(drop_position, canvas_rect, app_state);
    let id = app_state.next_object_id;
    let object = GameObject {
        id,
        x: scene.x - actor.width * 0.5,
        y: scene.y - actor.height * 0.5,
        width: actor.width,
        height: actor.height,
        layer_index: app_state.next_layer_index,
        image_path: actor.image_path,
        audio_path: actor.audio_path,
        animation_path: actor.animation_path,
        animation_playing: actor.animation_playing,
        collider: actor.collider,
        blueprint: actor.blueprint,
        blueprint_file: format!("blueprint_{id}.json"),
        variables: actor.variables,
    };
    app_state.game_objects.push(object);
    app_state.selected_object_id = Some(id);
    app_state.selected_ui_id = None;
    app_state.next_object_id += 1;
    app_state.next_layer_index += 1;
    Ok(id)
}

/// 将动画拖到已有Actor时直接绑定，拖到空白处时根据首帧创建贴图Actor。
fn apply_animation_asset_drop(
    context: &egui::Context,
    animation_path: &std::path::Path,
    drop_position: Pos2,
    canvas_rect: Rect,
    app_state: &mut AppState,
    editor_textures: &mut EditorTextures,
) -> Result<u64, String> {
    let relative_animation = project_relative_asset_path(animation_path);
    if let Some(id) = find_top_object_at(drop_position, canvas_rect, app_state) {
        let object = app_state
            .game_objects
            .iter_mut()
            .find(|object| object.id == id)
            .ok_or("动画目标Actor不存在")?;
        object.animation_path = relative_animation;
        object.animation_playing = true;
        app_state.selected_object_id = Some(id);
        return Ok(id);
    }
    let animation = crate::animation::SpriteAnimation::load(animation_path)?;
    let first_frame = animation
        .frames
        .first()
        .ok_or("动画没有序列帧，无法生成Actor")?;
    let frame_path = resolve_asset_path(&app_state.project_root, first_frame);
    let id = app_state.next_object_id;
    create_image_object_from_asset(
        context,
        &frame_path,
        drop_position,
        canvas_rect,
        app_state,
        editor_textures,
    )?;
    let object = app_state
        .game_objects
        .iter_mut()
        .find(|object| object.id == id)
        .ok_or("动画Actor创建失败")?;
    object.animation_path = relative_animation;
    object.animation_playing = true;
    Ok(id)
}

/// 处理UI元素选择和拖动，UI始终优先于下方场景物体。
fn handle_ui_interaction(ui: &egui::Ui, canvas_rect: Rect, app_state: &mut AppState) -> bool {
    let pointer = match ui.input(|input| input.pointer.interact_pos()) {
        Some(pointer) if canvas_rect.contains(pointer) => pointer,
        _ => return false,
    };
    let local = pointer - canvas_rect.min;
    let mut hit = None;
    let mut top_layer = 0;
    for element in &app_state.ui_elements {
        if !element.visible {
            continue;
        }
        let rect = Rect::from_min_size(
            Pos2::new(element.x, element.y),
            Vec2::new(element.width, element.height),
        );
        if rect.contains(Pos2::new(local.x, local.y))
            && (hit.is_none() || element.layer_index >= top_layer)
        {
            hit = Some(element.id);
            top_layer = element.layer_index;
        }
    }
    let primary_pressed = ui.input(|input| input.pointer.button_pressed(PointerButton::Primary));
    let double_clicked =
        ui.input(|input| input.pointer.button_double_clicked(PointerButton::Primary));
    if double_clicked {
        if let Some(id) = hit {
            app_state.selected_ui_id = Some(id);
            app_state.selected_object_id = None;
            app_state.blueprint_ui_id = Some(id);
            app_state.blueprint_object_id = None;
            app_state.blueprint_tab_active =
                app_state.editor_settings.blueprint_editor_mode == BlueprintEditorMode::IdeTabs;
            app_state.pending_blueprint_output = None;
            app_state.selected_blueprint_node_id = None;
            return true;
        }
    }
    if primary_pressed {
        if let Some(id) = hit {
            app_state.selected_ui_id = Some(id);
            app_state.selected_object_id = None;
        } else {
            app_state.selected_ui_id = None;
            return false;
        }
    }
    if let Some(id) = app_state.selected_ui_id {
        if ui.input(|input| input.pointer.primary_down()) && hit == Some(id) {
            let delta = ui.input(|input| input.pointer.delta());
            if let Some(element) = app_state.ui_elements.iter_mut().find(|item| item.id == id) {
                element.x += delta.x;
                element.y += delta.y;
            }
            return true;
        }
    }
    hit.is_some()
}

/// 在世界和瓦片上方绘制编辑器HUD预览。
fn draw_editor_ui_elements(
    context: &egui::Context,
    canvas_rect: Rect,
    app_state: &mut AppState,
    editor_textures: &EditorTextures,
) {
    // UI预览使用独立前景图层，确保不会被画布背景、网格、瓦片或场景物体覆盖。
    // 裁剪区域仍限制在中央画布内，避免UI绘制到左右属性面板上。
    let mut painter = context.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("game_ui_preview_layer"),
    ));
    painter.set_clip_rect(canvas_rect);
    let mut elements: Vec<_> = app_state.ui_elements.iter().collect();
    elements.sort_by_key(|element| element.layer_index);
    let mut rendered_count = 0;
    for element in elements {
        if !element.visible {
            continue;
        }
        let rect = Rect::from_min_size(
            canvas_rect.min + Vec2::new(element.x, element.y),
            Vec2::new(element.width, element.height),
        );
        if app_state.performance_settings.viewport_culling && !rect.intersects(canvas_rect) {
            continue;
        }
        rendered_count += 1;
        match &element.kind {
            UiElementKind::Text {
                content,
                font_size,
                color,
            } => {
                painter.text(
                    rect.left_center(),
                    egui::Align2::LEFT_CENTER,
                    content,
                    egui::FontId::proportional(*font_size),
                    Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3]),
                );
            }
            UiElementKind::Button { text } => {
                painter.rect_filled(rect, 5.0, Color32::from_rgb(70, 95, 145));
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    text,
                    egui::FontId::proportional(18.0),
                    Color32::WHITE,
                );
            }
            UiElementKind::ProgressBar {
                maximum,
                value,
                background_color,
                fill_color,
            } => {
                painter.rect_filled(rect, 3.0, color32(*background_color));
                let ratio = if *maximum <= 0.0 {
                    0.0
                } else {
                    (*value / *maximum).clamp(0.0, 1.0)
                };
                let fill =
                    Rect::from_min_size(rect.min, Vec2::new(rect.width() * ratio, rect.height()));
                painter.rect_filled(fill, 3.0, color32(*fill_color));
            }
            UiElementKind::ImagePanel { image_path } => {
                if let Some(texture) = editor_textures.textures.get(image_path) {
                    painter.image(
                        texture.id(),
                        rect,
                        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                        Color32::WHITE,
                    );
                } else {
                    painter.rect_filled(rect, 3.0, Color32::from_gray(80));
                    painter.text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "图片面板",
                        egui::FontId::proportional(16.0),
                        Color32::LIGHT_GRAY,
                    );
                }
            }
        }
        if app_state.selected_ui_id == Some(element.id) {
            painter.rect_stroke(rect, 0.0, Stroke::new(2.0_f32, Color32::YELLOW));
        }
    }
    app_state.performance_metrics.rendered_objects += rendered_count;
}

/// 将RGBA数组转换为egui颜色。
fn color32(color: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(color[0], color[1], color[2], color[3])
}

/// 根据场景保存的路径恢复编辑器当前瓦片集。
fn ensure_selected_tileset(app_state: &mut AppState) {
    if app_state.tile_editor.selected_tileset.is_some() {
        return;
    }
    if let Some(tileset) = &app_state.tile_map.tileset {
        app_state.tile_editor.selected_tileset = Some(tileset.clone());
        return;
    }
    if app_state.tile_map.tileset_path.is_empty() {
        return;
    }
    let path = std::env::current_dir()
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join(&app_state.tile_map.tileset_path);
    if let Ok(tileset) = TileSet::load(&path) {
        app_state.tile_editor.selected_tileset = Some(tileset);
        app_state.tile_editor.selected_tileset_path = Some(path);
    }
}

/// 按地面、装饰、碰撞顺序绘制场景中的全部瓦片。
fn draw_tile_map(
    context: &egui::Context,
    painter: &egui::Painter,
    canvas_rect: Rect,
    app_state: &mut AppState,
    textures: &mut EditorTextures,
) {
    let tileset = match &app_state.tile_editor.selected_tileset {
        Some(tileset) => tileset,
        None => return,
    };
    let image_path = resolve_asset_path(
        &std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
        &tileset.image_path,
    );
    let texture = match ensure_editor_texture(context, textures, &image_path) {
        Ok(texture) => texture,
        Err(_) => return,
    };
    let visible_scene = editor_visible_scene_rect(canvas_rect, app_state);
    let mut rendered_tiles = 0;
    for layer in &app_state.tile_map.layers {
        if !layer.visible {
            continue;
        }
        for cell in &layer.cells {
            if app_state.performance_settings.tile_chunk_culling
                && !tile_chunk_visible(cell.x, cell.y, &visible_scene, &app_state.tile_map)
            {
                continue;
            }
            let scene_position = Pos2::new(
                cell.x as f32 * app_state.tile_map.tile_width as f32,
                cell.y as f32 * app_state.tile_map.tile_height as f32,
            );
            let screen_position = scene_to_screen(scene_position, canvas_rect, app_state);
            let screen_size = Vec2::new(
                app_state.tile_map.tile_width as f32 * app_state.view_zoom,
                app_state.tile_map.tile_height as f32 * app_state.view_zoom,
            );
            let rect = Rect::from_min_size(screen_position, screen_size);
            if app_state.performance_settings.viewport_culling && !rect.intersects(canvas_rect) {
                continue;
            }
            rendered_tiles += 1;
            let tint = if layer.kind == TileLayerKind::Collision {
                Color32::from_rgba_premultiplied(255, 80, 80, 150)
            } else if tileset
                .property(cell.tile_id)
                .map(|property| property.transparent)
                .unwrap_or(false)
            {
                Color32::from_white_alpha(130)
            } else {
                Color32::WHITE
            };
            painter.image(
                texture.id(),
                rect,
                tile_uv_rect(tileset, cell.tile_id),
                tint,
            );
            let property_collision = tileset
                .property(cell.tile_id)
                .map(|property| property.collision)
                .unwrap_or(false);
            if app_state.assistant_settings.show_tile_colliders
                && (layer.kind == TileLayerKind::Collision || property_collision)
            {
                painter.rect_stroke(
                    rect,
                    0.0,
                    Stroke::new(2.0, Color32::from_rgb(255, 70, 70)),
                );
            }
        }
    }
    app_state.performance_metrics.rendered_tiles = rendered_tiles;
}

/// 处理画笔、橡皮擦和填充工具对当前瓦片层的操作。
fn handle_tilemap_interaction(ui: &egui::Ui, canvas_rect: Rect, app_state: &mut AppState) {
    let pointer = match ui.input(|input| input.pointer.interact_pos()) {
        Some(position) if canvas_rect.contains(position) => position,
        _ => return,
    };
    let primary_down = ui.input(|input| input.pointer.primary_down());
    let primary_pressed = ui.input(|input| input.pointer.button_pressed(PointerButton::Primary));
    let primary_released = ui.input(|input| input.pointer.button_released(PointerButton::Primary));
    if !primary_down && !primary_released {
        return;
    }
    let scene = screen_to_scene(pointer, canvas_rect, app_state);
    let tile_x = (scene.x / app_state.tile_map.tile_width.max(1) as f32).floor() as i32;
    let tile_y = (scene.y / app_state.tile_map.tile_height.max(1) as f32).floor() as i32;
    let tool = app_state.tile_editor.tool;
    let tile_id = app_state.tile_editor.selected_tile_id;
    if tool == TileTool::Rectangle {
        if primary_pressed {
            app_state.tile_editor.rectangle_start = Some((tile_x, tile_y));
        }
        if primary_released {
            if let Some((start_x, start_y)) = app_state.tile_editor.rectangle_start.take() {
                paint_tile_rectangle(app_state, start_x, start_y, tile_x, tile_y, tile_id);
            }
        }
        return;
    }
    let layer = match app_state
        .tile_map
        .layer_mut(app_state.tile_editor.active_layer)
    {
        Some(layer) => layer,
        None => return,
    };
    match tool {
        TileTool::Brush => layer.set_tile(tile_x, tile_y, tile_id),
        TileTool::Eraser => layer.erase_tile(tile_x, tile_y),
        TileTool::Fill if primary_pressed => layer.fill(tile_x, tile_y, tile_id),
        _ => {}
    }
}

/// 使用当前瓦片填充矩形起点和终点之间的全部格子。
fn paint_tile_rectangle(
    app_state: &mut AppState,
    start_x: i32,
    start_y: i32,
    end_x: i32,
    end_y: i32,
    tile_id: u32,
) {
    let minimum_x = start_x.min(end_x).max(0);
    let maximum_x = start_x
        .max(end_x)
        .min(app_state.tile_map.map_width as i32 - 1);
    let minimum_y = start_y.min(end_y).max(0);
    let maximum_y = start_y
        .max(end_y)
        .min(app_state.tile_map.map_height as i32 - 1);
    let active_layer = app_state.tile_editor.active_layer;
    if let Some(layer) = app_state.tile_map.layer_mut(active_layer) {
        for y in minimum_y..=maximum_y {
            for x in minimum_x..=maximum_x {
                layer.set_tile(x, y, tile_id);
            }
        }
    }
}

/// 使用当前选中瓦片一次填满当前层的有限地图范围。
fn fill_entire_tile_layer(app_state: &mut AppState) {
    let width = app_state.tile_map.map_width.min(2000);
    let height = app_state.tile_map.map_height.min(2000);
    let tile_id = app_state.tile_editor.selected_tile_id;
    let active_layer = app_state.tile_editor.active_layer;
    if let Some(layer) = app_state.tile_map.layer_mut(active_layer) {
        layer.cells.clear();
        for y in 0..height as i32 {
            for x in 0..width as i32 {
                layer.set_tile(x, y, tile_id);
            }
        }
    }
}

/// 处理画布中的物体双击，并打开被双击物体的蓝图编辑窗口。
fn handle_object_double_click(
    ui: &egui::Ui,
    canvas_response: &egui::Response,
    canvas_rect: Rect,
    app_state: &mut AppState,
) -> bool {
    if !canvas_response.double_clicked_by(PointerButton::Primary) {
        return false;
    }

    let pointer_position = match ui.input(|input| input.pointer.hover_pos()) {
        Some(position) => position,
        None => return false,
    };
    let object_id = match find_top_object_at(pointer_position, canvas_rect, app_state) {
        Some(id) => id,
        None => return false,
    };

    app_state.selected_object_id = Some(object_id);
    app_state.selected_ui_id = None;
    app_state.blueprint_object_id = Some(object_id);
    app_state.blueprint_ui_id = None;
    app_state.blueprint_tab_active =
        app_state.editor_settings.blueprint_editor_mode == BlueprintEditorMode::IdeTabs;
    app_state.pending_blueprint_output = None;
    app_state.selected_blueprint_node_id = None;
    app_state.object_interaction = None;
    true
}

/// 处理等待中的操作系统图片拖放事件，并在拖放位置创建游戏物体。
fn process_image_drops(
    context: &egui::Context,
    canvas_rect: Rect,
    app_state: &mut AppState,
    editor_textures: &mut EditorTextures,
) {
    if app_state.pending_image_drops.is_empty() {
        return;
    }

    let pending_drops = std::mem::take(&mut app_state.pending_image_drops);
    for pending_drop in pending_drops {
        let reported_position = Pos2::new(pending_drop.screen_x, pending_drop.screen_y);
        // Windows进入OLE文件拖放后可能停止发送CursorMoved，最后坐标可能是(0,0)
        // 或停留在窗口外。此时使用画布中心，确保文件不会被无提示地丢弃。
        let drop_position = if canvas_rect.contains(reported_position) {
            reported_position
        } else {
            canvas_rect.center()
        };

        let loaded_image = match image::open(&pending_drop.path) {
            Ok(image) => image,
            Err(error) => {
                app_state.status_message = format!(
                    "图片加载失败：{}，错误：{error}",
                    pending_drop.path.display()
                );
                continue;
            }
        };
        let image_width = loaded_image.width() as usize;
        let image_height = loaded_image.height() as usize;
        if image_width == 0 || image_height == 0 {
            app_state.status_message = "图片尺寸无效，无法导入".to_owned();
            continue;
        }

        let image_path = pending_drop.path.to_string_lossy().into_owned();
        let color_image = image_to_egui_texture(context, loaded_image);
        let texture = context.load_texture(
            format!("game_object_image_{}", app_state.next_object_id),
            color_image,
            egui::TextureOptions::LINEAR,
        );
        editor_textures.textures.insert(image_path.clone(), texture);

        let scene_position = screen_to_scene(drop_position, canvas_rect, app_state);
        let original_width = image_width as f32;
        let original_height = image_height as f32;
        let maximum_size = app_state.editor_settings.maximum_imported_image_size;
        let scale = (maximum_size / original_width.max(original_height)).min(1.0);
        let display_width = original_width * scale;
        let display_height = original_height * scale;

        // 图片中心放在鼠标松开的位置，操作体验与PPT等编辑器一致。
        app_state.add_image_object(
            project_relative_asset_path(&pending_drop.path),
            scene_position.x - display_width * 0.5,
            scene_position.y - display_height * 0.5,
            display_width,
            display_height,
        );
        app_state.status_message = format!("图片已导入：{}", pending_drop.path.display());
    }
}

/// 使用资源库图片在指定画布位置创建带贴图的GameObject。
fn create_image_object_from_asset(
    context: &egui::Context,
    image_path: &std::path::Path,
    drop_position: Pos2,
    canvas_rect: Rect,
    app_state: &mut AppState,
    editor_textures: &mut EditorTextures,
) -> Result<(), String> {
    let _ = ensure_editor_texture(context, editor_textures, image_path)?;
    let image = image::open(image_path).map_err(|error| format!("读取图片尺寸失败：{error}"))?;
    let original_width = image.width() as f32;
    let original_height = image.height() as f32;
    let maximum_size = app_state.editor_settings.maximum_imported_image_size;
    let scale = (maximum_size / original_width.max(original_height)).min(1.0);
    let display_width = original_width * scale;
    let display_height = original_height * scale;
    let scene_position = screen_to_scene(drop_position, canvas_rect, app_state);
    let path_string = project_relative_asset_path(image_path);
    app_state.add_image_object(
        path_string,
        scene_position.x - display_width * 0.5,
        scene_position.y - display_height * 0.5,
        display_width,
        display_height,
    );
    app_state.status_message = format!("已从资源库创建物体：{}", image_path.display());
    Ok(())
}

/// 将项目内资源保存为相对路径，避免场景绑定当前电脑的绝对路径。
fn project_relative_asset_path(image_path: &std::path::Path) -> String {
    let project_root = match std::env::current_dir() {
        Ok(path) => path,
        Err(_) => return image_path.to_string_lossy().into_owned(),
    };
    match image_path.strip_prefix(&project_root) {
        Ok(relative_path) => relative_path.to_string_lossy().replace('\\', "/"),
        Err(_) => image_path.to_string_lossy().into_owned(),
    }
}

/// 处理鼠标右键平移，以及鼠标滚轮缩放画布视图。
fn handle_view_navigation(ui: &egui::Ui, canvas_rect: Rect, app_state: &mut AppState) {
    let pointer_position = ui.input(|input| input.pointer.hover_pos());
    let pointer_inside_canvas = match pointer_position {
        Some(position) => canvas_rect.contains(position),
        None => false,
    };

    if !pointer_inside_canvas {
        return;
    }

    let secondary_down = ui.input(|input| input.pointer.secondary_down());
    if secondary_down {
        let pointer_delta = ui.input(|input| input.pointer.delta());
        app_state.view_offset_x += pointer_delta.x;
        app_state.view_offset_y += pointer_delta.y;
    }

    let scroll_delta = ui.input(|input| input.smooth_scroll_delta.y);
    if scroll_delta.abs() < 0.01 {
        return;
    }

    let pointer_position = pointer_position.unwrap_or(canvas_rect.center());
    let scene_position_before_zoom = screen_to_scene(pointer_position, canvas_rect, app_state);
    let zoom_multiplier = (scroll_delta * 0.0015).exp();
    app_state.view_zoom =
        (app_state.view_zoom * zoom_multiplier).clamp(MIN_VIEW_ZOOM, MAX_VIEW_ZOOM);

    app_state.view_offset_x = pointer_position.x
        - canvas_rect.left()
        - scene_position_before_zoom.x * app_state.view_zoom;
    app_state.view_offset_y =
        pointer_position.y - canvas_rect.top() - scene_position_before_zoom.y * app_state.view_zoom;
}

/// 绘制会随画布平移和缩放的网格线。
fn draw_grid(painter: &egui::Painter, canvas_rect: Rect, app_state: &AppState) {
    if !app_state.editor_settings.show_grid {
        return;
    }
    let grid_spacing = app_state.grid_size * app_state.view_zoom;
    if grid_spacing < 4.0 {
        return;
    }

    let grid_color = Color32::from_rgb(195, 195, 195);
    let grid_stroke = Stroke::new(1.0_f32, grid_color);

    let mut x = canvas_rect.left() + app_state.view_offset_x.rem_euclid(grid_spacing);
    while x <= canvas_rect.right() {
        painter.line_segment(
            [
                Pos2::new(x, canvas_rect.top()),
                Pos2::new(x, canvas_rect.bottom()),
            ],
            grid_stroke,
        );
        x += grid_spacing;
    }

    let mut y = canvas_rect.top() + app_state.view_offset_y.rem_euclid(grid_spacing);
    while y <= canvas_rect.bottom() {
        painter.line_segment(
            [
                Pos2::new(canvas_rect.left(), y),
                Pos2::new(canvas_rect.right(), y),
            ],
            grid_stroke,
        );
        y += grid_spacing;
    }
}

/// 在画布上方和左侧绘制2D刻度尺，并显示鼠标对应的场景坐标。
fn draw_canvas_rulers(
    ui: &egui::Ui,
    painter: &egui::Painter,
    canvas_rect: Rect,
    app_state: &AppState,
) {
    if !app_state.assistant_settings.show_rulers { return; }
    let ruler_size = 22.0;
    let background = Color32::from_rgba_unmultiplied(25, 30, 38, 220);
    painter.rect_filled(Rect::from_min_max(canvas_rect.min, Pos2::new(canvas_rect.right(), canvas_rect.top() + ruler_size)), 0.0, background);
    painter.rect_filled(Rect::from_min_max(canvas_rect.min, Pos2::new(canvas_rect.left() + ruler_size, canvas_rect.bottom())), 0.0, background);
    let step = (app_state.assistant_settings.grid_size.max(8.0) * app_state.view_zoom).max(24.0);
    let mut x = canvas_rect.left() + app_state.view_offset_x.rem_euclid(step);
    while x <= canvas_rect.right() {
        let scene = screen_to_scene(Pos2::new(x, canvas_rect.top()), canvas_rect, app_state);
        painter.line_segment([Pos2::new(x, canvas_rect.top()), Pos2::new(x, canvas_rect.top() + 7.0)], Stroke::new(1.0, Color32::LIGHT_GRAY));
        painter.text(Pos2::new(x + 2.0, canvas_rect.top() + 8.0), egui::Align2::LEFT_TOP, format!("{:.0}", scene.x), egui::FontId::monospace(9.0), Color32::LIGHT_GRAY);
        x += step;
    }
    let mut y = canvas_rect.top() + app_state.view_offset_y.rem_euclid(step);
    while y <= canvas_rect.bottom() {
        let scene = screen_to_scene(Pos2::new(canvas_rect.left(), y), canvas_rect, app_state);
        painter.line_segment([Pos2::new(canvas_rect.left(), y), Pos2::new(canvas_rect.left() + 7.0, y)], Stroke::new(1.0, Color32::LIGHT_GRAY));
        painter.text(Pos2::new(canvas_rect.left() + 8.0, y + 2.0), egui::Align2::LEFT_TOP, format!("{:.0}", scene.y), egui::FontId::monospace(9.0), Color32::LIGHT_GRAY);
        y += step;
    }
    if let Some(pointer) = ui.input(|input| input.pointer.hover_pos()).filter(|position| canvas_rect.contains(*position)) {
        let scene = screen_to_scene(pointer, canvas_rect, app_state);
        painter.text(pointer + Vec2::new(12.0, 12.0), egui::Align2::LEFT_TOP, format!("X {:.1}  Y {:.1}", scene.x, scene.y), egui::FontId::monospace(11.0), Color32::WHITE);
    }
}

/// 处理物体的点击选择、位置拖动和四角缩放。
fn handle_object_interaction(ui: &egui::Ui, canvas_rect: Rect, app_state: &mut AppState) {
    let pointer_position = ui.input(|input| input.pointer.interact_pos());
    let primary_pressed = ui.input(|input| input.pointer.button_pressed(PointerButton::Primary));
    let primary_down = ui.input(|input| input.pointer.primary_down());

    if primary_pressed {
        let pointer_position = match pointer_position {
            Some(position) if canvas_rect.contains(position) => position,
            _ => return,
        };

        if let Some(resize_handle) =
            hit_selected_resize_handle(pointer_position, canvas_rect, app_state)
        {
            start_object_interaction(pointer_position, Some(resize_handle), app_state);
        } else if let Some(object_id) = find_top_object_at(pointer_position, canvas_rect, app_state)
        {
            let toggle = ui.input(|input| input.modifiers.ctrl || input.modifiers.shift);
            if toggle {
                if app_state.selected_object_ids.contains(&object_id) {
                    app_state.selected_object_ids.retain(|id| *id != object_id);
                    app_state.selected_object_id = app_state.selected_object_ids.last().copied();
                } else {
                    app_state.selected_object_ids.push(object_id);
                    app_state.selected_object_id = Some(object_id);
                }
            } else if !app_state.selected_object_ids.contains(&object_id) {
                app_state.selected_object_ids.clear();
                app_state.selected_object_ids.push(object_id);
                app_state.selected_object_id = Some(object_id);
            } else {
                app_state.selected_object_id = Some(object_id);
            }
            app_state.selected_ui_id = None;
            start_object_interaction(pointer_position, None, app_state);
        } else {
            app_state.selected_object_id = None;
            app_state.selected_object_ids.clear();
            app_state.object_interaction = None;
        }
    }

    if !primary_down {
        app_state.object_interaction = None;
        return;
    }

    let pointer_position = match pointer_position {
        Some(position) => position,
        None => return,
    };
    update_object_interaction(pointer_position, app_state);
}

/// 记录拖动或缩放开始时的鼠标位置与物体尺寸。
fn start_object_interaction(
    pointer_position: Pos2,
    resize_handle: Option<ResizeHandle>,
    app_state: &mut AppState,
) {
    let selected_id = match app_state.selected_object_id {
        Some(id) => id,
        None => return,
    };
    let game_object = match app_state
        .game_objects
        .iter()
        .find(|game_object| game_object.id == selected_id)
    {
        Some(game_object) => game_object,
        None => return,
    };

    app_state.object_interaction = Some(ObjectInteraction {
        object_id: game_object.id,
        start_pointer_x: pointer_position.x,
        start_pointer_y: pointer_position.y,
        start_x: game_object.x,
        start_y: game_object.y,
        start_width: game_object.width,
        start_height: game_object.height,
        resize_handle,
        group_start_positions: app_state
            .game_objects
            .iter()
            .filter(|object| app_state.selected_object_ids.contains(&object.id))
            .map(|object| (object.id, object.x, object.y))
            .collect(),
    });
}

/// 根据鼠标从起点移动的距离，更新物体位置或尺寸。
fn update_object_interaction(pointer_position: Pos2, app_state: &mut AppState) {
    let interaction = match &app_state.object_interaction {
        Some(interaction) => interaction,
        None => return,
    };

    let delta_x = (pointer_position.x - interaction.start_pointer_x) / app_state.view_zoom;
    let delta_y = (pointer_position.y - interaction.start_pointer_y) / app_state.view_zoom;
    let object_id = interaction.object_id;
    let start_x = interaction.start_x;
    let start_y = interaction.start_y;
    let start_width = interaction.start_width;
    let start_height = interaction.start_height;
    let resize_handle = interaction.resize_handle;
    let group_start_positions = interaction.group_start_positions.clone();

    match resize_handle {
        None => {
            let target_x = snap_scene_value(start_x + delta_x, app_state);
            let target_y = snap_scene_value(start_y + delta_y, app_state);
            let group_delta_x = target_x - start_x;
            let group_delta_y = target_y - start_y;
            for (id, original_x, original_y) in group_start_positions {
                if let Some(object) = app_state
                    .game_objects
                    .iter_mut()
                    .find(|object| object.id == id)
                {
                    object.x = original_x + group_delta_x;
                    object.y = original_y + group_delta_y;
                }
            }
        }
        Some(ResizeHandle::TopLeft) => {
            let game_object = match app_state.game_objects.iter_mut().find(|object| object.id == object_id) { Some(object) => object, None => return };
            resize_from_left(game_object, start_x, start_width, delta_x);
            resize_from_top(game_object, start_y, start_height, delta_y);
        }
        Some(ResizeHandle::TopRight) => {
            let game_object = match app_state.game_objects.iter_mut().find(|object| object.id == object_id) { Some(object) => object, None => return };
            resize_from_right(game_object, start_width, delta_x);
            resize_from_top(game_object, start_y, start_height, delta_y);
        }
        Some(ResizeHandle::BottomLeft) => {
            let game_object = match app_state.game_objects.iter_mut().find(|object| object.id == object_id) { Some(object) => object, None => return };
            resize_from_left(game_object, start_x, start_width, delta_x);
            resize_from_bottom(game_object, start_height, delta_y);
        }
        Some(ResizeHandle::BottomRight) => {
            let game_object = match app_state.game_objects.iter_mut().find(|object| object.id == object_id) { Some(object) => object, None => return };
            resize_from_right(game_object, start_width, delta_x);
            resize_from_bottom(game_object, start_height, delta_y);
        }
    }
}

/// 根据Assistant Toolkit网格吸附设置计算最终场景坐标。
fn snap_scene_value(value: f32, app_state: &AppState) -> f32 {
    let grid_size = app_state.assistant_settings.grid_size.max(1.0);
    if app_state.assistant_settings.snap_to_grid {
        (value / grid_size).round() * grid_size
    } else {
        value
    }
}

/// 从物体左边缩放，同时保证宽度不会小于最小尺寸。
fn resize_from_left(game_object: &mut GameObject, start_x: f32, start_width: f32, delta: f32) {
    let new_width = (start_width - delta).max(MIN_OBJECT_SIZE);
    game_object.x = start_x + start_width - new_width;
    game_object.width = new_width;
}

/// 从物体右边缩放，同时保证宽度不会小于最小尺寸。
fn resize_from_right(game_object: &mut GameObject, start_width: f32, delta: f32) {
    game_object.width = (start_width + delta).max(MIN_OBJECT_SIZE);
}

/// 从物体上边缩放，同时保证高度不会小于最小尺寸。
fn resize_from_top(game_object: &mut GameObject, start_y: f32, start_height: f32, delta: f32) {
    let new_height = (start_height - delta).max(MIN_OBJECT_SIZE);
    game_object.y = start_y + start_height - new_height;
    game_object.height = new_height;
}

/// 从物体下边缩放，同时保证高度不会小于最小尺寸。
fn resize_from_bottom(game_object: &mut GameObject, start_height: f32, delta: f32) {
    game_object.height = (start_height + delta).max(MIN_OBJECT_SIZE);
}

/// 按图层从高到低查找鼠标位置下方的物体。
fn find_top_object_at(
    pointer_position: Pos2,
    canvas_rect: Rect,
    app_state: &AppState,
) -> Option<u64> {
    let mut top_object_id = None;
    let mut top_layer_index = 0;

    for game_object in &app_state.game_objects {
        let object_rect = object_screen_rect(game_object, canvas_rect, app_state);
        if object_rect.contains(pointer_position)
            && (top_object_id.is_none() || game_object.layer_index >= top_layer_index)
        {
            top_object_id = Some(game_object.id);
            top_layer_index = game_object.layer_index;
        }
    }

    top_object_id
}

/// 检查鼠标是否按在选中物体的某个角落控制点上。
fn hit_selected_resize_handle(
    pointer_position: Pos2,
    canvas_rect: Rect,
    app_state: &AppState,
) -> Option<ResizeHandle> {
    let selected_id = app_state.selected_object_id?;
    let game_object = app_state
        .game_objects
        .iter()
        .find(|game_object| game_object.id == selected_id)?;
    let object_rect = object_screen_rect(game_object, canvas_rect, app_state);

    for (handle, handle_rect) in resize_handle_rects(object_rect) {
        if handle_rect.contains(pointer_position) {
            return Some(handle);
        }
    }
    None
}

/// 按图层顺序绘制物体，最后为选中物体绘制边框和四角控制点。
fn draw_game_objects(
    painter: &egui::Painter,
    canvas_rect: Rect,
    app_state: &mut AppState,
    editor_textures: &EditorTextures,
) {
    let mut sorted_objects: Vec<&GameObject> = app_state.game_objects.iter().collect();
    sorted_objects.sort_by_key(|game_object| game_object.layer_index);
    let mut rendered_count = 0;

    for game_object in sorted_objects {
        let object_rect = object_screen_rect(game_object, canvas_rect, app_state);
        if app_state.performance_settings.viewport_culling && !object_rect.intersects(canvas_rect) {
            continue;
        }
        rendered_count += 1;
        if !game_object.audio_path.is_empty() {
            painter.rect_filled(object_rect, 6.0, Color32::from_rgb(80, 65, 130));
            painter.text(
                object_rect.center(),
                egui::Align2::CENTER_CENTER,
                "♪",
                egui::FontId::proportional((30.0 * app_state.view_zoom).max(12.0)),
                Color32::WHITE,
            );
            let audio_name = std::path::Path::new(&game_object.audio_path)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("音效");
            painter.text(
                object_rect.center_bottom() + Vec2::new(0.0, 14.0),
                egui::Align2::CENTER_TOP,
                audio_name,
                egui::FontId::proportional(12.0),
                Color32::from_rgb(70, 55, 110),
            );
        } else if let Some(texture) = editor_textures.textures.get(&game_object.image_path) {
            painter.image(
                texture.id(),
                object_rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            painter.rect_filled(object_rect, 2.0, Color32::from_rgb(90, 150, 220));
        }
        painter.rect_stroke(
            object_rect,
            2.0,
            Stroke::new(1.0_f32, Color32::from_rgb(55, 105, 165)),
        );
        if let Some(collider) = &game_object.collider {
            let category_visible = if collider.is_dynamic {
                app_state.assistant_settings.show_dynamic_colliders
            } else {
                app_state.assistant_settings.show_static_colliders
            };
            if app_state.assistant_settings.show_object_colliders && category_visible {
                let color = if collider.is_dynamic {
                    Color32::from_rgb(40, 235, 120)
                } else {
                    Color32::from_rgb(255, 165, 40)
                };
                painter.rect_stroke(object_rect, 0.0, Stroke::new(3.0, color));
            }
        }
        if app_state.selected_object_ids.contains(&game_object.id)
            && app_state.selected_object_id != Some(game_object.id)
        {
            painter.rect_stroke(
                object_rect,
                0.0,
                Stroke::new(2.0, Color32::from_rgb(60, 220, 255)),
            );
        }
    }
    app_state.performance_metrics.rendered_objects += rendered_count;

    let selected_id = match app_state.selected_object_id {
        Some(id) => id,
        None => return,
    };
    let selected_object = match app_state
        .game_objects
        .iter()
        .find(|game_object| game_object.id == selected_id)
    {
        Some(game_object) => game_object,
        None => return,
    };
    let selected_rect = object_screen_rect(selected_object, canvas_rect, app_state);
    painter.rect_stroke(selected_rect, 0.0, Stroke::new(2.0_f32, Color32::WHITE));

    for (_, handle_rect) in resize_handle_rects(selected_rect) {
        painter.rect_filled(handle_rect, 1.0, Color32::WHITE);
        painter.rect_stroke(
            handle_rect,
            1.0,
            Stroke::new(1.0_f32, Color32::from_rgb(40, 90, 150)),
        );
    }
}

/// 生成物体四个角落的缩放控制点矩形。
fn resize_handle_rects(object_rect: Rect) -> [(ResizeHandle, Rect); 4] {
    [
        (
            ResizeHandle::TopLeft,
            Rect::from_center_size(object_rect.left_top(), Vec2::splat(RESIZE_HANDLE_SIZE)),
        ),
        (
            ResizeHandle::TopRight,
            Rect::from_center_size(object_rect.right_top(), Vec2::splat(RESIZE_HANDLE_SIZE)),
        ),
        (
            ResizeHandle::BottomLeft,
            Rect::from_center_size(object_rect.left_bottom(), Vec2::splat(RESIZE_HANDLE_SIZE)),
        ),
        (
            ResizeHandle::BottomRight,
            Rect::from_center_size(object_rect.right_bottom(), Vec2::splat(RESIZE_HANDLE_SIZE)),
        ),
    ]
}

/// 将游戏物体的场景坐标转换成屏幕上的矩形。
fn object_screen_rect(game_object: &GameObject, canvas_rect: Rect, app_state: &AppState) -> Rect {
    let minimum = scene_to_screen(
        Pos2::new(game_object.x, game_object.y),
        canvas_rect,
        app_state,
    );
    let size = Vec2::new(
        game_object.width * app_state.view_zoom,
        game_object.height * app_state.view_zoom,
    );
    Rect::from_min_size(minimum, size)
}

/// 将编辑器画布边界转换成场景坐标可见矩形。
fn editor_visible_scene_rect(canvas_rect: Rect, app_state: &AppState) -> Rect {
    Rect::from_min_max(
        screen_to_scene(canvas_rect.min, canvas_rect, app_state),
        screen_to_scene(canvas_rect.max, canvas_rect, app_state),
    )
}

/// 判断瓦片所在16x16区块是否与当前场景可见区域相交。
fn tile_chunk_visible(
    tile_x: i32,
    tile_y: i32,
    visible_scene: &Rect,
    tile_map: &crate::tilemap::TileMap,
) -> bool {
    const CHUNK_SIZE: i32 = 16;
    let chunk_x = tile_x.div_euclid(CHUNK_SIZE);
    let chunk_y = tile_y.div_euclid(CHUNK_SIZE);
    let tile_width = tile_map.tile_width.max(1) as f32;
    let tile_height = tile_map.tile_height.max(1) as f32;
    let minimum = Pos2::new(
        chunk_x as f32 * CHUNK_SIZE as f32 * tile_width,
        chunk_y as f32 * CHUNK_SIZE as f32 * tile_height,
    );
    let size = Vec2::new(
        CHUNK_SIZE as f32 * tile_width,
        CHUNK_SIZE as f32 * tile_height,
    );
    Rect::from_min_size(minimum, size).intersects(*visible_scene)
}

/// 将场景坐标转换成画布中的屏幕坐标。
fn scene_to_screen(scene_position: Pos2, canvas_rect: Rect, app_state: &AppState) -> Pos2 {
    Pos2::new(
        canvas_rect.left() + app_state.view_offset_x + scene_position.x * app_state.view_zoom,
        canvas_rect.top() + app_state.view_offset_y + scene_position.y * app_state.view_zoom,
    )
}

/// 将画布中的屏幕坐标转换回场景坐标。
fn screen_to_scene(screen_position: Pos2, canvas_rect: Rect, app_state: &AppState) -> Pos2 {
    Pos2::new(
        (screen_position.x - canvas_rect.left() - app_state.view_offset_x) / app_state.view_zoom,
        (screen_position.y - canvas_rect.top() - app_state.view_offset_y) / app_state.view_zoom,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证超大图片在上传egui前会缩小到GPU允许范围。
    #[test]
    fn oversized_editor_texture_is_resized() {
        let context = egui::Context::default();
        let image = image::DynamicImage::new_rgba8(3184, 2232);
        let color_image = image_to_egui_texture(&context, image);
        let maximum_side = context.input(|input| input.max_texture_side);

        assert!(color_image.width() <= maximum_side);
        assert!(color_image.height() <= maximum_side);
    }
}
