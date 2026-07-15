//! Slide2D工程文件的保存、打开和资源打包。

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::app_state::{AppState, AssistantSettings, PerformanceSettings, Scene};
use crate::plugins::PluginRegistry;

/// 当前工程文件格式版本。
pub const PROJECT_FORMAT_VERSION: u32 = 1;

/// 编辑工程文件夹中的内部清单文件名。
pub const PROJECT_MANIFEST_NAME: &str = "slide2d.project.json";

/// 工程文件夹中的验证文件名。
pub const PROJECT_VERIFICATION_NAME: &str = "slide2d.verify.json";

/// 验证文件使用的固定标识，用于排除误选的普通文件夹。
pub const PROJECT_MAGIC: &str = "SLIDE2D_PROJECT";

/// 最近工程历史文件固定标识。
const RECENT_PROJECTS_MAGIC: &str = "SLIDE2D_RECENT_PROJECTS";

/// 最近工程历史JSON。
#[derive(Serialize, Deserialize)]
struct RecentProjectsFile {
    slide2d_engine: String,
    projects: Vec<PathBuf>,
}

/// 读取最近打开的五个Slide2D工程文件夹。
pub fn load_recent_projects() -> Vec<PathBuf> {
    let path = recent_projects_path();
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(_) => return Vec::new(),
    };
    let history: RecentProjectsFile = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };
    if history.slide2d_engine != RECENT_PROJECTS_MAGIC {
        return Vec::new();
    }
    history
        .projects
        .into_iter()
        .filter(|path| path.is_dir())
        .take(5)
        .collect()
}

/// 将工程移到最近列表首位，并永久保存最多五条记录。
pub fn remember_recent_project(projects: &mut Vec<PathBuf>, path: &Path) -> Result<(), String> {
    projects.retain(|existing| existing != path);
    projects.insert(0, path.to_path_buf());
    projects.truncate(5);
    let history = RecentProjectsFile {
        slide2d_engine: RECENT_PROJECTS_MAGIC.to_owned(),
        projects: projects.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&history)
        .map_err(|error| format!("生成Slide2D最近工程记录失败：{error}"))?;
    fs::write(recent_projects_path(), bytes)
        .map_err(|error| format!("保存Slide2D最近工程记录失败：{error}"))
}

/// 返回用户临时目录中的Slide2D最近工程记录路径。
fn recent_projects_path() -> PathBuf {
    std::env::temp_dir().join("slide2d_recent_projects.json")
}

/// 验证主工程JSON内置的Slide2D引擎标识和版本。
fn validate_project_identity(project: &Slide2dProject) -> Result<(), String> {
    if project.slide2d_engine != PROJECT_MAGIC {
        return Err("工程文件缺少有效的Slide2D引擎标识".to_owned());
    }
    if project.format_version != PROJECT_FORMAT_VERSION {
        return Err(format!(
            "工程格式版本{}不受支持，当前版本为{}",
            project.format_version, PROJECT_FORMAT_VERSION
        ));
    }
    Ok(())
}

/// 工程文件夹验证信息。
#[derive(Clone, Serialize, Deserialize)]
pub struct ProjectVerification {
    pub magic: String,
    pub format_version: u32,
    pub project_file: String,
    pub file_size: u64,
    pub checksum: String,
}

/// 工程中内嵌的单个素材文件。
#[derive(Clone, Serialize, Deserialize)]
pub struct PackedAsset {
    pub relative_path: String,
    pub bytes: Vec<u8>,
}

/// 工程包中显式保存的资源目录，使空文件夹也能完整恢复。
#[derive(Clone, Serialize, Deserialize)]
pub struct PackedDirectory {
    pub relative_path: String,
}

