use crate::localization::tr;
use crate::plugins::{PluginBehavior, PluginNodeCategory, PluginNodeDefinition};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// 蓝图编辑器提供的完整常用物理按键列表。
///
/// 第一个字符串是保存到JSON的winit KeyCode名称，第二个字符串是界面显示名称。
pub const AVAILABLE_KEYS: &[(&str, &str)] = &[
    ("KeyA", "A"),
    ("KeyB", "B"),
    ("KeyC", "C"),
    ("KeyD", "D"),
    ("KeyE", "E"),
    ("KeyF", "F"),
    ("KeyG", "G"),
    ("KeyH", "H"),
    ("KeyI", "I"),
    ("KeyJ", "J"),
    ("KeyK", "K"),
    ("KeyL", "L"),
    ("KeyM", "M"),
    ("KeyN", "N"),
    ("KeyO", "O"),
    ("KeyP", "P"),
    ("KeyQ", "Q"),
    ("KeyR", "R"),
    ("KeyS", "S"),
    ("KeyT", "T"),
    ("KeyU", "U"),
    ("KeyV", "V"),
    ("KeyW", "W"),
    ("KeyX", "X"),
    ("KeyY", "Y"),
    ("KeyZ", "Z"),
    ("Digit0", "数字 0"),
    ("Digit1", "数字 1"),
    ("Digit2", "数字 2"),
    ("Digit3", "数字 3"),
    ("Digit4", "数字 4"),
    ("Digit5", "数字 5"),
    ("Digit6", "数字 6"),
    ("Digit7", "数字 7"),
    ("Digit8", "数字 8"),
    ("Digit9", "数字 9"),
    ("ArrowUp", "方向键 上"),
    ("ArrowDown", "方向键 下"),
    ("ArrowLeft", "方向键 左"),
    ("ArrowRight", "方向键 右"),
    ("Space", "空格"),
    ("Enter", "回车"),
    ("Escape", "Esc"),
    ("Tab", "Tab"),
    ("Backspace", "退格"),
    ("Delete", "Delete"),
    ("Insert", "Insert"),
    ("Home", "Home"),
    ("End", "End"),
    ("PageUp", "Page Up"),
    ("PageDown", "Page Down"),
    ("ShiftLeft", "左 Shift"),
    ("ShiftRight", "右 Shift"),
    ("ControlLeft", "左 Ctrl"),
    ("ControlRight", "右 Ctrl"),
    ("AltLeft", "左 Alt"),
    ("AltRight", "右 Alt"),
    ("SuperLeft", "左 Windows"),
    ("SuperRight", "右 Windows"),
    ("CapsLock", "Caps Lock"),
    ("NumLock", "Num Lock"),
    ("ScrollLock", "Scroll Lock"),
    ("Pause", "Pause"),
    ("PrintScreen", "Print Screen"),
    ("ContextMenu", "菜单键"),
    ("Backquote", "`"),
    ("Minus", "-"),
    ("Equal", "="),
    ("BracketLeft", "["),
    ("BracketRight", "]"),
    ("Backslash", "\\"),
    ("Semicolon", ";"),
    ("Quote", "'"),
    ("Comma", ","),
    ("Period", "."),
    ("Slash", "/"),
    ("Numpad0", "小键盘 0"),
    ("Numpad1", "小键盘 1"),
    ("Numpad2", "小键盘 2"),
    ("Numpad3", "小键盘 3"),
    ("Numpad4", "小键盘 4"),
    ("Numpad5", "小键盘 5"),
    ("Numpad6", "小键盘 6"),
    ("Numpad7", "小键盘 7"),
    ("Numpad8", "小键盘 8"),
    ("Numpad9", "小键盘 9"),
    ("NumpadAdd", "小键盘 +"),
    ("NumpadSubtract", "小键盘 -"),
    ("NumpadMultiply", "小键盘 *"),
    ("NumpadDivide", "小键盘 /"),
    ("NumpadDecimal", "小键盘 ."),
    ("NumpadEnter", "小键盘 Enter"),
    ("NumpadEqual", "小键盘 ="),
    ("NumpadComma", "小键盘 ,"),
    ("F1", "F1"),
    ("F2", "F2"),
    ("F3", "F3"),
    ("F4", "F4"),
    ("F5", "F5"),
    ("F6", "F6"),
    ("F7", "F7"),
    ("F8", "F8"),
    ("F9", "F9"),
    ("F10", "F10"),
    ("F11", "F11"),
    ("F12", "F12"),
    ("F13", "F13"),
    ("F14", "F14"),
    ("F15", "F15"),
    ("F16", "F16"),
    ("F17", "F17"),
    ("F18", "F18"),
    ("F19", "F19"),
    ("F20", "F20"),
    ("F21", "F21"),
    ("F22", "F22"),
    ("F23", "F23"),
    ("F24", "F24"),
    ("F25", "F25"),
    ("F26", "F26"),
    ("F27", "F27"),
    ("F28", "F28"),
    ("F29", "F29"),
    ("F30", "F30"),
    ("F31", "F31"),
    ("F32", "F32"),
    ("F33", "F33"),
    ("F34", "F34"),
    ("F35", "F35"),
    ("BrowserBack", "浏览器 后退"),
    ("BrowserForward", "浏览器 前进"),
    ("BrowserHome", "浏览器 首页"),
    ("BrowserRefresh", "浏览器 刷新"),
    ("BrowserSearch", "浏览器 搜索"),
    ("BrowserStop", "浏览器 停止"),
    ("BrowserFavorites", "浏览器 收藏"),
    ("AudioVolumeUp", "音量增加"),
    ("AudioVolumeDown", "音量降低"),
    ("AudioVolumeMute", "静音"),
    ("MediaPlayPause", "播放/暂停"),
    ("MediaStop", "媒体停止"),
    ("MediaTrackNext", "下一曲"),
    ("MediaTrackPrevious", "上一曲"),
    ("IntlBackslash", "国际键 \\"),
    ("IntlRo", "国际键 Ro"),
    ("IntlYen", "国际键 Yen"),
    ("Convert", "转换键"),
    ("NonConvert", "无转换键"),
    ("KanaMode", "假名模式"),
    ("Lang1", "语言键 1"),
    ("Lang2", "语言键 2"),
    ("Lang3", "语言键 3"),
    ("Lang4", "语言键 4"),
    ("Lang5", "语言键 5"),
];

