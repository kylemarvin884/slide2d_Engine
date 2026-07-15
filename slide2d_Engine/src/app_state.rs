use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use crate::animation::AnimationEditorState;
use crate::blueprint::model::Blueprint;
use crate::game_ui::{UiElement, UiTemplate};
use crate::plugins::PluginRegistry;
use crate::tilemap::{TileEditorState, TileMap};

/// 场景文件的顶层数据结构。
#[derive(Clone, Serialize, Deserialize)]
pub struct Scene {
    /// Slide2D场景JSON内置格式标识。
    #[serde(default = "scene_format")]
    pub slide2d_engine: String,
    /// 场景名称，用于工程菜单和蓝图场景切换。
    #[serde(default = "default_scene_name")]
    pub name: String,
    pub game_objects: Vec<GameObject>,
    #[serde(default = "TileMap::new")]
    pub tile_map: TileMap,
    #[serde(default)]
    pub ui_elements: Vec<UiElement>,
}

/// 描述游戏物体使用的矩形碰撞体。
#[derive(Clone, Serialize, Deserialize)]
pub struct ColliderConfig {
    pub is_dynamic: bool,
}

/// 表示场景中的一个游戏物体。
///
/// 该结构体使用serde标记，后续可以直接保存为JSON或从JSON读取。
#[derive(Clone, Serialize, Deserialize)]
pub struct GameObject {
    pub id: u64,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub layer_index: u32,
    pub image_path: String,
    /// 音效物体的资源路径。非音效物体为空字符串。
    #[serde(default)]
    pub audio_path: String,
    /// 物体绑定的精灵动画资源路径。
    #[serde(default)]
    pub animation_path: String,
    /// Runtime是否播放当前动画，蓝图播放/暂停节点会修改该值。
    #[serde(default = "default_animation_playing")]
    pub animation_playing: bool,
    pub collider: Option<ColliderConfig>,
    #[serde(default)]
    pub blueprint: Blueprint,
    /// 该物体专属蓝图文件名，场景导出时会同步写入。
    #[serde(default)]
    pub blueprint_file: String,
    /// 当前物体拥有的运行时数值变量，例如生命值、分数和速度。
    #[serde(default)]
    pub variables: HashMap<String, f32>,
}