/// .slide2d文件的完整数据，JSON本身就是可移植的单文件工程包。
#[derive(Clone, Serialize, Deserialize)]
pub struct Slide2dProject {
    /// 固定的Slide2D格式标识，避免普通JSON被误认为工程文件。
    pub slide2d_engine: String,
    pub format_version: u32,
    pub name: String,
    pub startup_scene_name: String,
    pub active_scene_name: String,
    pub scenes: Vec<Scene>,
    /// Scene Manager自定义分类文件夹。
    #[serde(default = "default_scene_categories")]
    pub scene_categories: Vec<String>,
    #[serde(default)]
    pub global_variables: HashMap<String, f32>,
    #[serde(default)]
    pub assets: Vec<PackedAsset>,
    #[serde(default)]
    pub asset_directories: Vec<PackedDirectory>,
    /// 当前工程启用的Slide2D声明式插件ID。
    #[serde(default = "default_enabled_plugins")]
    pub enabled_plugins: std::collections::HashSet<String>,
    /// Slide2D Performance System工程级优化开关。
    #[serde(default = "default_performance_settings")]
    pub performance_settings: PerformanceSettings,
    /// Slide2D Assistant Toolkit工程级设置和摄像机书签。
    #[serde(default = "default_assistant_settings")]
    pub assistant_settings: AssistantSettings,
}

/// 旧工程没有插件状态字段时默认启用官方示例插件。
fn default_enabled_plugins() -> std::collections::HashSet<String> {
    std::collections::HashSet::from([crate::plugins::OFFICIAL_PICKUP_PLUGIN_ID.to_owned()])
}

/// 旧工程没有性能字段时使用安全默认优化设置。
fn default_performance_settings() -> PerformanceSettings {
    PerformanceSettings::new()
}

/// 旧工程没有辅助工具字段时使用默认设置。
fn default_assistant_settings() -> AssistantSettings {
    AssistantSettings::new()
}

/// 旧工程使用的默认场景分类。
fn default_scene_categories() -> Vec<String> {
    vec![
        "Main Menu".to_owned(),
        "Levels".to_owned(),
        "Ending".to_owned(),
    ]
}

/// 把缺少扩展名的路径自动补成.slide2d。
pub fn ensure_project_extension(path: PathBuf) -> PathBuf {
    if path.extension().and_then(|value| value.to_str()) == Some("slide2d") {
        path
    } else {
        path.with_extension("slide2d")
    }
}

/// 保存当前编辑器状态，并打包Content和旧版assets中的全部资源。
pub fn save_project(app_state: &mut AppState, path: &Path) -> Result<(), String> {
    app_state.store_active_scene();
    let mut assets = collect_assets(
        &app_state.project_root.join("assets"),
        &app_state.project_root,
    )?;
    assets.extend(collect_assets(
        &app_state.project_root.join("Content"),
        &app_state.project_root,
    )?);
    assets.extend(collect_assets(
        &app_state.project_root.join("plugins"),
        &app_state.project_root,
    )?);
    let mut asset_directories = collect_directories(
        &app_state.project_root.join("Content"),
        &app_state.project_root,
    )?;
    asset_directories.extend(collect_directories(
        &app_state.project_root.join("assets"),
        &app_state.project_root,
    )?);
    asset_directories.extend(collect_directories(
        &app_state.project_root.join("plugins"),
        &app_state.project_root,
    )?);
    let project = Slide2dProject {
        slide2d_engine: PROJECT_MAGIC.to_owned(),
        format_version: PROJECT_FORMAT_VERSION,
        name: path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("Slide2D工程")
            .to_owned(),
        startup_scene_name: app_state.startup_scene_name.clone(),
        active_scene_name: app_state.active_scene_name().to_owned(),
        scenes: app_state.project_scenes.clone(),
        scene_categories: app_state.scene_categories.clone(),
        global_variables: app_state.global_variables.clone(),
        assets,
        asset_directories,
        enabled_plugins: app_state.plugin_registry.enabled_ids(),
        performance_settings: app_state.performance_settings.clone(),
        assistant_settings: app_state.assistant_settings.clone(),
    };
    let json = serde_json::to_vec_pretty(&project)
        .map_err(|error| format!("生成工程文件失败：{error}"))?;
    atomic_write(path, &json).map_err(|error| format!("保存工程文件失败：{error}"))?;
    app_state.project_file_path = Some(path.to_path_buf());
    write_scene_files(app_state)?;
    Ok(())
}