/// 将旧场景中的简写按键名称转换为标准KeyCode名称。
pub fn canonical_key_name(key: &str) -> String {
    let trimmed = key.trim();
    if trimmed.len() == 1 {
        let character = trimmed.chars().next().unwrap_or(' ');
        if character.is_ascii_alphabetic() {
            return format!("Key{}", character.to_ascii_uppercase());
        }
        if character.is_ascii_digit() {
            return format!("Digit{character}");
        }
    }
    trimmed.to_owned()
}

/// 返回标准按键名称对应的中文界面名称。
pub fn key_display_name(key: &str) -> String {
    let canonical = canonical_key_name(key);
    AVAILABLE_KEYS
        .iter()
        .find(|(name, _)| *name == canonical)
        .map(|(_, display)| (*display).to_owned())
        .unwrap_or(canonical)
}

/// 一个游戏物体所拥有的完整蓝图。
#[derive(Clone, Serialize, Deserialize)]
pub struct Blueprint {
    /// Slide2D蓝图JSON内置格式标识。
    #[serde(default = "blueprint_format")]
    pub slide2d_engine: String,
    #[serde(default)]
    pub nodes: Vec<BlueprintNode>,
    #[serde(default)]
    pub connections: Vec<BlueprintConnection>,
    #[serde(default = "default_next_node_id")]
    pub next_node_id: u64,
}

/// 蓝图静态验证或运行时执行产生的诊断级别。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    Warning,
    Error,
}

/// 可由编辑器和Runtime共同展示的蓝图诊断。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlueprintDiagnostic {
    pub severity: Severity,
    pub code: String,
    pub message: String,
    pub node_id: Option<u64>,
}

impl BlueprintDiagnostic {
    pub(crate) fn new(
        severity: Severity,
        code: impl Into<String>,
        message: impl Into<String>,
        node_id: Option<u64>,
    ) -> Self {
        Self {
            severity,
            code: code.into(),
            message: message.into(),
            node_id,
        }
    }
}

impl Default for Blueprint {
    /// 创建默认蓝图时也从1开始分配节点ID，保持旧场景兼容行为一致。
    fn default() -> Self {
        Self::new()
    }
}

/// 蓝图节点的数据。位置使用蓝图窗口内部的二维坐标。
#[derive(Clone, Serialize, Deserialize)]
pub struct BlueprintNode {
    pub id: u64,
    pub x: f32,
    pub y: f32,
    pub kind: BlueprintNodeKind,
}

/// 蓝图节点所属的主要类别。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BlueprintNodeCategory {
    Event,
    Logic,
    Action,
    Variable,
}

/// 数值比较节点支持的比较方式。
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum ComparisonOperator {
    Equal,
    NotEqual,
    Greater,
    GreaterOrEqual,
    Less,
    LessOrEqual,
}

impl ComparisonOperator {
    /// 比较两个数值并返回条件是否成立。
    pub fn compare(self, left: f32, right: f32) -> bool {
        match self {
            Self::Equal => (left - right).abs() <= f32::EPSILON,
            Self::NotEqual => (left - right).abs() > f32::EPSILON,
            Self::Greater => left > right,
            Self::GreaterOrEqual => left >= right,
            Self::Less => left < right,
            Self::LessOrEqual => left <= right,
        }
    }

    /// 返回编辑器中显示的比较符号。
    pub fn display_name(self) -> String {
        match self {
            Self::Equal => tr("compare.equal"),
            Self::NotEqual => tr("compare.not_equal"),
            Self::Greater => tr("compare.greater"),
            Self::GreaterOrEqual => tr("compare.greater_equal"),
            Self::Less => tr("compare.less"),
            Self::LessOrEqual => tr("compare.less_equal"),
        }
    }
}

