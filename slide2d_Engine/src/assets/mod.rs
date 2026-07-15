//! 项目资源素材管理。

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::app_state::{ColliderConfig, GameObject};
use crate::blueprint::model::Blueprint;
use crate::localization::tr;
use crate::plugins::PluginResourceDefinition;

/// 自动扫描assets目录的时间间隔。
const ASSET_SCAN_INTERVAL: Duration = Duration::from_secs(120);

/// 资源树中的文件或文件夹。
#[derive(Clone)]
pub enum AssetEntry {
    Folder {
        name: String,
        path: PathBuf,
        children: Vec<AssetEntry>,
    },
    File {
        name: String,
        path: PathBuf,
    },
}

/// egui内部拖拽图片时携带的数据。
#[derive(Clone)]
pub struct ImageAssetDragPayload {
    pub path: PathBuf,
}

/// egui内部拖拽音效时携带的数据。
#[derive(Clone)]
pub struct AudioAssetDragPayload {
    pub path: PathBuf,
}

/// Actor资源拖到画布时携带的模板文件路径。
#[derive(Clone)]
pub struct ActorAssetDragPayload {
    pub path: PathBuf,
}

/// 动画资源拖到画布时携带的动画文件路径。
#[derive(Clone)]
pub struct AnimationAssetDragPayload {
    pub path: PathBuf,
}

/// 独立Actor模板资源，保存可复用属性，不保存场景实例ID和坐标。
#[derive(Clone, Serialize, Deserialize)]
pub struct ActorAsset {
    #[serde(default = "actor_format")]
    pub slide2d_engine: String,
    pub name: String,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub image_path: String,
    #[serde(default)]
    pub audio_path: String,
    #[serde(default)]
    pub animation_path: String,
    #[serde(default = "default_true")]
    pub animation_playing: bool,
    #[serde(default)]
    pub collider: Option<ColliderConfig>,
    #[serde(default)]
    pub blueprint: Blueprint,
    #[serde(default)]
    pub variables: std::collections::HashMap<String, f32>,
}

impl ActorAsset {
    /// 从场景物体创建不含实例坐标和ID的Actor模板。
    pub fn from_game_object(name: String, object: &GameObject) -> Self {
        Self {
            slide2d_engine: actor_format(),
            name,
            width: object.width,
            height: object.height,
            image_path: object.image_path.clone(),
            audio_path: object.audio_path.clone(),
            animation_path: object.animation_path.clone(),
            animation_playing: object.animation_playing,
            collider: object.collider.clone(),
            blueprint: object.blueprint.clone(),
            variables: object.variables.clone(),
        }
    }

    /// 从.s2actor文件读取Actor模板并验证Slide2D标识。
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = fs::read(path).map_err(|error| format!("读取Slide2D Actor失败：{error}"))?;
        let actor: Self = serde_json::from_slice(&bytes)
            .map_err(|error| format!("解析Slide2D Actor失败：{error}"))?;
        if actor.slide2d_engine != actor_format() {
            return Err("Actor文件缺少有效的SLIDE2D_ACTOR标识".to_owned());
        }
        Ok(actor)
    }

    /// 将Actor模板保存为格式化JSON。
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| format!("生成Slide2D Actor失败：{error}"))?;
        fs::write(path, bytes).map_err(|error| format!("保存Slide2D Actor失败：{error}"))
    }
}

/// 返回Actor资源固定格式标识。
fn actor_format() -> String {
    "SLIDE2D_ACTOR".to_owned()
}

/// 旧Actor资源缺少动画播放字段时默认播放。
fn default_true() -> bool {
    true
}

/// 用户当前要在哪一个资源分类下创建文件夹。
#[derive(Clone, Copy, PartialEq)]
pub enum AssetCategory {
    Actor,
    Blueprint,
    Animation,
    AssetsTextures,
    AssetsAudio,
    AssetsTilesets,
    AssetsUi,
}

