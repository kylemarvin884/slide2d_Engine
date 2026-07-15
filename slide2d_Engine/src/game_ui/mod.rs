//! 游戏UI持久化数据模型。

use serde::{Deserialize, Serialize};

use crate::blueprint::model::Blueprint;

/// 编辑器资源库中可拖出的UI组件模板。
#[derive(Clone, Copy, PartialEq)]
pub enum UiTemplate {
    Text,
    Button,
    ProgressBar,
    ImagePanel,
}

/// egui内部拖拽UI组件时携带的数据。
#[derive(Clone)]
pub struct UiDragPayload {
    pub template: UiTemplate,
}

/// 场景中一个UI元素的完整数据。
#[derive(Clone, Serialize, Deserialize)]
pub struct UiElement {
    pub id: u64,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub layer_index: u32,
    #[serde(default = "default_true")]
    pub visible: bool,
    pub kind: UiElementKind,
    /// UI元素自己的蓝图，按钮等组件可直接双击编辑逻辑。
    #[serde(default)]
    pub blueprint: Blueprint,
}

/// 当前支持的四种游戏UI元素。
#[derive(Clone, Serialize, Deserialize)]
pub enum UiElementKind {
    Text {
        content: String,
        font_size: f32,
        color: [u8; 4],
    },
    Button {
        text: String,
    },
    ProgressBar {
        maximum: f32,
        value: f32,
        background_color: [u8; 4],
        fill_color: [u8; 4],
    },
    ImagePanel {
        image_path: String,
    },
}

impl UiElement {
    /// 根据资源模板创建一个带默认参数的UI元素。
    pub fn from_template(id: u64, layer_index: u32, template: UiTemplate, x: f32, y: f32) -> Self {
        let (width, height, kind) = match template {
            UiTemplate::Text => (
                220.0,
                40.0,
                UiElementKind::Text {
                    content: "文本".to_owned(),
                    font_size: 24.0,
                    color: [255, 255, 255, 255],
                },
            ),
            UiTemplate::Button => (
                160.0,
                48.0,
                UiElementKind::Button {
                    text: "按钮".to_owned(),
                },
            ),
            UiTemplate::ProgressBar => (
                240.0,
                28.0,
                UiElementKind::ProgressBar {
                    maximum: 100.0,
                    value: 100.0,
                    background_color: [60, 60, 60, 255],
                    fill_color: [70, 190, 90, 255],
                },
            ),
            UiTemplate::ImagePanel => (
                160.0,
                120.0,
                UiElementKind::ImagePanel {
                    image_path: String::new(),
                },
            ),
        };
        Self {
            id,
            x,
            y,
            width,
            height,
            layer_index,
            visible: true,
            kind,
            blueprint: Blueprint::new(),
        }
    }
}

/// serde读取旧场景时让UI默认可见。
fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证UI参数可以完整保存并恢复。
    #[test]
    fn ui_element_json_round_trip() {
        let element = UiElement::from_template(1, 0, UiTemplate::ProgressBar, 20.0, 30.0);
        let text = serde_json::to_string(&element).unwrap();
        let restored: UiElement = serde_json::from_str(&text).unwrap();
        assert_eq!(restored.id, 1);
        assert!(matches!(restored.kind, UiElementKind::ProgressBar { .. }));
    }
}