/// 表示当前正在拖动哪一种缩放控制点。
#[derive(Clone, Copy, PartialEq)]
pub enum ResizeHandle {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// 保存一次物体拖拽或缩放开始时的数据。
///
/// 使用起始数据计算位置可以避免每帧累加鼠标误差。
pub struct ObjectInteraction {
    pub object_id: u64,
    pub start_pointer_x: f32,
    pub start_pointer_y: f32,
    pub start_x: f32,
    pub start_y: f32,
    pub start_width: f32,
    pub start_height: f32,
    pub resize_handle: Option<ResizeHandle>,
    /// 多选拖动开始时每个对象的原始坐标。
    pub group_start_positions: Vec<(u64, f32, f32)>,
}

/// 可持久化的编辑器摄像机书签。
#[derive(Clone, Serialize, Deserialize)]
pub struct CameraBookmark {
    pub name: String,
    pub offset_x: f32,
    pub offset_y: f32,
    pub zoom: f32,
}

/// Slide2D Assistant Toolkit随工程保存的全部开关和书签。
#[derive(Clone, Serialize, Deserialize)]
pub struct AssistantSettings {
    pub grid_size: f32,
    pub snap_to_grid: bool,
    pub show_rulers: bool,
    pub show_object_colliders: bool,
    pub show_tile_colliders: bool,
    pub show_static_colliders: bool,
    pub show_dynamic_colliders: bool,
    pub transparent_screenshot: bool,
    #[serde(default)]
    pub camera_bookmarks: Vec<CameraBookmark>,
}

impl AssistantSettings {
    /// 创建适合PPT式拖拽编辑的默认辅助工具设置。
    pub fn new() -> Self {
        Self {
            grid_size: 24.0,
            snap_to_grid: false,
            show_rulers: true,
            show_object_colliders: false,
            show_tile_colliders: false,
            show_static_colliders: true,
            show_dynamic_colliders: true,
            transparent_screenshot: false,
            camera_bookmarks: Vec::new(),
        }
    }
}

/// 保存操作系统拖入的图片路径及鼠标放下时的窗口坐标。
pub struct PendingImageDrop {
    pub path: PathBuf,
    pub screen_x: f32,
    pub screen_y: f32,
}

/// 保存只影响编辑器界面的用户设置。
pub struct EditorSettings {
    pub show_grid: bool,
    pub dark_theme: bool,
    pub canvas_background: [u8; 3],
    pub maximum_imported_image_size: f32,
    pub blueprint_editor_mode: BlueprintEditorMode,
}

/// Slide2D Performance System全部可视化优化开关。
#[derive(Clone, Serialize, Deserialize)]
pub struct PerformanceSettings {
    pub viewport_culling: bool,
    pub tile_chunk_culling: bool,
    pub resource_cache: bool,
    pub idle_cache_release: bool,
    pub blueprint_cache: bool,
    pub dormant_blueprints: bool,
    pub static_physics_cache: bool,
    pub distant_physics_sleep: bool,
    pub automatic_image_compression: bool,
    pub activity_margin: f32,
}

impl PerformanceSettings {
    /// 创建兼顾低配设备和逻辑安全的默认优化设置。
    pub fn new() -> Self {
        Self {
            viewport_culling: true,
            tile_chunk_culling: true,
            resource_cache: true,
            idle_cache_release: true,
            blueprint_cache: true,
            dormant_blueprints: true,
            static_physics_cache: true,
            distant_physics_sleep: true,
            automatic_image_compression: true,
            activity_margin: 256.0,
        }
    }
}

/// 性能监视器实时显示的编辑器和Runtime阶段指标。
pub struct PerformanceMetrics {
    pub frame_rate: f32,
    pub runtime_frame_rate: f32,
    pub memory_bytes: u64,
    pub cached_assets: usize,
    pub rendered_objects: usize,
    pub rendered_tiles: usize,
    pub blueprint_time_ms: f32,
    pub physics_time_ms: f32,
    pub last_frame_at: Instant,
}

/// Runtime写给编辑器性能监视器的轻量JSON指标。
#[derive(Serialize, Deserialize)]
pub struct RuntimePerformanceReport {
    pub slide2d_engine: String,
    pub frame_rate: f32,
    pub memory_bytes: u64,
    pub cached_assets: usize,
    pub rendered_objects: usize,
    pub rendered_tiles: usize,
    pub blueprint_time_ms: f32,
    pub physics_time_ms: f32,
}

impl PerformanceMetrics {
    /// 创建空指标并记录首帧时间。
    pub fn new() -> Self {
        Self {
            frame_rate: 0.0,
            runtime_frame_rate: 0.0,
            memory_bytes: 0,
            cached_assets: 0,
            rendered_objects: 0,
            rendered_tiles: 0,
            blueprint_time_ms: 0.0,
            physics_time_ms: 0.0,
            last_frame_at: Instant::now(),
        }
    }