/// 文件导入后所属的资源类型。
#[derive(Clone, Copy, PartialEq)]
pub enum AssetKind {
    Image,
    Audio,
    Animation,
    Tileset,
}

/// 保存一次导入结果，调用方可根据类型决定是否在画布创建物体。
pub struct ImportedAsset {
    pub kind: AssetKind,
    pub path: PathBuf,
}

/// 保存项目资源目录及资源面板临时状态。
pub struct AssetLibrary {
    /// Unreal风格新资源根目录Content。
    pub content_root: PathBuf,
    /// 旧版assets资源根目录，只用于兼容扫描旧工程。
    pub root: PathBuf,
    pub new_folder_name: String,
    pub new_folder_category: AssetCategory,
    /// 在任意展开目录右键选择后，新文件夹会创建在这里。
    pub new_folder_parent: Option<PathBuf>,
    /// 右键重命名窗口当前操作的资源路径。
    pub rename_target: Option<PathBuf>,
    /// 右键重命名窗口中的新名称。
    pub rename_text: String,
    /// 是否对新导入PNG执行Slide2D自动压缩。
    pub automatic_image_compression: bool,
    actor_entries: Vec<AssetEntry>,
    image_entries: Vec<AssetEntry>,
    audio_entries: Vec<AssetEntry>,
    animation_entries: Vec<AssetEntry>,
    tileset_entries: Vec<AssetEntry>,
    ui_entries: Vec<AssetEntry>,
    blueprint_entries: Vec<AssetEntry>,
    last_scan_time: Instant,
}

impl AssetLibrary {
    /// 创建资源库，并确保assets/images和assets/audio目录存在。
    pub fn new(project_root: PathBuf) -> Result<Self, String> {
        let root = project_root.join("assets");
        let content_root = project_root.join("Content");
        for directory in [
            "Actor",
            "Blueprint",
            "Animation",
            "Assets/Textures",
            "Assets/Audio",
            "Assets/TileSets",
            "Assets/UI",
        ] {
            fs::create_dir_all(content_root.join(directory))
                .map_err(|error| format!("创建Slide2D Content/{directory}目录失败：{error}"))?;
        }
        fs::create_dir_all(root.join("images"))
            .map_err(|error| format!("创建图片资源目录失败：{error}"))?;
        fs::create_dir_all(root.join("audio"))
            .map_err(|error| format!("创建音效资源目录失败：{error}"))?;
        fs::create_dir_all(root.join("animations"))
            .map_err(|error| format!("创建动画资源目录失败：{error}"))?;
        fs::create_dir_all(root.join("tilesets"))
            .map_err(|error| format!("创建瓦片集目录失败：{error}"))?;
        let mut library = Self {
            content_root,
            root,
            new_folder_name: String::new(),
            new_folder_category: AssetCategory::Actor,
            new_folder_parent: None,
            rename_target: None,
            rename_text: String::new(),
            automatic_image_compression: true,
            actor_entries: Vec::new(),
            image_entries: Vec::new(),
            audio_entries: Vec::new(),
            animation_entries: Vec::new(),
            tileset_entries: Vec::new(),
            ui_entries: Vec::new(),
            blueprint_entries: Vec::new(),
            last_scan_time: Instant::now(),
        };
        // 每次打开项目立即扫描一次，确保用户手动放进assets的素材可见。
        library.refresh();
        Ok(library)
    }

    /// 返回图片素材根目录。
    pub fn image_root(&self) -> PathBuf {
        self.content_root.join("Assets/Textures")
    }

    /// 返回音效素材根目录。
    pub fn audio_root(&self) -> PathBuf {
        self.content_root.join("Assets/Audio")
    }

    /// 返回动画素材根目录。
    pub fn animation_root(&self) -> PathBuf {
        self.content_root.join("Animation")
    }

    /// 返回瓦片集资源根目录。
    pub fn tileset_root(&self) -> PathBuf {
        self.content_root.join("Assets/TileSets")
    }