/// 将当前编辑状态保存到工程文件夹，不打包素材字节。
pub fn save_project_folder(app_state: &mut AppState, folder: &Path) -> Result<(), String> {
    fs::create_dir_all(folder).map_err(|error| format!("创建工程文件夹失败：{error}"))?;
    if app_state.project_root != folder {
        copy_directory_contents(
            &app_state.project_root.join("assets"),
            &folder.join("assets"),
        )?;
        copy_directory_contents(
            &app_state.project_root.join("plugins"),
            &folder.join("plugins"),
        )?;
        copy_directory_contents(
            &app_state.project_root.join("Content"),
            &folder.join("Content"),
        )?;
    }
    app_state.project_root = folder.to_path_buf();
    app_state.project_file_path = Some(folder.to_path_buf());
    app_state.store_active_scene();
    let project = Slide2dProject {
        slide2d_engine: PROJECT_MAGIC.to_owned(),
        format_version: PROJECT_FORMAT_VERSION,
        name: folder
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("Slide2D工程")
            .to_owned(),
        startup_scene_name: app_state.startup_scene_name.clone(),
        active_scene_name: app_state.active_scene_name().to_owned(),
        scenes: app_state.project_scenes.clone(),
        scene_categories: app_state.scene_categories.clone(),
        global_variables: app_state.global_variables.clone(),
        assets: Vec::new(),
        asset_directories: collect_directories(&folder.join("Content"), folder)?,
        enabled_plugins: app_state.plugin_registry.enabled_ids(),
        performance_settings: app_state.performance_settings.clone(),
        assistant_settings: app_state.assistant_settings.clone(),
    };
    let json = serde_json::to_vec_pretty(&project)
        .map_err(|error| format!("生成工程清单失败：{error}"))?;
    atomic_write(&folder.join(PROJECT_MANIFEST_NAME), &json)
        .map_err(|error| format!("写入工程清单失败：{error}"))?;
    write_scene_files(app_state)?;
    let package_name = project_package_name(folder);
    let package_path = folder.join(&package_name);
    let folder_identity = app_state.project_file_path.clone();
    save_project(app_state, &package_path)?;
    app_state.project_file_path = folder_identity;
    write_verification_file(folder, &package_name, &package_path)?;
    Ok(())
}

/// 从用户选择的工程文件夹恢复全部场景和全局变量。
pub fn open_project_folder(folder: &Path) -> Result<AppState, String> {
    let verification_path = folder.join(PROJECT_VERIFICATION_NAME);
    if verification_path.exists() {
        return open_verified_project_folder(folder, &verification_path);
    }

    // 兼容升级前只有slide2d.project.json的工程，首次保存后会自动补齐验证文件和主工程包。
    let manifest_path = folder.join(PROJECT_MANIFEST_NAME);
    if !manifest_path.exists() {
        return Err(format!(
            "所选文件夹不是Slide2D工程，缺少 {}",
            PROJECT_MANIFEST_NAME
        ));
    }
    let bytes = fs::read(&manifest_path).map_err(|error| format!("读取工程清单失败：{error}"))?;
    let project = parse_project(&bytes).map_err(|error| format!("解析工程清单失败：{error}"))?;
    if project.scenes.is_empty() {
        return Err("工程中没有可用场景".to_owned());
    }
    let active_index = project
        .scenes
        .iter()
        .position(|scene| scene.name == project.active_scene_name)
        .unwrap_or(0);
    let mut state = AppState::from_scene(project.scenes[active_index].clone());
    state.project_scenes = project.scenes;
    state.scene_categories = project.scene_categories;
    state.active_scene_index = active_index;
    state.startup_scene_name = project.startup_scene_name;
    state.global_variables = project.global_variables;
    state.project_file_path = Some(folder.to_path_buf());
    state.project_root = folder.to_path_buf();
    state.plugin_registry = PluginRegistry::load(folder.to_path_buf(), &project.enabled_plugins);
    state.performance_settings = project.performance_settings;
    state.assistant_settings = project.assistant_settings;
    state.grid_size = state.assistant_settings.grid_size;
    Ok(state)
}