    /// 根据本次重绘间隔平滑更新编辑器帧率。
    pub fn update_editor_frame_rate(&mut self) {
        let now = Instant::now();
        let elapsed = now
            .duration_since(self.last_frame_at)
            .as_secs_f32()
            .max(0.0001);
        self.last_frame_at = now;
        let current = 1.0 / elapsed;
        self.frame_rate = if self.frame_rate <= 0.0 {
            current
        } else {
            self.frame_rate * 0.9 + current * 0.1
        };
    }
}

/// 决定蓝图编辑器显示在IDE标签页中，还是显示为独立浮动窗口。
#[derive(Clone, Copy, PartialEq)]
pub enum BlueprintEditorMode {
    IdeTabs,
    SeparateWindow,
}

/// 未保存修改确认后要继续执行的编辑器动作。
#[derive(Clone, Copy, PartialEq)]
pub enum PendingEditorAction {
    NewProject,
    ExitEditor,
}

impl EditorSettings {
    /// 创建编辑器默认设置。
    pub fn new() -> Self {
        Self {
            show_grid: true,
            dark_theme: true,
            canvas_background: [225, 225, 225],
            maximum_imported_image_size: 500.0,
            blueprint_editor_mode: BlueprintEditorMode::IdeTabs,
        }
    }
}

/// 保存整个编辑器都会使用的全局状态。
pub struct AppState {
    pub grid_size: f32,
    pub game_objects: Vec<GameObject>,
    pub selected_object_id: Option<u64>,
    /// 场景画布批量对齐和复制使用的对象多选集合。
    pub selected_object_ids: Vec<u64>,
    pub next_object_id: u64,
    pub next_layer_index: u32,
    pub view_offset_x: f32,
    pub view_offset_y: f32,
    pub view_zoom: f32,
    pub object_interaction: Option<ObjectInteraction>,
    pub status_message: String,
    pub blueprint_object_id: Option<u64>,
    /// 当前打开蓝图的UI元素ID；与GameObject蓝图二选一。
    pub blueprint_ui_id: Option<u64>,
    pub pending_blueprint_output: Option<u64>,
    /// 正在拖出的执行端口编号：普通端口为0，If真/假端口为1/2。
    pub pending_blueprint_output_port: u8,
    pub selected_blueprint_node_id: Option<u64>,
    /// 蓝图批量操作使用的节点ID列表。
    pub selected_blueprint_node_ids: Vec<u64>,
    pub pending_image_drops: Vec<PendingImageDrop>,
    pub is_file_hovering: bool,
    pub settings_window_open: bool,
    pub editor_settings: EditorSettings,
    pub blueprint_tab_active: bool,
    pub blueprint_view_offset_x: f32,
    pub blueprint_view_offset_y: f32,
    pub blueprint_view_zoom: f32,
    /// 当前从资源库拖出的图片路径，释放后清空，允许同一素材重复拖入。
    pub dragging_image_asset: Option<PathBuf>,
    /// 当前从资源库拖出的音效路径。
    pub dragging_audio_asset: Option<PathBuf>,
    /// 当前从资源库拖出的Actor模板路径。
    pub dragging_actor_asset: Option<PathBuf>,
    /// 当前从资源库拖出的动画资源路径。
    pub dragging_animation_asset: Option<PathBuf>,
    pub animation_editor: AnimationEditorState,
    pub asset_refresh_requested: bool,
    pub tile_map: TileMap,
    pub tile_editor: TileEditorState,
    pub ui_elements: Vec<UiElement>,
    pub selected_ui_id: Option<u64>,
    pub next_ui_id: u64,
    pub next_ui_layer: u32,
    pub dragging_ui_template: Option<UiTemplate>,
    /// 当前工程中的全部场景，当前场景也会在切换和保存前同步到这里。
    pub project_scenes: Vec<Scene>,
    /// 当前正在编辑的场景下标。
    pub active_scene_index: usize,
    /// Runtime启动时首先加载的场景名称。
    pub startup_scene_name: String,
    /// 所有场景蓝图共同读写的全局数值变量。
    pub global_variables: HashMap<String, f32>,
    /// 当前打开或保存的.slide2d工程路径。
    pub project_file_path: Option<PathBuf>,
    /// 当前工程解包和资源扫描使用的工作目录。
    pub project_root: PathBuf,
    /// 最近成功打开或创建的五个工程文件夹。
    pub recent_projects: Vec<PathBuf>,
    /// 最近一次成功保存时的工程数据快照。
    pub saved_project_snapshot: String,
    /// 未保存确认框关闭后要继续执行的动作。
    pub pending_editor_action: Option<PendingEditorAction>,
    /// 保存完成提示框是否显示。
    pub save_notice_open: bool,
    /// 事件循环读到该标记后安全退出编辑器。
    pub exit_requested: bool,
    /// 当前工程的Slide2D声明式插件注册表。
    pub plugin_registry: PluginRegistry,
    /// 插件管理器窗口是否打开。
    pub plugin_manager_open: bool,
    /// 当前打开的声明式插件工具ID。
    pub open_plugin_tool: Option<(String, String)>,
    /// 当前工程保存的性能优化开关。
    pub performance_settings: PerformanceSettings,
    /// 性能监视器是否打开。
    pub performance_monitor_open: bool,
    /// 当前编辑器实时性能指标。
    pub performance_metrics: PerformanceMetrics,
    /// Slide2D语言设置弹窗是否打开。
    pub language_settings_open: bool,
    /// Slide2D Assistant Toolkit窗口是否打开。
    pub assistant_toolkit_open: bool,
    /// 当前工程保存的辅助工具设置。
    pub assistant_settings: AssistantSettings,
    /// 素材批处理窗口临时选择的文件。
    pub assistant_selected_assets: Vec<PathBuf>,
    /// 批量有序重命名使用的基础名称。
    pub assistant_batch_name: String,
    /// 批量贴图缩放允许的最大边长。
    pub assistant_texture_max_size: u32,
    /// 截图合成器记录的最近画布像素尺寸。
    pub last_canvas_width: u32,
    pub last_canvas_height: u32,
}

impl AppState {
    /// 创建一份默认的全局状态。
    pub fn new() -> Self {
        let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let plugin_registry = PluginRegistry::load_new_project(project_root.clone());
        Self {
            grid_size: 24.0,
            game_objects: Vec::new(),
            selected_object_id: None,
            selected_object_ids: Vec::new(),
            next_object_id: 1,
            next_layer_index: 0,
            view_offset_x: 0.0,
            view_offset_y: 0.0,
            view_zoom: 1.0,
            object_interaction: None,
            status_message: String::new(),
            blueprint_object_id: None,
            blueprint_ui_id: None,
            pending_blueprint_output: None,
            pending_blueprint_output_port: 0,
            selected_blueprint_node_id: None,
            selected_blueprint_node_ids: Vec::new(),
            pending_image_drops: Vec::new(),
            is_file_hovering: false,
            settings_window_open: false,
            editor_settings: EditorSettings::new(),
            blueprint_tab_active: false,
            blueprint_view_offset_x: 0.0,
            blueprint_view_offset_y: 0.0,
            blueprint_view_zoom: 1.0,
            dragging_image_asset: None,
            dragging_audio_asset: None,
            dragging_actor_asset: None,
            dragging_animation_asset: None,
            animation_editor: AnimationEditorState::new(),
            asset_refresh_requested: false,
            tile_map: TileMap::new(),
            tile_editor: TileEditorState::new(),
            ui_elements: Vec::new(),
            selected_ui_id: None,
            next_ui_id: 1,
            next_ui_layer: 0,
            dragging_ui_template: None,
            project_scenes: vec![Scene::empty("场景1")],
            active_scene_index: 0,
            startup_scene_name: "场景1".to_owned(),
            global_variables: HashMap::new(),
            project_file_path: None,
            project_root,
            recent_projects: crate::project::load_recent_projects(),
            saved_project_snapshot: String::new(),
            pending_editor_action: None,
            save_notice_open: false,
            exit_requested: false,
            plugin_registry,
            plugin_manager_open: false,
            open_plugin_tool: None,
            performance_settings: PerformanceSettings::new(),
            performance_monitor_open: false,
            performance_metrics: PerformanceMetrics::new(),
            language_settings_open: false,
            assistant_toolkit_open: false,
            assistant_settings: AssistantSettings::new(),
            assistant_selected_assets: Vec::new(),
            assistant_batch_name: "asset".to_owned(),
            assistant_texture_max_size: 2048,
            last_canvas_width: 1280,
            last_canvas_height: 720,
        }
    }