    /// 返回标准纹理目录。
    pub fn texture_root(&self) -> PathBuf {
        self.content_root.join("Assets/Textures")
    }

    /// 返回标准UI资源目录。
    pub fn ui_root(&self) -> PathBuf {
        self.content_root.join("Assets/UI")
    }

    /// 返回标准蓝图资源目录。
    pub fn blueprint_root(&self) -> PathBuf {
        self.content_root.join("Blueprint")
    }

    /// 返回固定Actor分类根目录。
    pub fn actor_root(&self) -> PathBuf {
        self.content_root.join("Actor")
    }

    /// 返回Actor资源树缓存。
    pub fn scan_actors(&self) -> &[AssetEntry] {
        &self.actor_entries
    }

    /// 扫描图片资源树。
    pub fn scan_images(&self) -> &[AssetEntry] {
        &self.image_entries
    }

    /// 扫描音效资源树。
    pub fn scan_audio(&self) -> &[AssetEntry] {
        &self.audio_entries
    }

    /// 返回动画资源树缓存。
    pub fn scan_animations(&self) -> &[AssetEntry] {
        &self.animation_entries
    }

    /// 返回瓦片集资源树缓存。
    pub fn scan_tilesets(&self) -> &[AssetEntry] {
        &self.tileset_entries
    }

    /// 返回UI资源树缓存。
    pub fn scan_ui(&self) -> &[AssetEntry] {
        &self.ui_entries
    }

    /// 返回蓝图资源树缓存。
    pub fn scan_blueprints(&self) -> &[AssetEntry] {
        &self.blueprint_entries
    }

    /// 如果距离上次扫描已经超过两分钟，就重新扫描全部资源目录。
    pub fn refresh_if_due(&mut self) -> bool {
        if self.last_scan_time.elapsed() < ASSET_SCAN_INTERVAL {
            return false;
        }
        self.refresh();
        true
    }

    /// 立即递归扫描图片和音效目录，并替换资源树缓存。
    pub fn refresh(&mut self) {
        self.actor_entries = scan_directory(&self.actor_root(), &["s2actor"]);
        self.image_entries = scan_directories(
            &[
                self.image_root(),
                self.content_root.join("Sprites"),
                self.content_root.join("Textures"),
                self.root.join("images"),
            ],
            &["png"],
        );
        self.audio_entries = scan_directories(
            &[
                self.audio_root(),
                self.content_root.join("Audio"),
                self.root.join("audio"),
            ],
            &["wav", "ogg", "mp3", "flac"],
        );
        self.animation_entries = scan_directories(
            &[
                self.animation_root(),
                self.content_root.join("Animations"),
                self.root.join("animations"),
            ],
            &["s2anim"],
        );
        self.tileset_entries = scan_directories(
            &[
                self.tileset_root(),
                self.content_root.join("TileMaps"),
                self.root.join("tilesets"),
            ],
            &["s2tileset"],
        );
        self.ui_entries =
            scan_directories(&[self.ui_root(), self.content_root.join("UI")], &["png"]);
        self.blueprint_entries = scan_directories(
            &[self.blueprint_root(), self.content_root.join("Blueprints")],
            &["json", "s2blueprint"],
        );
        self.last_scan_time = Instant::now();
    }

    /// 打开一个统一文件对话框，自动识别并导入图片或音频文件。
    pub fn import_files(&self) -> Result<Vec<ImportedAsset>, String> {
        let source_paths = match rfd::FileDialog::new()
            .set_title(tr("dialog.import_content"))
            .add_filter(
                tr("filter.supported_assets"),
                &["png", "wav", "ogg", "mp3", "flac", "s2anim", "s2tileset"],
            )
            .add_filter(tr("filter.images"), &["png"])
            .add_filter(tr("filter.audio"), &["wav", "ogg", "mp3", "flac"])
            .add_filter(tr("filter.animations"), &["s2anim"])
            .add_filter(tr("filter.tilesets"), &["s2tileset"])
            .pick_files()
        {
            Some(paths) => paths,
            None => return Ok(Vec::new()),
        };

        let mut imported_assets = Vec::new();
        for source_path in source_paths {
            imported_assets.push(self.import_file(&source_path)?);
        }
        Ok(imported_assets)
    }