/// 蓝图当前支持的事件节点、动作节点和变量节点。
#[derive(Clone, Serialize, Deserialize)]
pub enum BlueprintNodeKind {
    /// 声明式插件节点，只保存数据和引擎白名单行为，不包含脚本代码。
    PluginNode {
        plugin_id: String,
        node_type: String,
        display_name: String,
        description: String,
        category: PluginNodeCategory,
        behavior: PluginBehavior,
        variable_name: String,
        value: f32,
    },
    /// 每个runtime逻辑帧都会触发的事件。
    FrameUpdated,
    /// 指定物理按键保持按下时触发的事件。
    KeyPressed { key: String },
    /// 当前物体与其他碰撞体开始接触时触发。
    CollisionTriggered,
    /// 指定UI按钮在当前帧被点击时触发。
    ButtonClicked { ui_id: u64 },
    /// 经过指定秒数后触发，可选循环。
    Timer { delay_seconds: f32, repeat: bool },
    /// 鼠标左键点击当前物体时触发。
    ObjectClicked,
    /// 当前场景加载完成后的第一帧触发。
    SceneLoaded,
    /// 根据变量和值比较，并从满足或不满足端口继续执行。
    IfCondition {
        variable_name: String,
        comparison: ComparisonOperator,
        compare_value: f32,
        use_global: bool,
    },
    /// 比较两个变量，并从满足或不满足端口继续执行。
    CompareVariables {
        left_name: String,
        right_name: String,
        comparison: ComparisonOperator,
        use_global: bool,
    },
    /// 按照每秒变化量移动当前物体。
    ModifyPosition { delta_x: f32, delta_y: f32 },
    /// 将当前物体中的指定变量设置成一个数值。
    SetVariable { name: String, value: f32 },
    /// 请求runtime播放一个音效文件。
    PlaySound { path: String, volume: f32 },
    /// 请求runtime停止当前物体触发的全部音效。
    StopSound,
    /// 切换当前物体使用的精灵动画资源。
    SwitchAnimation { animation_path: String },
    /// 暂停当前精灵动画。
    PauseAnimation,
    /// 继续播放当前精灵动画。
    PlayAnimation,
    /// 检测角色脚下瓦片，并把瓦片ID写入指定数值变量。
    DetectTile { variable_name: String },
    /// 修改文本框显示内容。
    SetUiText { ui_id: u64, content: String },
    /// 修改进度条当前数值。
    SetUiProgress { ui_id: u64, value: f32 },
    /// 显示或隐藏任意UI元素。
    SetUiVisible { ui_id: u64, visible: bool },
    /// 请求Runtime切换到工程中的指定场景。
    SwitchScene { scene_name: String },
    /// 按照场景内模板物体ID生成一个新物体。
    SpawnObject {
        template_object_id: u64,
        x: f32,
        y: f32,
    },
    /// 销毁指定物体；ID为0时销毁当前物体。
    DestroyObject { object_id: u64 },
    /// 写入所有场景共同使用的全局变量。
    SetGlobalVariable { name: String, value: f32 },
    /// 定义当前物体拥有的数值变量及其初始值。
    NumberVariable { name: String, initial_value: f32 },
    /// 声明所有场景和对象共同使用的全局数值变量。
    GlobalNumberVariable { name: String, initial_value: f32 },
    /// 将多个节点收纳为可折叠分组。
    NodeGroup {
        title: String,
        collapsed: bool,
        node_ids: Vec<u64>,
    },
}

impl BlueprintNodeKind {
    /// 返回节点所属类别，编辑器据此选择颜色和菜单分组。
    pub fn category(&self) -> BlueprintNodeCategory {
        match self {
            Self::PluginNode { category, .. } => match category {
                PluginNodeCategory::Event => BlueprintNodeCategory::Event,
                PluginNodeCategory::Logic => BlueprintNodeCategory::Logic,
                PluginNodeCategory::Action => BlueprintNodeCategory::Action,
                PluginNodeCategory::Variable => BlueprintNodeCategory::Variable,
            },
            Self::FrameUpdated
            | Self::KeyPressed { .. }
            | Self::CollisionTriggered
            | Self::ButtonClicked { .. }
            | Self::Timer { .. }
            | Self::ObjectClicked
            | Self::SceneLoaded => BlueprintNodeCategory::Event,
            Self::IfCondition { .. } | Self::CompareVariables { .. } => {
                BlueprintNodeCategory::Logic
            }
            Self::ModifyPosition { .. }
            | Self::SetVariable { .. }
            | Self::PlaySound { .. }
            | Self::StopSound
            | Self::SwitchAnimation { .. }
            | Self::PauseAnimation
            | Self::PlayAnimation
            | Self::SwitchScene { .. }
            | Self::SpawnObject { .. }
            | Self::DestroyObject { .. }
            | Self::SetGlobalVariable { .. } => BlueprintNodeCategory::Action,
            Self::DetectTile { .. } => BlueprintNodeCategory::Action,
            Self::SetUiText { .. } | Self::SetUiProgress { .. } | Self::SetUiVisible { .. } => {
                BlueprintNodeCategory::Action
            }
            Self::NumberVariable { .. }
            | Self::GlobalNumberVariable { .. }
            | Self::NodeGroup { .. } => BlueprintNodeCategory::Variable,
        }
    }