/// 校验工程文件夹中的验证文件和.slide2d主文件，再恢复编辑状态。
fn open_verified_project_folder(
    folder: &Path,
    verification_path: &Path,
) -> Result<AppState, String> {
    let verification_bytes =
        fs::read(verification_path).map_err(|error| format!("读取工程验证文件失败：{error}"))?;
    let verification: ProjectVerification = serde_json::from_slice(&verification_bytes)
        .map_err(|error| format!("解析工程验证文件失败：{error}"))?;
    if verification.magic != PROJECT_MAGIC {
        return Err("工程验证失败：标识不是SLIDE2D_PROJECT".to_owned());
    }
    if verification.format_version != PROJECT_FORMAT_VERSION {
        return Err(format!(
            "工程验证失败：格式版本{}不受支持，当前版本为{}",
            verification.format_version, PROJECT_FORMAT_VERSION
        ));
    }
    let package_relative = safe_relative_path(&verification.project_file)?;
    if package_relative.components().count() != 1
        || package_relative
            .extension()
            .and_then(|value| value.to_str())
            != Some("slide2d")
    {
        return Err("工程验证失败：主工程文件必须是文件夹根目录中的.slide2d文件".to_owned());
    }
    let package_path = folder.join(package_relative);
    let package_bytes =
        fs::read(&package_path).map_err(|error| format!("读取主工程文件失败：{error}"))?;
    if package_bytes.len() as u64 != verification.file_size {
        return Err("工程验证失败：主工程文件大小不一致，文件可能已损坏".to_owned());
    }
    let actual_checksum = checksum_hex(&package_bytes);
    if actual_checksum != verification.checksum {
        return Err("工程验证失败：主工程文件校验值不一致，文件可能已损坏".to_owned());
    }
    let project = parse_project(&package_bytes)
        .map_err(|error| format!("解析主.slide2d工程文件失败：{error}"))?;
    restore_project_to_folder(project, folder)
}

/// 根据.slide2d数据恢复文件夹工程状态，并确保包内素材已经解压。
fn restore_project_to_folder(project: Slide2dProject, folder: &Path) -> Result<AppState, String> {
    if project.scenes.is_empty() {
        return Err("工程中没有可用场景".to_owned());
    }
    for directory in &project.asset_directories {
        let relative = safe_relative_path(&directory.relative_path)?;
        fs::create_dir_all(folder.join(relative))
            .map_err(|error| format!("恢复资源文件夹失败：{error}"))?;
    }
    for asset in &project.assets {
        let relative = safe_relative_path(&asset.relative_path)?;
        let output = folder.join(relative);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("恢复素材目录失败：{error}"))?;
        }
        fs::write(output, &asset.bytes).map_err(|error| format!("恢复工程素材失败：{error}"))?;
    }
    let active_index = project
        .scenes
        .iter()
        .position(|scene| scene.name == project.active_scene_name)
        .unwrap_or(0);
    let mut state = AppState::from_scene(project.scenes[active_index].clone());
    state.project_scenes = project.scenes;
    state.scene_categories = project.scene_categories;
    state.active_scene_index = active_index;
    state.startup_scene_name = project.startup_scene_name;
    state.global_variables = project.global_variables;
    state.project_file_path = Some(folder.to_path_buf());
    state.project_root = folder.to_path_buf();
    state.plugin_registry = PluginRegistry::load(folder.to_path_buf(), &project.enabled_plugins);
    state.performance_settings = project.performance_settings;
    state.assistant_settings = project.assistant_settings;
    state.grid_size = state.assistant_settings.grid_size;
    write_scene_files(&state)?;
    Ok(state)
}

/// 根据工程文件夹名称生成便于识别和导入的.slide2d主文件名。
fn project_package_name(folder: &Path) -> String {
    let name = folder
        .file_name()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Slide2DProject");
    format!("{}.slide2d", sanitize_file_name(name, 0))
}

/// 为主.slide2d文件计算信息并写入验证文件。
fn write_verification_file(
    folder: &Path,
    package_name: &str,
    package_path: &Path,
) -> Result<(), String> {
    let package_bytes =
        fs::read(package_path).map_err(|error| format!("读取待验证主工程文件失败：{error}"))?;
    let verification = ProjectVerification {
        magic: PROJECT_MAGIC.to_owned(),
        format_version: PROJECT_FORMAT_VERSION,
        project_file: package_name.to_owned(),
        file_size: package_bytes.len() as u64,
        checksum: checksum_hex(&package_bytes),
    };
    let json = serde_json::to_vec_pretty(&verification)
        .map_err(|error| format!("生成工程验证文件失败：{error}"))?;
    atomic_write(&folder.join(PROJECT_VERIFICATION_NAME), &json)
        .map_err(|error| format!("写入工程验证文件失败：{error}"))
}