    /// 打开PNG专用文件对话框，可一次选择并导入多张图片。
    pub fn import_png_images(&self) -> Result<Vec<ImportedAsset>, String> {
        let source_paths = match rfd::FileDialog::new()
            .set_title(tr("dialog.import_sprites"))
            .add_filter(tr("filter.png"), &["png"])
            .pick_files()
        {
            Some(paths) => paths,
            None => return Ok(Vec::new()),
        };

        let mut imported_assets = Vec::new();
        for source_path in source_paths {
            let imported_asset = self.import_file(&source_path)?;
            if imported_asset.kind == AssetKind::Image {
                imported_assets.push(imported_asset);
            }
        }
        Ok(imported_assets)
    }

    /// 检查文件格式，自动复制到图片或音效资源目录。
    pub fn import_file(&self, source_path: &Path) -> Result<ImportedAsset, String> {
        let kind = classify_file(source_path).ok_or_else(|| {
            format!(
                "不支持的Slide2D素材格式：{}。支持PNG、WAV、OGG、MP3、FLAC、S2ANIM、S2TILESET。",
                source_path.display()
            )
        })?;
        let file_name = source_path.file_name().ok_or("无法读取素材文件名")?;
        let destination_directory = match kind {
            AssetKind::Image => self.image_root(),
            AssetKind::Audio => self.audio_root(),
            AssetKind::Animation => self.animation_root(),
            AssetKind::Tileset => self.tileset_root(),
        };

        // 如果拖入的文件已经在目标资源目录中，就直接复用，不重复复制。
        let destination = if source_path.parent() == Some(destination_directory.as_path()) {
            source_path.to_path_buf()
        } else {
            let destination = unique_destination(&destination_directory, file_name);
            if kind == AssetKind::Image && self.automatic_image_compression {
                optimize_imported_png(source_path, &destination)?;
            } else {
                fs::copy(source_path, &destination)
                    .map_err(|error| format!("复制素材到assets目录失败：{error}"))?;
            }
            destination
        };
        Ok(ImportedAsset {
            kind,
            path: destination,
        })
    }

    /// 在当前选择的图片或音效分类中创建文件夹。
    pub fn create_folder(&mut self) -> Result<(), String> {
        let folder_name = self.new_folder_name.trim();
        if folder_name.is_empty() {
            return Err("文件夹名称不能为空".to_owned());
        }
        if folder_name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
            return Err("文件夹名称包含Windows不允许的字符".to_owned());
        }
        let parent = match self.new_folder_category {
            AssetCategory::Actor => self.actor_root(),
            AssetCategory::Blueprint => self.blueprint_root(),
            AssetCategory::Animation => self.animation_root(),
            AssetCategory::AssetsTextures => self.texture_root(),
            AssetCategory::AssetsAudio => self.audio_root(),
            AssetCategory::AssetsTilesets => self.tileset_root(),
            AssetCategory::AssetsUi => self.ui_root(),
        };
        let parent = self.new_folder_parent.clone().unwrap_or(parent);
        fs::create_dir(parent.join(folder_name))
            .map_err(|error| format!("创建资源文件夹失败：{error}"))?;
        self.new_folder_name.clear();
        self.refresh();
        Ok(())
    }
}