    /// 从已经保存的场景恢复编辑器状态，并重新计算后续ID。
    pub fn from_scene(scene: Scene) -> Self {
        let mut state = Self::new();
        state.project_scenes = vec![scene.clone()];
        state.startup_scene_name = scene.name.clone();
        state.game_objects = scene.game_objects;
        state.tile_map = scene.tile_map;
        state.ui_elements = scene.ui_elements;
        for object in &mut state.game_objects {
            if object.blueprint_file.is_empty() {
                object.blueprint_file = format!("blueprint_{}.json", object.id);
            }
        }
        state.next_object_id = state
            .game_objects
            .iter()
            .map(|object| object.id)
            .max()
            .unwrap_or(0)
            + 1;
        state.next_layer_index = state
            .game_objects
            .iter()
            .map(|object| object.layer_index)
            .max()
            .unwrap_or(0)
            + 1;
        state.next_ui_id = state
            .ui_elements
            .iter()
            .map(|element| element.id)
            .max()
            .unwrap_or(0)
            + 1;
        state.next_ui_layer = state
            .ui_elements
            .iter()
            .map(|element| element.layer_index)
            .max()
            .unwrap_or(0)
            + 1;
        state
    }

    /// 使用图片信息创建一个新的场景物体，并自动放到最高图层。
    pub fn add_image_object(
        &mut self,
        image_path: String,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) {
        let game_object = GameObject {
            id: self.next_object_id,
            x,
            y,
            width,
            height,
            layer_index: self.next_layer_index,
            image_path,
            audio_path: String::new(),
            animation_path: String::new(),
            animation_playing: true,
            collider: None,
            blueprint: Blueprint::new(),
            blueprint_file: format!("blueprint_{}.json", self.next_object_id),
            variables: HashMap::new(),
        };
        self.selected_object_id = Some(game_object.id);
        self.selected_ui_id = None;
        self.game_objects.push(game_object);
        self.next_object_id += 1;
        self.next_layer_index += 1;
    }