/// 使用FNV-1a生成稳定的64位文件校验值，不额外增加依赖。
fn checksum_hex(bytes: &[u8]) -> String {
    let mut checksum = 0xcbf29ce484222325_u64;
    for byte in bytes {
        checksum ^= *byte as u64;
        checksum = checksum.wrapping_mul(0x100000001b3);
    }
    format!("{checksum:016x}")
}

/// 将素材目录递归复制到另存为的工程文件夹。
fn copy_directory_contents(source: &Path, target: &Path) -> Result<(), String> {
    if !source.exists() {
        return Ok(());
    }
    fs::create_dir_all(target).map_err(|error| format!("创建素材目录失败：{error}"))?;
    for entry in fs::read_dir(source).map_err(|error| format!("读取素材目录失败：{error}"))?
    {
        let entry = entry.map_err(|error| format!("读取素材条目失败：{error}"))?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() {
            copy_directory_contents(&source_path, &target_path)?;
        } else {
            fs::copy(&source_path, &target_path)
                .map_err(|error| format!("复制素材失败：{error}"))?;
        }
    }
    Ok(())
}

/// 打开.slide2d文件，并将内嵌素材恢复到工程专属工作目录。
pub fn open_project(path: &Path) -> Result<AppState, String> {
    let bytes = fs::read(path).map_err(|error| format!("读取工程文件失败：{error}"))?;
    let project = parse_project(&bytes).map_err(|error| format!("解析工程文件失败：{error}"))?;
    if project.scenes.is_empty() {
        return Err("工程中没有可用场景".to_owned());
    }
    let project_root = workspace_path(path);
    fs::create_dir_all(&project_root).map_err(|error| format!("创建工程工作目录失败：{error}"))?;
    for directory in &project.asset_directories {
        let relative = safe_relative_path(&directory.relative_path)?;
        fs::create_dir_all(project_root.join(relative))
            .map_err(|error| format!("恢复资源文件夹失败：{error}"))?;
    }
    for asset in &project.assets {
        let relative = safe_relative_path(&asset.relative_path)?;
        let output = project_root.join(relative);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("创建素材目录失败：{error}"))?;
        }
        fs::write(output, &asset.bytes).map_err(|error| format!("恢复工程素材失败：{error}"))?;
    }
    let active_index = project
        .scenes
        .iter()
        .position(|scene| scene.name == project.active_scene_name)
        .unwrap_or(0);
    let mut state = AppState::from_scene(project.scenes[active_index].clone());
    state.project_scenes = project.scenes;
    state.scene_categories = project.scene_categories;
    state.active_scene_index = active_index;
    state.startup_scene_name = project.startup_scene_name;
    state.global_variables = project.global_variables;
    state.project_file_path = Some(path.to_path_buf());
    state.project_root = project_root;
    state.plugin_registry =
        PluginRegistry::load(state.project_root.clone(), &project.enabled_plugins);
    state.performance_settings = project.performance_settings;
    state.assistant_settings = project.assistant_settings;
    state.grid_size = state.assistant_settings.grid_size;
    write_scene_files(&state)?;
    Ok(state)
}

/// 返回工程文件旁边的专属解包目录。
pub fn workspace_path(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("project");
    path.parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".{stem}_slide2d"))
}

/// 将多场景分别写入scenes目录，方便Runtime和用户检查场景文件。
pub fn write_scene_files(app_state: &AppState) -> Result<(), String> {
    let directory = app_state.project_root.join("scenes");
    fs::create_dir_all(&directory).map_err(|error| format!("创建场景目录失败：{error}"))?;
    for scene in &app_state.project_scenes {
        let json = serde_json::to_vec_pretty(scene)
            .map_err(|error| format!("生成场景JSON失败：{error}"))?;
        atomic_write(&directory.join(format!("{}.json", scene.scene_id)), &json)
            .map_err(|error| format!("写入场景文件失败：{error}"))?;
    }
    Ok(())
}