/// 对导入PNG执行无损重新编码，大于2048像素时按比例缩小以适配2D游戏。
fn optimize_imported_png(source: &Path, destination: &Path) -> Result<(), String> {
    use image::GenericImageView;
    let image = image::open(source).map_err(|error| format!("读取待压缩PNG失败：{error}"))?;
    let (width, height) = image.dimensions();
    let maximum = 2048_u32;
    let optimized = if width > maximum || height > maximum {
        let scale = maximum as f32 / width.max(height) as f32;
        image.resize(
            (width as f32 * scale).round().max(1.0) as u32,
            (height as f32 * scale).round().max(1.0) as u32,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        image
    };
    optimized
        .save_with_format(destination, image::ImageFormat::Png)
        .map_err(|error| format!("Slide2D无损压缩PNG失败：{error}"))
}

/// 复制资源或目录并自动生成不冲突名称。
pub fn duplicate_resource(path: &Path) -> Result<PathBuf, String> {
    let parent = path.parent().ok_or("资源没有父目录")?;
    let target = unique_destination(parent, path.file_name().ok_or("资源名称无效")?);
    if path.is_dir() {
        copy_resource_directory(path, &target)?;
    } else {
        fs::copy(path, &target).map_err(|error| format!("复制资源失败：{error}"))?;
    }
    Ok(target)
}

/// 删除指定资源或自定义文件夹。
pub fn delete_resource(path: &Path) -> Result<(), String> {
    if path.is_dir() {
        fs::remove_dir_all(path).map_err(|error| format!("删除资源文件夹失败：{error}"))
    } else {
        fs::remove_file(path).map_err(|error| format!("删除资源失败：{error}"))
    }
}

/// 将资源重命名，文件未提供扩展名时保留原扩展名。
pub fn rename_resource(path: &Path, new_name: &str) -> Result<PathBuf, String> {
    let name = new_name.trim();
    if name.is_empty() || name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
        return Err("资源新名称为空或包含Windows非法字符".to_owned());
    }
    let final_name = if path.is_file() && Path::new(name).extension().is_none() {
        match path.extension().and_then(|value| value.to_str()) {
            Some(extension) => format!("{name}.{extension}"),
            None => name.to_owned(),
        }
    } else {
        name.to_owned()
    };
    let target = path.parent().ok_or("资源没有父目录")?.join(final_name);
    fs::rename(path, &target).map_err(|error| format!("重命名资源失败：{error}"))?;
    Ok(target)
}

/// 按启用插件声明的目录和扩展名扫描自定义资源树。
pub fn scan_plugin_resource(
    project_root: &Path,
    definition: &PluginResourceDefinition,
) -> Vec<AssetEntry> {
    let directory = project_root.join(&definition.folder);
    let _ = fs::create_dir_all(&directory);
    let extensions: Vec<&str> = definition.extensions.iter().map(String::as_str).collect();
    scan_directory(&directory, &extensions)
}

/// 递归复制资源文件夹。
fn copy_resource_directory(source: &Path, target: &Path) -> Result<(), String> {
    fs::create_dir_all(target).map_err(|error| format!("创建复制目录失败：{error}"))?;
    for entry in fs::read_dir(source).map_err(|error| format!("读取复制目录失败：{error}"))?
    {
        let entry = entry.map_err(|error| format!("读取资源失败：{error}"))?;
        let output = target.join(entry.file_name());
        if entry.path().is_dir() {
            copy_resource_directory(&entry.path(), &output)?;
        } else {
            fs::copy(entry.path(), output).map_err(|error| format!("复制资源失败：{error}"))?;
        }
    }
    Ok(())
}

/// 合并扫描多个兼容资源目录，并保留各目录自己的文件夹层级。
fn scan_directories(directories: &[PathBuf], extensions: &[&str]) -> Vec<AssetEntry> {
    let mut entries = Vec::new();
    for directory in directories {
        entries.extend(scan_directory(directory, extensions));
    }
    entries.sort_by(|left, right| entry_name(left).cmp(entry_name(right)));
    entries
}

/// 根据文件扩展名判断资源类型。
pub fn classify_file(path: &Path) -> Option<AssetKind> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some(AssetKind::Image),
        "wav" | "ogg" | "mp3" | "flac" => Some(AssetKind::Audio),
        "s2anim" => Some(AssetKind::Animation),
        "s2tileset" => Some(AssetKind::Tileset),
        _ => None,
    }
}