    /// 创建一个只在Runtime播放、不在游戏画面显示的音效物体。
    pub fn add_audio_object(&mut self, audio_path: String, x: f32, y: f32) {
        let game_object = GameObject {
            id: self.next_object_id,
            x,
            y,
            width: 48.0,
            height: 48.0,
            layer_index: self.next_layer_index,
            image_path: String::new(),
            audio_path,
            animation_path: String::new(),
            animation_playing: true,
            collider: None,
            blueprint: Blueprint::new(),
            blueprint_file: format!("blueprint_{}.json", self.next_object_id),
            variables: HashMap::new(),
        };
        self.selected_object_id = Some(game_object.id);
        self.selected_ui_id = None;
        self.game_objects.push(game_object);
        self.next_object_id += 1;
        self.next_layer_index += 1;
    }

    /// 在场景中添加一个测试方块，并自动选中新物体。
    pub fn add_test_object(&mut self) {
        let position_offset = self.game_objects.len() as f32 * 20.0;
        let game_object = GameObject {
            id: self.next_object_id,
            x: 80.0 + position_offset,
            y: 80.0 + position_offset,
            width: 160.0,
            height: 100.0,
            layer_index: self.next_layer_index,
            image_path: String::new(),
            audio_path: String::new(),
            animation_path: String::new(),
            animation_playing: true,
            collider: None,
            blueprint: Blueprint::new(),
            blueprint_file: format!("blueprint_{}.json", self.next_object_id),
            variables: HashMap::new(),
        };

        self.selected_object_id = Some(game_object.id);
        self.selected_ui_id = None;
        self.game_objects.push(game_object);
        self.next_object_id += 1;
        self.next_layer_index += 1;
    }

