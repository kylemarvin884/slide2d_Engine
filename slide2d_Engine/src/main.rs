mod animation;
mod app_state;
mod assets;
mod blueprint;
mod editor;
mod game_ui;
mod localization;
mod plugins;
mod project;
mod runtime;
mod tilemap;

use app_state::{AppState, Scene};

/// 程序入口。根据命令行参数启动编辑器窗口或独立游戏运行时窗口。
fn main() {
    localization::initialize();
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.len() >= 3 && arguments[1] == "--runtime" {
        if let Err(error) = runtime::run(&arguments[2]) {
            eprintln!("启动 Slide2D Runtime 失败：{error}");
        }
        return;
    }

    let app_state = load_saved_editor_state();

    if let Err(error) = editor::run(app_state) {
        eprintln!("启动 Slide2D Engine 失败：{error}");
    }
}

/// 如果工作目录中有scenes.json，就恢复上次保存的场景；否则创建空场景。
fn load_saved_editor_state() -> AppState {
    let scene_path = std::path::Path::new("scenes.json");
    let json_text = match std::fs::read_to_string(scene_path) {
        Ok(text) => text,
        Err(_) => return AppState::new(),
    };
    let scene = match serde_json::from_str::<Scene>(&json_text) {
        Ok(scene) => scene,
        Err(error) => {
            eprintln!("读取scenes.json失败，将创建空场景：{error}");
            return AppState::new();
        }
    };
    let mut state = AppState::from_scene(scene);
    for object in &mut state.game_objects {
        let blueprint_path = std::path::Path::new(&object.blueprint_file);
        let blueprint_text = match std::fs::read_to_string(blueprint_path) {
            Ok(text) => text,
            Err(_) => continue,
        };
        if let Ok(blueprint) = serde_json::from_str(&blueprint_text) {
            object.blueprint = blueprint;
        }
    }
    state
}