/// 递归扫描一个资源目录，并只保留指定扩展名的文件。
fn scan_directory(directory: &Path, extensions: &[&str]) -> Vec<AssetEntry> {
    let mut entries = Vec::new();
    let read_directory = match fs::read_dir(directory) {
        Ok(value) => value,
        Err(_) => return entries,
    };
    for directory_entry in read_directory.flatten() {
        let path = directory_entry.path();
        let name = directory_entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            entries.push(AssetEntry::Folder {
                name,
                children: scan_directory(&path, extensions),
                path,
            });
        } else if has_extension(&path, extensions) {
            entries.push(AssetEntry::File { name, path });
        }
    }
    entries.sort_by(|left, right| entry_name(left).cmp(entry_name(right)));
    entries
}

/// 判断文件扩展名是否属于允许列表。
fn has_extension(path: &Path, extensions: &[&str]) -> bool {
    let extension = match path.extension().and_then(|value| value.to_str()) {
        Some(value) => value,
        None => return false,
    };
    extensions
        .iter()
        .any(|allowed| extension.eq_ignore_ascii_case(allowed))
}

/// 返回资源项名称，用于稳定排序。
fn entry_name(entry: &AssetEntry) -> &str {
    match entry {
        AssetEntry::Folder { name, .. } | AssetEntry::File { name, .. } => name,
    }
}

/// 文件重名时自动添加数字后缀，避免覆盖已有素材。
fn unique_destination(directory: &Path, file_name: &std::ffi::OsStr) -> PathBuf {
    let initial = directory.join(file_name);
    if !initial.exists() {
        return initial;
    }
    let source_path = Path::new(file_name);
    let stem = source_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("image");
    let extension = source_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("png");
    let mut number = 1;
    loop {
        let candidate = directory.join(format!("{stem}_{number}.{extension}"));
        if !candidate.exists() {
            return candidate;
        }
        number += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证图片和音频文件会按扩展名自动分类。
    #[test]
    fn files_are_classified_by_extension() {
        assert!(matches!(
            classify_file(Path::new("hero.PNG")),
            Some(AssetKind::Image)
        ));
        assert!(matches!(
            classify_file(Path::new("jump.wav")),
            Some(AssetKind::Audio)
        ));
        assert!(matches!(
            classify_file(Path::new("music.OGG")),
            Some(AssetKind::Audio)
        ));
        assert!(matches!(
            classify_file(Path::new("terrain.s2tileset")),
            Some(AssetKind::Tileset)
        ));
        assert!(classify_file(Path::new("notes.txt")).is_none());
    }

    /// 验证Actor模板保存后可以完整恢复物体资源和蓝图。
    #[test]
    fn actor_asset_round_trip() {
        let path = std::env::temp_dir().join("slide2d_actor_asset_test.s2actor");
        let object = GameObject {
            id: 7,
            x: 10.0,
            y: 20.0,
            width: 64.0,
            height: 96.0,
            layer_index: 3,
            image_path: "Content/Assets/Textures/player.png".to_owned(),
            audio_path: String::new(),
            animation_path: "Content/Animation/idle.s2anim".to_owned(),
            animation_playing: true,
            collider: Some(ColliderConfig { is_dynamic: true }),
            blueprint: Blueprint::new(),
            blueprint_file: "blueprint_7.json".to_owned(),
            variables: std::collections::HashMap::new(),
        };
        let actor = ActorAsset::from_game_object("Player".to_owned(), &object);
        actor.save(&path).expect("Actor应保存成功");
        let restored = ActorAsset::load(&path).expect("Actor应读取成功");
        assert_eq!(restored.slide2d_engine, "SLIDE2D_ACTOR");
        assert_eq!(restored.width, 64.0);
        assert_eq!(restored.animation_path, "Content/Animation/idle.s2anim");
        let _ = fs::remove_file(path);
    }
}