    /// 返回插件逻辑节点需要的成功和失败端口。
    pub fn uses_branch_outputs(&self) -> bool {
        matches!(
            self,
            Self::IfCondition { .. } | Self::CompareVariables { .. }
        ) || matches!(
            self,
            Self::PluginNode {
                behavior: PluginBehavior::PickupCheck,
                ..
            }
        )
    }

    /// 事件节点拥有执行输出端口。
    pub fn has_execution_output(&self) -> bool {
        matches!(
            self.category(),
            BlueprintNodeCategory::Event
                | BlueprintNodeCategory::Logic
                | BlueprintNodeCategory::Action
        )
    }

    /// 动作节点拥有执行输入端口。
    pub fn has_execution_input(&self) -> bool {
        matches!(
            self.category(),
            BlueprintNodeCategory::Logic | BlueprintNodeCategory::Action
        )
    }
}

/// 一条执行连线，从事件节点的输出端连接到执行节点的输入端。
#[derive(Clone, Serialize, Deserialize)]
pub struct BlueprintConnection {
    pub from_node_id: u64,
    pub to_node_id: u64,
    /// 输出端口：0为普通执行口，1为满足，2为不满足。
    #[serde(default)]
    pub from_port: u8,
}

impl Blueprint {
    /// 创建空蓝图，并从1开始分配节点ID。
    pub fn new() -> Self {
        Self {
            slide2d_engine: blueprint_format(),
            nodes: Vec::new(),
            connections: Vec::new(),
            next_node_id: 1,
        }
    }

    /// 静态检查蓝图格式、执行连线和变量引用，不修改原始序列化数据。
    pub fn validate(&self) -> Vec<BlueprintDiagnostic> {
        let mut diagnostics = Vec::new();
        if self.slide2d_engine != blueprint_format() {
            diagnostics.push(BlueprintDiagnostic::new(
                Severity::Error,
                "invalid_magic",
                format!("无效蓝图格式标识: {}", self.slide2d_engine),
                None,
            ));
        }

        let mut node_by_id = HashMap::new();
        let mut duplicate_ids = HashSet::new();
        for node in &self.nodes {
            if node_by_id.insert(node.id, node).is_some() && duplicate_ids.insert(node.id) {
                diagnostics.push(BlueprintDiagnostic::new(
                    Severity::Error,
                    "duplicate_node_id",
                    format!("节点ID {} 重复", node.id),
                    Some(node.id),
                ));
            }
        }

        let local_variables: HashSet<&str> = self
            .nodes
            .iter()
            .filter_map(|node| match &node.kind {
                BlueprintNodeKind::NumberVariable { name, .. } if !name.trim().is_empty() => {
                    Some(name.as_str())
                }
                BlueprintNodeKind::PluginNode {
                    behavior: PluginBehavior::NumberVariable,
                    variable_name,
                    ..
                } if !variable_name.trim().is_empty() => Some(variable_name.as_str()),
                _ => None,
            })
            .collect();
        for node in &self.nodes {
            match &node.kind {
                BlueprintNodeKind::NumberVariable { name, .. } => {
                    validate_variable_name(&mut diagnostics, node.id, name);
                }
                BlueprintNodeKind::GlobalNumberVariable { name, .. }
                | BlueprintNodeKind::SetGlobalVariable { name, .. } => {
                    validate_variable_name(&mut diagnostics, node.id, name);
                }
                BlueprintNodeKind::SetVariable { name, .. } => {
                    validate_local_variable(&mut diagnostics, &local_variables, node.id, name);
                }
                BlueprintNodeKind::DetectTile { variable_name }
                | BlueprintNodeKind::IfCondition {
                    variable_name,
                    use_global: false,
                    ..
                } => {
                    validate_local_variable(
                        &mut diagnostics,
                        &local_variables,
                        node.id,
                        variable_name,
                    );
                }
                BlueprintNodeKind::IfCondition {
                    variable_name,
                    use_global: true,
                    ..
                } => validate_variable_name(&mut diagnostics, node.id, variable_name),
                BlueprintNodeKind::CompareVariables {
                    left_name,
                    right_name,
                    use_global,
                    ..
                } => {
                    if *use_global {
                        validate_variable_name(&mut diagnostics, node.id, left_name);
                        validate_variable_name(&mut diagnostics, node.id, right_name);
                    } else {
                        validate_local_variable(
                            &mut diagnostics,
                            &local_variables,
                            node.id,
                            left_name,
                        );
                        validate_local_variable(
                            &mut diagnostics,
                            &local_variables,
                            node.id,
                            right_name,
                        );
                    }
                }
                BlueprintNodeKind::PluginNode {
                    behavior,
                    variable_name,
                    ..
                } if matches!(behavior, PluginBehavior::NumberVariable) => {
                    validate_variable_name(&mut diagnostics, node.id, variable_name)
                }
                BlueprintNodeKind::PluginNode {
                    behavior,
                    variable_name,
                    ..
                } if matches!(
                    behavior,
                    PluginBehavior::PickupCheck | PluginBehavior::SetObjectVariable
                ) =>
                {
                    validate_local_variable(
                        &mut diagnostics,
                        &local_variables,
                        node.id,
                        variable_name,
                    )
                }
                BlueprintNodeKind::PluginNode {
                    behavior: PluginBehavior::SetGlobalVariable,
                    variable_name,
                    ..
                } => validate_variable_name(&mut diagnostics, node.id, variable_name),
                _ => {}
            }
        }

        let mut outgoing: HashMap<u64, Vec<u64>> = HashMap::new();
        for connection in &self.connections {
            let source = node_by_id.get(&connection.from_node_id).copied();
            let target = node_by_id.get(&connection.to_node_id).copied();
            if source.is_none() || target.is_none() {
                diagnostics.push(BlueprintDiagnostic::new(
                    Severity::Error,
                    "invalid_connection_node",
                    format!(
                        "连线 {} -> {} 引用了不存在的节点",
                        connection.from_node_id, connection.to_node_id
                    ),
                    source
                        .map(|node| node.id)
                        .or_else(|| target.map(|node| node.id)),
                ));
                continue;
            }
            let source = source.expect("已检查源节点");
            let target = target.expect("已检查目标节点");
            if !source.kind.has_execution_output() || !target.kind.has_execution_input() {
                diagnostics.push(BlueprintDiagnostic::new(
                    Severity::Error,
                    "invalid_connection_direction",
                    format!("连线 {} -> {} 的执行方向无效", source.id, target.id),
                    Some(source.id),
                ));
                continue;
            }
            let port_is_valid = if source.kind.uses_branch_outputs() {
                matches!(connection.from_port, 1 | 2)
            } else {
                connection.from_port == 0
            };
            if !port_is_valid {
                diagnostics.push(BlueprintDiagnostic::new(
                    Severity::Error,
                    "invalid_connection_port",
                    format!(
                        "节点 {} 的输出端口 {} 无效",
                        source.id, connection.from_port
                    ),
                    Some(source.id),
                ));
                continue;
            }
            outgoing.entry(source.id).or_default().push(target.id);
        }

        let event_ids: Vec<u64> = self
            .nodes
            .iter()
            .filter(|node| node.kind.category() == BlueprintNodeCategory::Event)
            .map(|node| node.id)
            .collect();
        if event_ids.is_empty()
            && self
                .nodes
                .iter()
                .any(|node| node.kind.has_execution_input())
        {
            diagnostics.push(BlueprintDiagnostic::new(
                Severity::Warning,
                "no_event_entry",
                "执行图没有事件入口",
                None,
            ));
        }
        let mut reachable = HashSet::new();
        let mut pending = event_ids.clone();
        while let Some(node_id) = pending.pop() {
            if reachable.insert(node_id) {
                pending.extend(outgoing.get(&node_id).into_iter().flatten().copied());
            }
        }
        for node in &self.nodes {
            if node.kind.has_execution_input() && !reachable.contains(&node.id) {
                diagnostics.push(BlueprintDiagnostic::new(
                    Severity::Warning,
                    "unreachable_from_event",
                    format!("执行节点 {} 无法从任何事件到达", node.id),
                    Some(node.id),
                ));
            }
        }

        let mut visiting = HashSet::new();
        let mut visited = HashSet::new();
        for node in &self.nodes {
            if execution_cycle(node.id, &outgoing, &mut visiting, &mut visited) {
                diagnostics.push(BlueprintDiagnostic::new(
                    Severity::Warning,
                    "execution_cycle",
                    "执行图包含循环，可能触发单帧执行上限",
                    Some(node.id),
                ));
                break;
            }
        }
        diagnostics
    }