    /// 将编辑器中的游戏物体复制到可序列化的场景结构中。
    pub fn create_scene(&self) -> Scene {
        Scene {
            slide2d_engine: scene_format(),
            name: self.active_scene_name().to_owned(),
            game_objects: self.game_objects.clone(),
            tile_map: self.tile_map.clone(),
            ui_elements: self.ui_elements.clone(),
        }
    }

    /// 在指定屏幕坐标创建UI元素并自动选中。
    pub fn add_ui_element(&mut self, template: UiTemplate, x: f32, y: f32) {
        let element = UiElement::from_template(self.next_ui_id, self.next_ui_layer, template, x, y);
        self.selected_object_id = None;
        self.selected_ui_id = Some(element.id);
        self.ui_elements.push(element);
        self.next_ui_id += 1;
        self.next_ui_layer += 1;
    }

    /// 返回当前选中UI元素的可变引用。
    pub fn selected_ui_mut(&mut self) -> Option<&mut UiElement> {
        let id = self.selected_ui_id?;
        self.ui_elements.iter_mut().find(|element| element.id == id)
    }

    /// 判断当前是否打开了任意物体或UI蓝图。
    pub fn blueprint_owner_is_open(&self) -> bool {
        self.blueprint_object_id.is_some() || self.blueprint_ui_id.is_some()
    }

    /// 关闭当前蓝图持有者并清理节点交互状态。
    pub fn close_blueprint_owner(&mut self) {
        self.blueprint_object_id = None;
        self.blueprint_ui_id = None;
        self.pending_blueprint_output = None;
        self.selected_blueprint_node_id = None;
        self.selected_blueprint_node_ids.clear();
    }

    /// 根据ID查找并返回游戏物体的可变引用。
    pub fn selected_object_mut(&mut self) -> Option<&mut GameObject> {
        let selected_id = self.selected_object_id?;
        self.game_objects
            .iter_mut()
            .find(|game_object| game_object.id == selected_id)
    }

    /// 返回当前场景名称；旧数据没有名称时仍使用“场景1”。
    pub fn active_scene_name(&self) -> &str {
        self.project_scenes
            .get(self.active_scene_index)
            .map(|scene| scene.name.as_str())
            .unwrap_or("场景1")
    }

    /// 将画布内容同步回工程场景列表，保存工程和切换场景前都要调用。
    pub fn store_active_scene(&mut self) {
        let scene = self.create_scene();
        if self.active_scene_index < self.project_scenes.len() {
            self.project_scenes[self.active_scene_index] = scene;
        } else {
            self.project_scenes.push(scene);
            self.active_scene_index = self.project_scenes.len() - 1;
        }
    }

    /// 切换到指定场景并恢复其中的物体、瓦片和UI。
    pub fn switch_scene(&mut self, scene_index: usize) {
        if scene_index >= self.project_scenes.len() || scene_index == self.active_scene_index {
            return;
        }
        self.store_active_scene();
        let scene = self.project_scenes[scene_index].clone();
        let project_data = (
            self.project_scenes.clone(),
            self.startup_scene_name.clone(),
            self.global_variables.clone(),
            self.project_file_path.clone(),
            self.project_root.clone(),
            self.recent_projects.clone(),
            self.saved_project_snapshot.clone(),
            self.plugin_registry.clone(),
            self.performance_settings.clone(),
            self.assistant_settings.clone(),
        );
        let mut restored = Self::from_scene(scene);
        restored.project_scenes = project_data.0;
        restored.active_scene_index = scene_index;
        restored.startup_scene_name = project_data.1;
        restored.global_variables = project_data.2;
        restored.project_file_path = project_data.3;
        restored.project_root = project_data.4;
        restored.recent_projects = project_data.5;
        restored.saved_project_snapshot = project_data.6;
        restored.plugin_registry = project_data.7;
        restored.performance_settings = project_data.8;
        restored.assistant_settings = project_data.9;
        restored.grid_size = restored.assistant_settings.grid_size;
        restored.status_message = format!("已切换场景：{}", restored.active_scene_name());
        *self = restored;
    }