/// 严格读取当前格式，或显式识别并迁移缺少magic的version 0旧工程。
fn parse_project(bytes: &[u8]) -> Result<Slide2dProject, String> {
    let mut value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| "工程根节点必须是JSON对象".to_owned())?;
    match object.get("slide2d_engine") {
        Some(_) => {}
        None => {
            let legacy_version = match object.get("format_version") {
                None => 0,
                Some(value) => value
                    .as_u64()
                    .ok_or_else(|| "legacy工程格式版本必须是整数0".to_owned())?,
            };
            if legacy_version != 0 {
                return Err("缺少Slide2D标识的工程仅允许使用legacy version 0".to_owned());
            }
            object.insert(
                "slide2d_engine".to_owned(),
                serde_json::Value::String(PROJECT_MAGIC.to_owned()),
            );
            object.insert(
                "format_version".to_owned(),
                serde_json::Value::from(PROJECT_FORMAT_VERSION),
            );
        }
    }
    let mut project: Slide2dProject =
        serde_json::from_value(value).map_err(|error| error.to_string())?;
    validate_project_identity(&project)?;
    migrate_scene_ids(&mut project.scenes);
    Ok(project)
}

/// 为旧工程中的空、重复或不安全场景ID分配确定且互不冲突的新ID。
fn migrate_scene_ids(scenes: &mut [Scene]) {
    let mut used = HashSet::new();
    let mut next_number = 1_u64;
    for scene in scenes {
        let valid = !scene.scene_id.is_empty()
            && scene.scene_id.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
            });
        if valid && used.insert(scene.scene_id.clone()) {
            continue;
        }
        loop {
            let candidate = format!("scene_{next_number:06}");
            next_number += 1;
            if used.insert(candidate.clone()) {
                scene.scene_id = candidate;
                break;
            }
        }
    }
}

/// 在目标文件同目录持久化临时文件，并通过备份重命名实现Windows安全替换。
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| format!("创建目标目录失败：{error}"))?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "目标文件名无效".to_owned())?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary_path = parent.join(format!(".{file_name}.{nonce}.tmp"));
    let backup_path = parent.join(format!(".{file_name}.{nonce}.bak"));

    let write_result = (|| -> Result<(), String> {
        let mut temporary =
            File::create(&temporary_path).map_err(|error| format!("创建临时文件失败：{error}"))?;
        temporary
            .write_all(bytes)
            .map_err(|error| format!("写入临时文件失败：{error}"))?;
        temporary
            .flush()
            .map_err(|error| format!("刷新临时文件失败：{error}"))?;
        temporary
            .sync_all()
            .map_err(|error| format!("同步临时文件失败：{error}"))?;
        drop(temporary);

        let had_original = path.exists();
        if had_original {
            fs::rename(path, &backup_path).map_err(|error| format!("备份旧文件失败：{error}"))?;
        }
        if let Err(error) = fs::rename(&temporary_path, path) {
            if had_original {
                let _ = fs::rename(&backup_path, path);
            }
            return Err(format!("替换目标文件失败：{error}"));
        }
        if had_original {
            let _ = fs::remove_file(&backup_path);
        }
        Ok(())
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    write_result
}

/// 递归收集assets目录中的所有普通文件。
fn collect_assets(directory: &Path, project_root: &Path) -> Result<Vec<PackedAsset>, String> {
    let mut assets = Vec::new();
    if !directory.exists() {
        return Ok(assets);
    }
    for entry in fs::read_dir(directory).map_err(|error| format!("扫描素材目录失败：{error}"))?
    {
        let path = entry
            .map_err(|error| format!("读取素材条目失败：{error}"))?
            .path();
        if path.is_dir() {
            assets.extend(collect_assets(&path, project_root)?);
        } else {
            let relative = path
                .strip_prefix(project_root)
                .map_err(|_| "素材路径不在工程目录内".to_owned())?;
            assets.push(PackedAsset {
                relative_path: relative.to_string_lossy().replace('\\', "/"),
                bytes: fs::read(&path).map_err(|error| format!("读取素材失败：{error}"))?,
            });
        }
    }
    Ok(assets)
}

/// 递归收集资源目录本身，确保用户创建的空目录也能写入.slide2d。
fn collect_directories(
    directory: &Path,
    project_root: &Path,
) -> Result<Vec<PackedDirectory>, String> {
    let mut directories = Vec::new();
    if !directory.exists() {
        return Ok(directories);
    }
    let relative = directory
        .strip_prefix(project_root)
        .map_err(|_| "资源目录不在工程目录内".to_owned())?;
    directories.push(PackedDirectory {
        relative_path: relative.to_string_lossy().replace('\\', "/"),
    });
    for entry in fs::read_dir(directory).map_err(|error| format!("扫描资源目录失败：{error}"))?
    {
        let path = entry
            .map_err(|error| format!("读取资源目录失败：{error}"))?
            .path();
        if path.is_dir() {
            directories.extend(collect_directories(&path, project_root)?);
        }
    }
    Ok(directories)
}

/// 拒绝绝对路径和父目录跳转，避免工程包写出专属目录。
fn safe_relative_path(value: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(format!("工程包含不安全素材路径：{value}"));
    }
    Ok(path)
}