    /// 添加“键盘按下”事件节点，默认监听W键。
    pub fn add_key_pressed_node(&mut self) {
        let node = BlueprintNode {
            id: self.next_node_id,
            x: 50.0,
            y: 80.0 + self.nodes.len() as f32 * 105.0,
            kind: BlueprintNodeKind::KeyPressed {
                key: "KeyW".to_owned(),
            },
        };
        self.next_node_id += 1;
        self.nodes.push(node);
    }

    /// 添加每个runtime逻辑帧触发一次的“帧更新”事件节点。
    pub fn add_frame_updated_node(&mut self) {
        let node = BlueprintNode {
            id: self.next_node_id,
            x: 50.0,
            y: 80.0 + self.nodes.len() as f32 * 105.0,
            kind: BlueprintNodeKind::FrameUpdated,
        };
        self.next_node_id += 1;
        self.nodes.push(node);
    }

    /// 添加“物体碰撞”事件节点。
    pub fn add_collision_triggered_node(&mut self) {
        let node = BlueprintNode {
            id: self.next_node_id,
            x: 50.0,
            y: 80.0 + self.nodes.len() as f32 * 105.0,
            kind: BlueprintNodeKind::CollisionTriggered,
        };
        self.next_node_id += 1;
        self.nodes.push(node);
    }

    /// 添加“按钮点击”UI事件节点。
    pub fn add_button_clicked_node(&mut self) {
        let node = BlueprintNode {
            id: self.next_node_id,
            x: 50.0,
            y: 80.0 + self.nodes.len() as f32 * 105.0,
            kind: BlueprintNodeKind::ButtonClicked { ui_id: 1 },
        };
        self.next_node_id += 1;
        self.nodes.push(node);
    }