    /// 新建一个空场景并立即切换过去。
    pub fn add_scene(&mut self, name: String) {
        self.store_active_scene();
        let final_name = if name.trim().is_empty() {
            format!("场景{}", self.project_scenes.len() + 1)
        } else {
            name.trim().to_owned()
        };
        self.project_scenes.push(Scene::empty(&final_name));
        let index = self.project_scenes.len() - 1;
        self.switch_scene(index);
    }

    /// 生成只包含工程持久化数据的稳定JSON快照，用于判断是否存在未保存修改。
    pub fn project_snapshot(&self) -> String {
        let mut scenes = self.project_scenes.clone();
        let active_scene = self.create_scene();
        if self.active_scene_index < scenes.len() {
            scenes[self.active_scene_index] = active_scene;
        }
        let mut enabled_plugins: Vec<String> =
            self.plugin_registry.enabled_ids().into_iter().collect();
        enabled_plugins.sort();
        serde_json::to_string(&(
            scenes,
            &self.startup_scene_name,
            &self.global_variables,
            enabled_plugins,
            &self.performance_settings,
            &self.assistant_settings,
        ))
        .unwrap_or_default()
    }

    /// 保存或打开工程成功后记录当前状态为未修改基准。
    pub fn mark_project_saved(&mut self) {
        self.saved_project_snapshot = self.project_snapshot();
    }

    /// 判断场景、瓦片、UI、蓝图或全局变量是否已发生未保存变化。
    pub fn has_unsaved_changes(&self) -> bool {
        !self.saved_project_snapshot.is_empty()
            && self.saved_project_snapshot != self.project_snapshot()
    }

    /// 清除当前场景全部物体，并同步清理选择、交互和物体蓝图窗口状态。
    pub fn clear_scene_objects(&mut self) {
        self.game_objects.clear();
        self.selected_object_id = None;
        self.selected_object_ids.clear();
        self.object_interaction = None;
        self.blueprint_object_id = None;
        self.blueprint_tab_active = false;
    }
}

impl Scene {
    /// 创建包含默认瓦片地图的空场景。
    pub fn empty(name: &str) -> Self {
        Self {
            slide2d_engine: scene_format(),
            name: name.to_owned(),
            game_objects: Vec::new(),
            tile_map: TileMap::new(),
            ui_elements: Vec::new(),
        }
    }
}

/// 旧场景文件缺少名称时提供稳定名称。
fn default_scene_name() -> String {
    "场景1".to_owned()
}

/// 返回场景文件固定的Slide2D格式标识。
fn scene_format() -> String {
    "SLIDE2D_SCENE".to_owned()
}

/// 旧场景缺少animation_playing字段时默认播放动画。
fn default_animation_playing() -> bool {
    true
}

#[cfg(test)]
mod project_state_tests {
    use super::*;

    /// 验证工程保存基准可以检测画布对象修改。
    #[test]
    fn project_snapshot_detects_unsaved_changes() {
        let mut state = AppState::new();
        state.mark_project_saved();
        assert!(!state.has_unsaved_changes());
        state.add_test_object();
        assert!(state.has_unsaved_changes());
        state.mark_project_saved();
        assert!(!state.has_unsaved_changes());
    }

    /// 验证性能设置可完整写入和读取Slide2D工程JSON。
    #[test]
    fn performance_settings_json_round_trip() {
        let mut settings = PerformanceSettings::new();
        settings.viewport_culling = false;
        settings.activity_margin = 512.0;
        let json = serde_json::to_string(&settings).expect("性能设置应可保存");
        let restored: PerformanceSettings = serde_json::from_str(&json).expect("性能设置应可读取");
        assert!(!restored.viewport_culling);
        assert_eq!(restored.activity_margin, 512.0);
    }
}