/// 将场景名称转换为Windows可用文件名。
fn sanitize_file_name(name: &str, index: usize) -> String {
    let value: String = name
        .chars()
        .map(|character| {
            if matches!(
                character,
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
            ) {
                '_'
            } else {
                character
            }
        })
        .collect();
    if value.trim().is_empty() {
        format!("scene_{}", index + 1)
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证工程文件可以完整保存并恢复场景、全局变量和素材字节。
    #[test]
    fn project_round_trip_restores_all_data() {
        let unique = format!(
            "slide2d_project_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("系统时间应有效")
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        fs::create_dir_all(root.join("assets/images")).expect("应创建测试素材目录");
        fs::write(root.join("assets/images/test.bin"), [1_u8, 2, 3]).expect("应写入测试素材");
        let project_path = root.join("complete.slide2d");
        let mut state = AppState::new();
        state.project_root = root.clone();
        state.global_variables.insert("score".to_owned(), 42.0);
        state.add_scene("关卡2".to_owned());
        save_project(&mut state, &project_path).expect("工程应保存成功");

        let loaded = open_project(&project_path).expect("工程应打开成功");
        assert_eq!(loaded.project_scenes.len(), 2);
        assert_eq!(loaded.global_variables.get("score"), Some(&42.0));
        assert_eq!(
            fs::read(loaded.project_root.join("assets/images/test.bin")).expect("素材应恢复"),
            vec![1, 2, 3]
        );
        let _ = fs::remove_dir_all(root);
    }

    /// 验证文件夹工程会生成验证文件和可导入的.slide2d主文件。
    #[test]
    fn project_folder_contains_verification_and_package() {
        let root = unique_test_directory("folder_structure");
        let mut state = AppState::new();
        state.project_root = root.clone();
        state.global_variables.insert("health".to_owned(), 100.0);
        save_project_folder(&mut state, &root).expect("文件夹工程应保存成功");

        let verification_path = root.join(PROJECT_VERIFICATION_NAME);
        assert!(verification_path.exists());
        let verification: ProjectVerification =
            serde_json::from_slice(&fs::read(&verification_path).expect("应读取验证文件"))
                .expect("验证文件应为有效JSON");
        assert_eq!(verification.magic, PROJECT_MAGIC);
        assert!(root.join(&verification.project_file).exists());

        let loaded = open_project_folder(&root).expect("验证通过后应打开工程");
        assert_eq!(loaded.global_variables.get("health"), Some(&100.0));
        let _ = fs::remove_dir_all(root);
    }

    /// 验证主.slide2d文件被修改后会拒绝打开。
    #[test]
    fn tampered_project_package_is_rejected() {
        let root = unique_test_directory("tampered_package");
        let mut state = AppState::new();
        state.project_root = root.clone();
        save_project_folder(&mut state, &root).expect("文件夹工程应保存成功");
        let verification: ProjectVerification = serde_json::from_slice(
            &fs::read(root.join(PROJECT_VERIFICATION_NAME)).expect("应读取验证文件"),
        )
        .expect("验证文件应有效");
        fs::write(root.join(verification.project_file), b"damaged").expect("应篡改测试工程包");

        let error = match open_project_folder(&root) {
            Ok(_) => panic!("损坏工程不应打开成功"),
            Err(error) => error,
        };
        assert!(error.contains("验证失败"));
        let _ = fs::remove_dir_all(root);
    }

    /// 验证.slide2d工程包会保存并恢复用户创建的多级空文件夹。
    #[test]
    fn empty_resource_folders_are_restored() {
        let root = unique_test_directory("empty_folders");
        fs::create_dir_all(root.join("Content/Actor/Enemies/Bosses")).expect("应创建测试空目录");
        let package = root.join("empty.slide2d");
        let mut state = AppState::new();
        state.project_root = root.clone();
        save_project(&mut state, &package).expect("工程包应保存成功");
        fs::remove_dir_all(root.join("Content")).expect("应清理原始Content目录");

        let restored = open_project(&package).expect("工程包应打开成功");
        assert!(restored
            .project_root
            .join("Content/Actor/Enemies/Bosses")
            .is_dir());
        let restored_root = restored.project_root.clone();
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(restored_root);
    }

    #[test]
    fn project_identity_is_strict_and_version_zero_is_migrated_explicitly() {
        let state = AppState::new();
        let mut current = serde_json::to_value(Slide2dProject {
            slide2d_engine: PROJECT_MAGIC.to_owned(),
            format_version: PROJECT_FORMAT_VERSION,
            name: "test".to_owned(),
            startup_scene_name: "场景1".to_owned(),
            active_scene_name: "场景1".to_owned(),
            scenes: state.project_scenes.clone(),
            scene_categories: Vec::new(),
            global_variables: HashMap::new(),
            assets: Vec::new(),
            asset_directories: Vec::new(),
            enabled_plugins: HashSet::new(),
            performance_settings: PerformanceSettings::new(),
            assistant_settings: AssistantSettings::new(),
        })
        .expect("工程应可序列化");

        current.as_object_mut().unwrap().remove("slide2d_engine");
        assert!(parse_project(&serde_json::to_vec(&current).unwrap()).is_err());

        current["format_version"] = serde_json::Value::from(0);
        let migrated = parse_project(&serde_json::to_vec(&current).unwrap())
            .expect("version 0旧工程应显式迁移");
        assert_eq!(migrated.slide2d_engine, PROJECT_MAGIC);
        assert_eq!(migrated.format_version, PROJECT_FORMAT_VERSION);

        current["slide2d_engine"] = serde_json::Value::String("WRONG".to_owned());
        assert!(parse_project(&serde_json::to_vec(&current).unwrap()).is_err());
    }

    #[test]
    fn duplicate_and_empty_scene_ids_are_migrated_and_used_as_file_names() {
        let root = unique_test_directory("scene_ids");
        let mut state = AppState::new();
        state.project_root = root.clone();
        state.project_scenes[0].scene_id.clear();
        state.project_scenes.push(Scene::empty("Second"));
        state.project_scenes[1].scene_id = String::new();

        migrate_scene_ids(&mut state.project_scenes);
        assert!(!state.project_scenes[0].scene_id.is_empty());
        assert_ne!(
            state.project_scenes[0].scene_id,
            state.project_scenes[1].scene_id
        );
        write_scene_files(&state).expect("场景文件应写入");
        for scene in &state.project_scenes {
            assert!(root
                .join("scenes")
                .join(format!("{}.json", scene.scene_id))
                .exists());
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn atomic_write_replaces_existing_file() {
        let root = unique_test_directory("atomic_write");
        fs::create_dir_all(&root).expect("应创建目录");
        let path = root.join("data.json");
        fs::write(&path, b"old").expect("应写入旧文件");

        atomic_write(&path, b"new").expect("应安全替换文件");

        assert_eq!(fs::read(&path).unwrap(), b"new");
        let _ = fs::remove_dir_all(root);
    }

    /// 创建不会与其他测试冲突的临时目录。
    fn unique_test_directory(label: &str) -> PathBuf {
        let unique = format!(
            "slide2d_{label}_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("系统时间应有效")
                .as_nanos()
        );
        std::env::temp_dir().join(unique)
    }
}