    /// 添加“修改物体坐标”执行节点，默认每秒向上移动200像素。
    pub fn add_modify_position_node(&mut self) {
        let node = BlueprintNode {
            id: self.next_node_id,
            x: 350.0,
            y: 80.0 + self.nodes.len() as f32 * 105.0,
            kind: BlueprintNodeKind::ModifyPosition {
                delta_x: 0.0,
                delta_y: -200.0,
            },
        };
        self.next_node_id += 1;
        self.nodes.push(node);
    }

    /// 添加“设置变量”动作节点。
    pub fn add_set_variable_node(&mut self) {
        let node = BlueprintNode {
            id: self.next_node_id,
            x: 350.0,
            y: 80.0 + self.nodes.len() as f32 * 105.0,
            kind: BlueprintNodeKind::SetVariable {
                name: "score".to_owned(),
                value: 0.0,
            },
        };
        self.next_node_id += 1;
        self.nodes.push(node);
    }

    /// 添加“播放音效”动作节点，先保存路径和音量供runtime音频接口消费。
    pub fn add_play_sound_node(&mut self) {
        let node = BlueprintNode {
            id: self.next_node_id,
            x: 350.0,
            y: 80.0 + self.nodes.len() as f32 * 105.0,
            kind: BlueprintNodeKind::PlaySound {
                path: String::new(),
                volume: 1.0,
            },
        };
        self.next_node_id += 1;
        self.nodes.push(node);
    }

    /// 添加“停止音效”动作节点。
    pub fn add_stop_sound_node(&mut self) {
        let node = BlueprintNode {
            id: self.next_node_id,
            x: 350.0,
            y: 80.0 + self.nodes.len() as f32 * 105.0,
            kind: BlueprintNodeKind::StopSound,
        };
        self.next_node_id += 1;
        self.nodes.push(node);
    }

    /// 添加“切换精灵动画”动作节点。
    pub fn add_switch_animation_node(&mut self) {
        self.push_action(BlueprintNodeKind::SwitchAnimation {
            animation_path: String::new(),
        });
    }

    /// 添加“暂停动画”动作节点。
    pub fn add_pause_animation_node(&mut self) {
        self.push_action(BlueprintNodeKind::PauseAnimation);
    }

    /// 添加“播放动画”动作节点。
    pub fn add_play_animation_node(&mut self) {
        self.push_action(BlueprintNodeKind::PlayAnimation);
    }

    /// 添加“检测瓦片”动作节点。
    pub fn add_detect_tile_node(&mut self) {
        self.push_action(BlueprintNodeKind::DetectTile {
            variable_name: "tile_id".to_owned(),
        });
    }

    /// 添加设置文本内容动作节点。
    pub fn add_set_ui_text_node(&mut self) {
        self.push_action(BlueprintNodeKind::SetUiText {
            ui_id: 1,
            content: "新文本".to_owned(),
        });
    }

    /// 添加修改进度条数值动作节点。
    pub fn add_set_ui_progress_node(&mut self) {
        self.push_action(BlueprintNodeKind::SetUiProgress {
            ui_id: 1,
            value: 50.0,
        });
    }

    /// 添加显示或隐藏UI动作节点。
    pub fn add_set_ui_visible_node(&mut self) {
        self.push_action(BlueprintNodeKind::SetUiVisible {
            ui_id: 1,
            visible: true,
        });
    }

    /// 使用统一默认位置添加一个动作节点。
    fn push_action(&mut self, kind: BlueprintNodeKind) {
        let node = BlueprintNode {
            id: self.next_node_id,
            x: 350.0,
            y: 80.0 + self.nodes.len() as f32 * 105.0,
            kind,
        };
        self.next_node_id += 1;
        self.nodes.push(node);
    }

    /// 添加一个带名称和初始值的数值变量节点。
    pub fn add_number_variable_node(&mut self) {
        let node = BlueprintNode {
            id: self.next_node_id,
            x: 650.0,
            y: 80.0 + self.nodes.len() as f32 * 105.0,
            kind: BlueprintNodeKind::NumberVariable {
                name: "score".to_owned(),
                initial_value: 0.0,
            },
        };
        self.next_node_id += 1;
        self.nodes.push(node);
    }

    /// 添加一条连线。如果相同连线已经存在，则不重复添加。
    #[allow(dead_code)]
    pub fn connect(&mut self, from_node_id: u64, to_node_id: u64) {
        self.connect_from_port(from_node_id, 0, to_node_id);
    }

    /// 从指定执行端口添加连线，If节点使用1和2表示两个分支。
    pub fn connect_from_port(&mut self, from_node_id: u64, from_port: u8, to_node_id: u64) {
        let source_is_event = self
            .nodes
            .iter()
            .any(|node| node.id == from_node_id && node.kind.has_execution_output());
        let target_is_action = self
            .nodes
            .iter()
            .any(|node| node.id == to_node_id && node.kind.has_execution_input());
        if !source_is_event || !target_is_action {
            return;
        }
        let already_exists = self.connections.iter().any(|connection| {
            connection.from_node_id == from_node_id
                && connection.to_node_id == to_node_id
                && connection.from_port == from_port
        });
        if !already_exists && from_node_id != to_node_id {
            self.connections.push(BlueprintConnection {
                from_node_id,
                to_node_id,
                from_port,
            });
        }
    }

