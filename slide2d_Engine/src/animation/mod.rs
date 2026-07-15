//! 精灵动画资源、编辑器和运行时共用数据。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// 动画编辑器内部拖拽帧顺序时携带的索引。
#[derive(Clone)]
pub struct AnimationFrameDragPayload {
    pub index: usize,
}

/// 一个精灵动画资源，保存为assets/animations目录中的JSON文件。
#[derive(Clone, Serialize, Deserialize)]
pub struct SpriteAnimation {
    /// Slide2D动画JSON内置格式标识。
    #[serde(default = "animation_format")]
    pub slide2d_engine: String,
    pub name: String,
    pub frames: Vec<String>,
    pub frames_per_second: f32,
    pub looping: bool,
}

impl SpriteAnimation {
    /// 创建一个没有序列帧的默认动画。
    pub fn new() -> Self {
        Self {
            slide2d_engine: animation_format(),
            name: "新动画".to_owned(),
            frames: Vec::new(),
            frames_per_second: 12.0,
            looping: true,
        }
    }

    /// 从JSON文件读取精灵动画。
    pub fn load(path: &Path) -> Result<Self, String> {
        let text =
            std::fs::read_to_string(path).map_err(|error| format!("读取动画文件失败：{error}"))?;
        serde_json::from_str(&text).map_err(|error| format!("解析动画文件失败：{error}"))
    }

    /// 将精灵动画保存为格式化JSON。
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self)
            .map_err(|error| format!("生成动画JSON失败：{error}"))?;
        std::fs::write(path, text).map_err(|error| format!("保存动画文件失败：{error}"))
    }
}

/// 返回动画资源固定的Slide2D格式标识。
fn animation_format() -> String {
    "SLIDE2D_ANIMATION".to_owned()
}

/// 保存动画编辑窗口当前正在编辑的资源和预览状态。
pub struct AnimationEditorState {
    pub window_open: bool,
    pub animation_path: Option<PathBuf>,
    pub animation: SpriteAnimation,
    pub selected_frame: Option<usize>,
    pub preview_playing: bool,
    pub preview_started: Instant,
}

impl AnimationEditorState {
    /// 创建关闭状态的动画编辑器。
    pub fn new() -> Self {
        Self {
            window_open: false,
            animation_path: None,
            animation: SpriteAnimation::new(),
            selected_frame: None,
            preview_playing: true,
            preview_started: Instant::now(),
        }
    }

    /// 创建一个新的动画草稿并打开窗口。
    pub fn create_new(&mut self) {
        self.animation = SpriteAnimation::new();
        self.animation_path = None;
        self.selected_frame = None;
        self.preview_playing = true;
        self.preview_started = Instant::now();
        self.window_open = true;
    }

    /// 打开已有动画文件。
    pub fn open(&mut self, path: PathBuf) -> Result<(), String> {
        self.animation = SpriteAnimation::load(&path)?;
        self.animation_path = Some(path);
        self.selected_frame = None;
        self.preview_playing = true;
        self.preview_started = Instant::now();
        self.window_open = true;
        Ok(())
    }

    /// 根据时间计算预览应显示的帧索引。
    pub fn preview_frame_index(&self) -> Option<usize> {
        if self.animation.frames.is_empty() {
            return None;
        }
        if !self.preview_playing {
            return Some(
                self.selected_frame
                    .unwrap_or(0)
                    .min(self.animation.frames.len() - 1),
            );
        }
        let fps = self.animation.frames_per_second.max(1.0);
        let raw_index = (self.preview_started.elapsed().as_secs_f32() * fps) as usize;
        if self.animation.looping {
            Some(raw_index % self.animation.frames.len())
        } else {
            Some(raw_index.min(self.animation.frames.len() - 1))
        }
    }
}

/// 将绝对资源路径转换为项目相对路径。
pub fn relative_to_project(project_root: &Path, path: &Path) -> String {
    path.strip_prefix(project_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// 将动画中的相对帧路径解析为真实文件路径。
pub fn resolve_asset_path(project_root: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证动画资源可以完整序列化并恢复帧率、循环和帧顺序。
    #[test]
    fn animation_json_round_trip_preserves_data() {
        let animation = SpriteAnimation {
            slide2d_engine: animation_format(),
            name: "walk".to_owned(),
            frames: vec!["a.png".to_owned(), "b.png".to_owned()],
            frames_per_second: 8.0,
            looping: false,
        };
        let text = serde_json::to_string(&animation).unwrap();
        let restored: SpriteAnimation = serde_json::from_str(&text).unwrap();

        assert_eq!(restored.name, "walk");
        assert_eq!(restored.frames, vec!["a.png", "b.png"]);
        assert_eq!(restored.frames_per_second, 8.0);
        assert!(!restored.looping);
    }
}