    /// 使用统一位置添加任意高级节点。
    pub fn add_kind(&mut self, kind: BlueprintNodeKind) {
        let x = match kind.category() {
            BlueprintNodeCategory::Event => 50.0,
            BlueprintNodeCategory::Logic => 350.0,
            BlueprintNodeCategory::Action => 650.0,
            BlueprintNodeCategory::Variable => 950.0,
        };
        self.nodes.push(BlueprintNode {
            id: self.next_node_id,
            x,
            y: 80.0 + self.nodes.len() as f32 * 105.0,
            kind,
        });
        self.next_node_id += 1;
    }

    /// 根据已启用插件定义添加一个无脚本插件节点。
    pub fn add_plugin_node(&mut self, plugin_id: String, definition: PluginNodeDefinition) {
        self.add_kind(BlueprintNodeKind::PluginNode {
            plugin_id,
            node_type: definition.node_type,
            display_name: definition.display_name,
            description: definition.description,
            category: definition.category,
            behavior: definition.behavior,
            variable_name: definition.variable_name,
            value: definition.value,
        });
    }

    /// 将剪贴板蓝图粘贴进当前蓝图，并为节点和内部连线重新分配ID。
    pub fn paste_blueprint(&mut self, clipboard: &Blueprint, offset: f32) -> Vec<u64> {
        let mut id_map = std::collections::HashMap::new();
        let mut new_ids = Vec::new();
        for source in &clipboard.nodes {
            let mut node = source.clone();
            node.id = self.next_node_id;
            self.next_node_id += 1;
            node.x += offset;
            node.y += offset;
            id_map.insert(source.id, node.id);
            new_ids.push(node.id);
            self.nodes.push(node);
        }
        for connection in &clipboard.connections {
            if let (Some(from), Some(to)) = (
                id_map.get(&connection.from_node_id),
                id_map.get(&connection.to_node_id),
            ) {
                self.connections.push(BlueprintConnection {
                    from_node_id: *from,
                    to_node_id: *to,
                    from_port: connection.from_port,
                });
            }
        }
        new_ids
    }

    /// 删除一批节点及其全部连线。
    pub fn remove_nodes(&mut self, node_ids: &[u64]) {
        self.nodes.retain(|node| !node_ids.contains(&node.id));
        self.connections.retain(|connection| {
            !node_ids.contains(&connection.from_node_id)
                && !node_ids.contains(&connection.to_node_id)
        });
    }

    /// 删除一个节点，并自动删除所有连接到该节点的连线。
    pub fn remove_node(&mut self, node_id: u64) {
        self.nodes.retain(|node| node.id != node_id);
        self.connections.retain(|connection| {
            connection.from_node_id != node_id && connection.to_node_id != node_id
        });
    }

    /// 断开一个节点的全部输入和输出连线，但保留节点本身。
    pub fn disconnect_node(&mut self, node_id: u64) {
        self.connections.retain(|connection| {
            connection.from_node_id != node_id && connection.to_node_id != node_id
        });
    }

    /// 根据连线在数组中的位置删除一条指定连线。
    pub fn remove_connection(&mut self, connection_index: usize) {
        if connection_index < self.connections.len() {
            self.connections.remove(connection_index);
        }
    }
}

fn validate_variable_name(diagnostics: &mut Vec<BlueprintDiagnostic>, node_id: u64, name: &str) {
    if name.trim().is_empty() {
        diagnostics.push(BlueprintDiagnostic::new(
            Severity::Error,
            "empty_variable_name",
            "变量名不能为空",
            Some(node_id),
        ));
    }
}

fn validate_local_variable(
    diagnostics: &mut Vec<BlueprintDiagnostic>,
    local_variables: &HashSet<&str>,
    node_id: u64,
    name: &str,
) {
    validate_variable_name(diagnostics, node_id, name);
    if !name.trim().is_empty() && !local_variables.contains(name) {
        diagnostics.push(BlueprintDiagnostic::new(
            Severity::Error,
            "undeclared_local_variable",
            format!("局部变量 {name} 未声明"),
            Some(node_id),
        ));
    }
}

fn execution_cycle(
    node_id: u64,
    outgoing: &HashMap<u64, Vec<u64>>,
    visiting: &mut HashSet<u64>,
    visited: &mut HashSet<u64>,
) -> bool {
    if visiting.contains(&node_id) {
        return true;
    }
    if !visited.insert(node_id) {
        return false;
    }
    visiting.insert(node_id);
    let has_cycle = outgoing
        .get(&node_id)
        .into_iter()
        .flatten()
        .any(|next| execution_cycle(*next, outgoing, visiting, visited));
    visiting.remove(&node_id);
    has_cycle
}

/// 返回蓝图文件固定的Slide2D格式标识。
fn blueprint_format() -> String {
    "SLIDE2D_BLUEPRINT".to_owned()
}

/// 旧蓝图缺少next_node_id时仍从1开始。
fn default_next_node_id() -> u64 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证旧场景缺少蓝图字段时，默认节点ID从1开始。
    #[test]
    fn default_blueprint_starts_node_ids_at_one() {
        assert_eq!(Blueprint::default().next_node_id, 1);
    }

    /// 验证模型层会拒绝方向错误的节点连线。
    #[test]
    fn invalid_connection_direction_is_rejected() {
        let mut blueprint = Blueprint::new();
        blueprint.add_key_pressed_node();
        blueprint.add_modify_position_node();
        blueprint.connect(2, 1);
        assert!(blueprint.connections.is_empty());

        blueprint.connect(1, 2);
        assert_eq!(blueprint.connections.len(), 1);
    }

    /// 验证删除节点时会同时清理与它有关的连线。
    #[test]
    fn removing_node_removes_its_connections() {
        let mut blueprint = Blueprint::new();
        blueprint.add_key_pressed_node();
        blueprint.add_modify_position_node();
        blueprint.connect(1, 2);

        blueprint.remove_node(1);

        assert_eq!(blueprint.nodes.len(), 1);
        assert!(blueprint.connections.is_empty());
    }

    /// 验证三种节点类别及执行端口能力正确。
    #[test]
    fn node_categories_have_correct_execution_ports() {
        let event = BlueprintNodeKind::FrameUpdated;
        let action = BlueprintNodeKind::SetVariable {
            name: "score".to_owned(),
            value: 1.0,
        };
        let variable = BlueprintNodeKind::NumberVariable {
            name: "score".to_owned(),
            initial_value: 0.0,
        };

        assert_eq!(event.category(), BlueprintNodeCategory::Event);
        assert!(event.has_execution_output());
        assert!(!event.has_execution_input());
        assert_eq!(action.category(), BlueprintNodeCategory::Action);
        assert!(action.has_execution_input());
        assert!(action.has_execution_output());
        assert_eq!(variable.category(), BlueprintNodeCategory::Variable);
        assert!(!variable.has_execution_input());
        assert!(!variable.has_execution_output());
    }

    /// 验证旧蓝图连线缺少端口字段时自动使用普通输出端口。
    #[test]
    fn old_connection_defaults_to_normal_port() {
        let connection: BlueprintConnection =
            serde_json::from_str(r#"{"from_node_id":1,"to_node_id":2}"#)
                .expect("旧连线应当可以读取");
        assert_eq!(connection.from_port, 0);
    }

    /// 验证粘贴节点会重新分配ID并保留内部连线。
    #[test]
    fn pasted_nodes_receive_new_ids() {
        let mut clipboard = Blueprint::new();
        clipboard.add_frame_updated_node();
        clipboard.add_modify_position_node();
        clipboard.connect(1, 2);
        let mut target = Blueprint::new();
        target.add_frame_updated_node();
        let ids = target.paste_blueprint(&clipboard, 20.0);
        assert_eq!(ids, vec![2, 3]);
        assert!(target
            .connections
            .iter()
            .any(|connection| connection.from_node_id == 2 && connection.to_node_id == 3));
    }

    #[test]
    fn validation_reports_structure_and_variable_errors() {
        let blueprint = Blueprint {
            slide2d_engine: "WRONG".to_owned(),
            nodes: vec![
                BlueprintNode {
                    id: 1,
                    x: 0.0,
                    y: 0.0,
                    kind: BlueprintNodeKind::FrameUpdated,
                },
                BlueprintNode {
                    id: 1,
                    x: 0.0,
                    y: 0.0,
                    kind: BlueprintNodeKind::SetVariable {
                        name: "missing".to_owned(),
                        value: 1.0,
                    },
                },
                BlueprintNode {
                    id: 2,
                    x: 0.0,
                    y: 0.0,
                    kind: BlueprintNodeKind::ModifyPosition {
                        delta_x: 0.0,
                        delta_y: 0.0,
                    },
                },
            ],
            connections: vec![BlueprintConnection {
                from_node_id: 99,
                to_node_id: 1,
                from_port: 3,
            }],
            next_node_id: 3,
        };

        let codes: HashSet<_> = blueprint
            .validate()
            .into_iter()
            .map(|diagnostic| diagnostic.code)
            .collect();
        assert!(codes.contains("invalid_magic"));
        assert!(codes.contains("duplicate_node_id"));
        assert!(codes.contains("invalid_connection_node"));
        assert!(codes.contains("undeclared_local_variable"));
        assert!(codes.contains("unreachable_from_event"));
    }

    #[test]
    fn validation_warns_about_execution_cycles() {
        let mut blueprint = Blueprint::new();
        blueprint.add_frame_updated_node();
        blueprint.add_kind(BlueprintNodeKind::ModifyPosition {
            delta_x: 1.0,
            delta_y: 0.0,
        });
        blueprint.add_kind(BlueprintNodeKind::ModifyPosition {
            delta_x: 1.0,
            delta_y: 0.0,
        });
        blueprint.connect(1, 2);
        blueprint.connect(2, 3);
        blueprint.connect(3, 2);

        assert!(blueprint
            .validate()
            .iter()
            .any(|diagnostic| diagnostic.code == "execution_cycle"));
    }
}
