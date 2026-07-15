use std::collections::{HashMap, HashSet};

use crate::app_state::GameObject;
use crate::blueprint::model::{
    canonical_key_name, Blueprint, BlueprintDiagnostic, BlueprintNodeKind, Severity,
};
use crate::game_ui::UiElement;
use crate::plugins::{
    PluginAuthorizationMap, PluginBehavior, PluginNodeCategory, PluginRegistry,
};

/// 保存蓝图虚拟机当前帧需要读取的输入状态。
pub struct BlueprintInput {
    pub pressed_keys: HashSet<String>,
    pub collision_objects: HashSet<u64>,
    pub tiles_under_objects: HashMap<u64, i32>,
    pub clicked_ui_ids: HashSet<u64>,
    pub clicked_object_ids: HashSet<u64>,
    pub scene_just_loaded: bool,
    /// 仅由可信PluginRegistry/manifest构建的插件节点授权。
    pub plugin_authorizations: PluginAuthorizationMap,
    /// 当前处于Runtime活动视口及扩展边距内的Actor ID。
    pub active_object_ids: HashSet<u64>,
    /// 是否允许纯外部事件蓝图在活动区外休眠。
    pub dormant_blueprints_enabled: bool,
    /// 是否启用蓝图签名和变量初始化缓存。
    pub blueprint_cache_enabled: bool,
}

impl BlueprintInput {
    /// 创建没有任何按键被按下的输入状态。
    pub fn new() -> Self {
        Self {
            pressed_keys: HashSet::new(),
            collision_objects: HashSet::new(),
            tiles_under_objects: HashMap::new(),
            clicked_ui_ids: HashSet::new(),
            clicked_object_ids: HashSet::new(),
            scene_just_loaded: true,
            plugin_authorizations: HashMap::new(),
            active_object_ids: HashSet::new(),
            dormant_blueprints_enabled: false,
            blueprint_cache_enabled: true,
        }
    }

    /// 使用已验证注册表替换当前插件授权；空注册表不会授权任何插件节点。
    pub fn authorize_plugins(&mut self, registry: &PluginRegistry) {
        self.plugin_authorizations = registry.runtime_authorizations();
    }

    /// 验证插件、节点类型、manifest行为和Runtime能力四重授权边界。
    fn plugin_node_is_authorized(
        &self,
        plugin_id: &str,
        node_type: &str,
        behavior: &PluginBehavior,
    ) -> bool {
        self.plugin_authorizations
            .get(plugin_id)
            .is_some_and(|authorization| authorization.allows(node_type, behavior))
    }

    /// 设置一个物理按键当前是否处于按下状态。
    pub fn set_key_down(&mut self, key_name: String, is_down: bool) {
        if is_down {
            self.pressed_keys.insert(key_name);
        } else {
            self.pressed_keys.remove(&key_name);
        }
    }

    /// 清除全部按键，避免窗口失去焦点后出现卡键。
    pub fn clear(&mut self) {
        self.pressed_keys.clear();
        self.collision_objects.clear();
        self.tiles_under_objects.clear();
        self.clicked_ui_ids.clear();
        self.clicked_object_ids.clear();
    }

    /// 查询蓝图配置的按键当前是否处于按下状态。
    pub fn is_key_down(&self, key_name: &str) -> bool {
        self.pressed_keys.contains(&canonical_key_name(key_name))
    }

    /// 标记本帧刚刚发生碰撞的物体。
    pub fn set_collision_objects(&mut self, objects: HashSet<u64>) {
        self.collision_objects = objects;
    }

    /// 保存每个物体脚下检测到的瓦片ID，没有瓦片时使用-1。
    pub fn set_tiles_under_objects(&mut self, tiles: HashMap<u64, i32>) {
        self.tiles_under_objects = tiles;
    }

    /// 设置本帧被点击的UI按钮ID集合。
    pub fn set_clicked_ui_ids(&mut self, clicked: HashSet<u64>) {
        self.clicked_ui_ids = clicked;
    }

    /// 设置本帧鼠标点击命中的物体ID集合。
    pub fn set_clicked_object_ids(&mut self, clicked: HashSet<u64>) {
        self.clicked_object_ids = clicked;
    }
}

/// 蓝图VM输出的UI修改命令，由Runtime统一应用。
pub enum UiCommand {
    SetText { ui_id: u64, content: String },
    SetProgress { ui_id: u64, value: f32 },
    SetVisible { ui_id: u64, visible: bool },
}

/// 蓝图请求Runtime在当前逻辑帧结束后执行的场景或物体操作。
pub enum RuntimeCommand {
    Ui(UiCommand),
    SwitchScene(String),
    SpawnObject {
        template_object_id: u64,
        x: f32,
        y: f32,
    },
    DestroyObject(u64),
    PlaySound {
        owner_id: u64,
        path: String,
        volume: f32,
    },
    StopSound {
        owner_id: u64,
    },
}

/// 单个Actor或UI蓝图实例跨帧保留的数据。
#[derive(Default)]
pub struct BlueprintInstanceState {
    pub variables: HashMap<String, f32>,
    timers: HashMap<u64, f32>,
}

/// 保存编译计划、实例变量、定时器和可查询诊断。
pub struct BlueprintRuntimeState {
    instances: HashMap<u64, BlueprintInstanceState>,
    programs: HashMap<u64, BlueprintProgram>,
    diagnostics: HashMap<u64, Vec<BlueprintDiagnostic>>,
}

impl BlueprintRuntimeState {
    /// 创建空蓝图运行状态。
    pub fn new() -> Self {
        Self {
            instances: HashMap::new(),
            programs: HashMap::new(),
            diagnostics: HashMap::new(),
        }
    }

    /// 返回指定owner最近一次编译和执行产生的诊断。
    pub fn diagnostics(&self, owner_id: u64) -> &[BlueprintDiagnostic] {
        self.diagnostics
            .get(&owner_id)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }

    /// 返回指定owner的持久实例变量，UI蓝图也通过此接口读取状态。
    pub fn variables(&self, owner_id: u64) -> Option<&HashMap<String, f32>> {
        self.instances.get(&owner_id).map(|state| &state.variables)
    }

    /// 删除已销毁Actor或UI拥有的全部运行状态和缓存。
    pub fn remove_owner(&mut self, owner_id: u64) {
        self.instances.remove(&owner_id);
        self.programs.remove(&owner_id);
        self.diagnostics.remove(&owner_id);
    }
}

/// 蓝图只读执行计划，避免每帧重复扫描节点和连线。
#[derive(Clone)]
pub struct BlueprintProgram {
    event_node_ids: Vec<u64>,
    node_indices: HashMap<u64, usize>,
    outgoing: HashMap<(u64, u8), Vec<u64>>,
}

impl BlueprintProgram {
    /// 将蓝图节点和连线预解析为快速查找表，并保持原执行顺序。
    pub fn compile(blueprint: &Blueprint) -> (Self, Vec<BlueprintDiagnostic>) {
        let event_node_ids = blueprint
            .nodes
            .iter()
            .filter(|node| {
                node.kind.category() == crate::blueprint::model::BlueprintNodeCategory::Event
            })
            .map(|node| node.id)
            .collect();
        let node_indices = blueprint
            .nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id, index))
            .collect();
        let mut outgoing: HashMap<(u64, u8), Vec<u64>> = HashMap::new();
        for connection in &blueprint.connections {
            outgoing
                .entry((connection.from_node_id, connection.from_port))
                .or_default()
                .push(connection.to_node_id);
        }
        for nodes in outgoing.values_mut() {
            nodes.reverse();
        }
        (
            Self {
                event_node_ids,
                node_indices,
                outgoing,
            },
            blueprint.validate(),
        )
    }

    /// 返回指定端口后继节点的执行栈顺序。
    fn outgoing_nodes(&self, node_id: u64, port: u8) -> Vec<u64> {
        self.outgoing
            .get(&(node_id, port))
            .cloned()
            .unwrap_or_default()
    }
}

/// 每一帧解析所有物体的蓝图，并执行满足条件的连线。
pub fn update_blueprints_with_state(
    game_objects: &mut [GameObject],
    input: &BlueprintInput,
    delta_time: f32,
    global_variables: &mut HashMap<String, f32>,
    runtime_state: &mut BlueprintRuntimeState,
) -> Vec<RuntimeCommand> {
    let mut commands = Vec::new();
    for game_object in game_objects {
        let blueprint = std::mem::take(&mut game_object.blueprint);
        let owner_id = game_object.id;
        let is_new_program =
            !input.blueprint_cache_enabled || !runtime_state.programs.contains_key(&owner_id);
        if is_new_program {
            let (program, diagnostics) = BlueprintProgram::compile(&blueprint);
            runtime_state.programs.insert(owner_id, program);
            runtime_state.diagnostics.insert(owner_id, diagnostics);
            let instance = runtime_state.instances.entry(owner_id).or_default();
            initialize_variables(&mut instance.variables, &blueprint.nodes, input);
            initialize_global_variables(global_variables, &blueprint.nodes);
        }
        if let Some(instance) = runtime_state.instances.get_mut(&owner_id) {
            for (name, value) in &game_object.variables {
                instance.variables.insert(name.clone(), *value);
            }
            game_object.variables.clone_from(&instance.variables);
        }
        if should_sleep_blueprint(game_object.id, &blueprint, input) {
            game_object.blueprint = blueprint;
            continue;
        }
        let plan = if input.blueprint_cache_enabled {
            runtime_state
                .programs
                .entry(owner_id)
                .or_insert_with(|| BlueprintProgram::compile(&blueprint).0)
                .clone()
        } else {
            BlueprintProgram::compile(&blueprint).0
        };
        let mut queue = Vec::new();
        for node_id in &plan.event_node_ids {
            let node = match plan
                .node_indices
                .get(node_id)
                .and_then(|index| blueprint.nodes.get(*index))
            {
                Some(node) => node,
                None => continue,
            };
            if event_is_active_with_state(
                &node.kind,
                input,
                game_object.id,
                node.id,
                delta_time,
                runtime_state,
            ) {
                queue.extend(plan.outgoing_nodes(node.id, 0));
            }
        }
        let mut executed_steps = 0;
        while let Some(node_id) = queue.pop() {
            executed_steps += 1;
            if executed_steps > 1024 {
                let diagnostics = runtime_state.diagnostics.entry(owner_id).or_default();
                if !diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == "execution_limit_exceeded")
                {
                    diagnostics.push(BlueprintDiagnostic::new(
                        Severity::Error,
                        "execution_limit_exceeded",
                        "单帧蓝图执行超过1024步，执行已中止",
                        Some(node_id),
                    ));
                }
                break;
            }
            let action_node = match plan
                .node_indices
                .get(&node_id)
                .and_then(|index| blueprint.nodes.get(*index))
            {
                Some(node) => node,
                None => continue,
            };
            let mut output_port = 0;
            match &action_node.kind {
                BlueprintNodeKind::PluginNode {
                    plugin_id,
                    node_type,
                    behavior,
                    variable_name,
                    value,
                    ..
                } => {
                    if !input.plugin_node_is_authorized(plugin_id, node_type, behavior) {
                        continue;
                    }
                    match behavior {
                        PluginBehavior::PickupCheck => {
                            let picked = input.clicked_object_ids.contains(&game_object.id);
                            if !variable_name.trim().is_empty() {
                                game_object
                                    .variables
                                    .insert(variable_name.clone(), if picked { 1.0 } else { 0.0 });
                            }
                            output_port = if picked { 1 } else { 2 };
                        }
                        PluginBehavior::SetObjectVariable => {
                            if !variable_name.trim().is_empty() {
                                game_object.variables.insert(variable_name.clone(), *value);
                            }
                        }
                        PluginBehavior::SetGlobalVariable => {
                            if !variable_name.trim().is_empty() {
                                global_variables.insert(variable_name.clone(), *value);
                            }
                        }
                        PluginBehavior::MoveHorizontal => {
                            game_object.x += *value * delta_time;
                        }
                        PluginBehavior::SceneLoadedEvent
                        | PluginBehavior::ObjectClickedEvent
                        | PluginBehavior::NumberVariable => {}
                    }
                }
                BlueprintNodeKind::ModifyPosition { delta_x, delta_y } => {
                    game_object.x += delta_x * delta_time;
                    game_object.y += delta_y * delta_time;
                }
                BlueprintNodeKind::SetVariable { name, value } => {
                    if !name.trim().is_empty() {
                        game_object.variables.insert(name.clone(), *value);
                    }
                }
                BlueprintNodeKind::PlaySound { path, volume } => {
                    if !path.trim().is_empty() {
                        commands.push(RuntimeCommand::PlaySound {
                            owner_id,
                            path: path.clone(),
                            volume: *volume,
                        });
                    }
                }
                BlueprintNodeKind::StopSound => {
                    commands.push(RuntimeCommand::StopSound { owner_id });
                }
                BlueprintNodeKind::SwitchAnimation { animation_path } => {
                    if !animation_path.trim().is_empty()
                        && game_object.animation_path != *animation_path
                    {
                        game_object.animation_path = animation_path.clone();
                        game_object.animation_playing = true;
                    }
                }
                BlueprintNodeKind::PauseAnimation => {
                    game_object.animation_playing = false;
                }
                BlueprintNodeKind::PlayAnimation => {
                    game_object.animation_playing = true;
                }
                BlueprintNodeKind::DetectTile { variable_name } => {
                    if !variable_name.trim().is_empty() {
                        let tile_id = input
                            .tiles_under_objects
                            .get(&game_object.id)
                            .copied()
                            .unwrap_or(-1);
                        game_object
                            .variables
                            .insert(variable_name.clone(), tile_id as f32);
                    }
                }
                BlueprintNodeKind::SetUiText { ui_id, content } => {
                    commands.push(RuntimeCommand::Ui(UiCommand::SetText {
                        ui_id: *ui_id,
                        content: content.clone(),
                    }));
                }
                BlueprintNodeKind::SetUiProgress { ui_id, value } => {
                    commands.push(RuntimeCommand::Ui(UiCommand::SetProgress {
                        ui_id: *ui_id,
                        value: *value,
                    }));
                }
                BlueprintNodeKind::SetUiVisible { ui_id, visible } => {
                    commands.push(RuntimeCommand::Ui(UiCommand::SetVisible {
                        ui_id: *ui_id,
                        visible: *visible,
                    }));
                }
                BlueprintNodeKind::IfCondition {
                    variable_name,
                    comparison,
                    compare_value,
                    use_global,
                } => {
                    let value = if *use_global {
                        global_variables.get(variable_name)
                    } else {
                        game_object.variables.get(variable_name)
                    }
                    .copied()
                    .unwrap_or(0.0);
                    output_port = if comparison.compare(value, *compare_value) {
                        1
                    } else {
                        2
                    };
                }
                BlueprintNodeKind::CompareVariables {
                    left_name,
                    right_name,
                    comparison,
                    use_global,
                } => {
                    let variables = if *use_global {
                        &*global_variables
                    } else {
                        &game_object.variables
                    };
                    let left = variables.get(left_name).copied().unwrap_or(0.0);
                    let right = variables.get(right_name).copied().unwrap_or(0.0);
                    output_port = if comparison.compare(left, right) {
                        1
                    } else {
                        2
                    };
                }
                BlueprintNodeKind::SwitchScene { scene_name } => {
                    if !scene_name.trim().is_empty() {
                        commands.push(RuntimeCommand::SwitchScene(scene_name.clone()));
                    }
                }
                BlueprintNodeKind::SpawnObject {
                    template_object_id,
                    x,
                    y,
                } => {
                    commands.push(RuntimeCommand::SpawnObject {
                        template_object_id: *template_object_id,
                        x: *x,
                        y: *y,
                    });
                }
                BlueprintNodeKind::DestroyObject { object_id } => {
                    commands.push(RuntimeCommand::DestroyObject(if *object_id == 0 {
                        game_object.id
                    } else {
                        *object_id
                    }));
                }
                BlueprintNodeKind::SetGlobalVariable { name, value } => {
                    if !name.trim().is_empty() {
                        global_variables.insert(name.clone(), *value);
                    }
                }
                _ => {}
            }
            queue.extend(plan.outgoing_nodes(node_id, output_port));
        }
        runtime_state
            .instances
            .entry(owner_id)
            .or_default()
            .variables
            .clone_from(&game_object.variables);
        game_object.blueprint = blueprint;
    }
    commands
}

/// 仅让纯点击或碰撞事件蓝图在视口外休眠，保留所有持续逻辑语义。
fn should_sleep_blueprint(object_id: u64, blueprint: &Blueprint, input: &BlueprintInput) -> bool {
    if !input.dormant_blueprints_enabled
        || input.active_object_ids.contains(&object_id)
        || input.clicked_object_ids.contains(&object_id)
        || input.collision_objects.contains(&object_id)
    {
        return false;
    }
    !blueprint.nodes.iter().any(|node| {
        matches!(
            node.kind,
            BlueprintNodeKind::FrameUpdated
                | BlueprintNodeKind::Timer { .. }
                | BlueprintNodeKind::KeyPressed { .. }
                | BlueprintNodeKind::SceneLoaded
                | BlueprintNodeKind::PluginNode {
                    category: PluginNodeCategory::Event,
                    ..
                }
        )
    })
}

/// 使用临时全局状态执行蓝图，保留阶段1-8代码和测试使用的简单接口。
#[allow(dead_code)]
pub fn update_blueprints(
    game_objects: &mut [GameObject],
    input: &BlueprintInput,
    delta_time: f32,
) -> Vec<UiCommand> {
    let mut globals = HashMap::new();
    let mut state = BlueprintRuntimeState::new();
    update_blueprints_with_state(game_objects, input, delta_time, &mut globals, &mut state)
        .into_iter()
        .filter_map(|command| match command {
            RuntimeCommand::Ui(command) => Some(command),
            _ => None,
        })
        .collect()
}

/// 执行所有UI元素自己的蓝图，并返回其中产生的UI修改命令。
///
/// UI蓝图主要用于“按钮点击 -> 修改文本/进度条/显隐”逻辑。
#[allow(dead_code)]
pub fn update_ui_blueprints(elements: &[UiElement], input: &BlueprintInput) -> Vec<UiCommand> {
    let mut commands = Vec::new();
    let mut runtime_state = BlueprintRuntimeState::new();
    for element in elements {
        for connection in &element.blueprint.connections {
            let event = element
                .blueprint
                .nodes
                .iter()
                .find(|node| node.id == connection.from_node_id);
            let action = element
                .blueprint
                .nodes
                .iter()
                .find(|node| node.id == connection.to_node_id);
            let (event, action) = match (event, action) {
                (Some(event), Some(action)) => (event, action),
                _ => continue,
            };
            if !event_is_active_with_state(&event.kind, input, 0, event.id, 0.0, &mut runtime_state)
            {
                continue;
            }
            match &action.kind {
                BlueprintNodeKind::SetUiText { ui_id, content } => {
                    commands.push(UiCommand::SetText {
                        ui_id: *ui_id,
                        content: content.clone(),
                    });
                }
                BlueprintNodeKind::SetUiProgress { ui_id, value } => {
                    commands.push(UiCommand::SetProgress {
                        ui_id: *ui_id,
                        value: *value,
                    });
                }
                BlueprintNodeKind::SetUiVisible { ui_id, visible } => {
                    commands.push(UiCommand::SetVisible {
                        ui_id: *ui_id,
                        visible: *visible,
                    });
                }
                _ => {}
            }
        }
    }
    commands
}

/// 使用完整高级执行链运行UI蓝图，使按钮也能切换场景和修改全局变量。
pub fn update_ui_blueprints_with_state(
    elements: &[UiElement],
    input: &BlueprintInput,
    delta_time: f32,
    global_variables: &mut HashMap<String, f32>,
    runtime_state: &mut BlueprintRuntimeState,
) -> Vec<RuntimeCommand> {
    let mut commands = Vec::new();
    for element in elements {
        let mut virtual_object = GameObject {
            id: ui_blueprint_owner_id(element.id),
            x: element.x,
            y: element.y,
            width: element.width,
            height: element.height,
            layer_index: element.layer_index,
            image_path: String::new(),
            audio_path: String::new(),
            animation_path: String::new(),
            animation_playing: true,
            collider: None,
            blueprint: element.blueprint.clone(),
            blueprint_file: String::new(),
            variables: HashMap::new(),
            persistent: false,
        };
        commands.extend(update_blueprints_with_state(
            std::slice::from_mut(&mut virtual_object),
            input,
            delta_time,
            global_variables,
            runtime_state,
        ));
    }
    commands
}

/// 将场景UI ID映射到与Actor ID隔离的蓝图owner ID。
pub fn ui_blueprint_owner_id(ui_id: u64) -> u64 {
    u64::MAX - ui_id
}

/// 判断事件节点在当前帧是否被触发。
fn event_is_active_with_state(
    kind: &BlueprintNodeKind,
    input: &BlueprintInput,
    object_id: u64,
    node_id: u64,
    delta_time: f32,
    runtime_state: &mut BlueprintRuntimeState,
) -> bool {
    match kind {
        BlueprintNodeKind::FrameUpdated => true,
        BlueprintNodeKind::KeyPressed { key } => input.is_key_down(key),
        BlueprintNodeKind::CollisionTriggered => input.collision_objects.contains(&object_id),
        BlueprintNodeKind::ButtonClicked { ui_id } => input.clicked_ui_ids.contains(ui_id),
        BlueprintNodeKind::ObjectClicked => input.clicked_object_ids.contains(&object_id),
        BlueprintNodeKind::SceneLoaded => input.scene_just_loaded,
        BlueprintNodeKind::Timer {
            delay_seconds,
            repeat,
        } => {
            let delay = delay_seconds.max(0.001);
            let elapsed = runtime_state
                .instances
                .entry(object_id)
                .or_default()
                .timers
                .entry(node_id)
                .or_insert(0.0);
            *elapsed += delta_time;
            if *elapsed >= delay {
                if *repeat {
                    *elapsed -= delay;
                } else {
                    *elapsed = f32::NEG_INFINITY;
                }
                true
            } else {
                false
            }
        }
        BlueprintNodeKind::PluginNode {
            plugin_id,
            node_type,
            category,
            behavior,
            ..
        } => {
            if !input.plugin_node_is_authorized(plugin_id, node_type, behavior)
                || *category != PluginNodeCategory::Event
            {
                return false;
            }
            match behavior {
                PluginBehavior::SceneLoadedEvent => input.scene_just_loaded,
                PluginBehavior::ObjectClickedEvent => input.clicked_object_ids.contains(&object_id),
                _ => false,
            }
        }
        _ => false,
    }
}

/// 判断无状态事件是否激活，供旧执行路径和单元测试使用。
#[allow(dead_code)]
fn event_is_active(kind: &BlueprintNodeKind, input: &BlueprintInput, object_id: u64) -> bool {
    let mut state = BlueprintRuntimeState::new();
    event_is_active_with_state(kind, input, object_id, 0, 0.0, &mut state)
}

/// 根据变量节点声明，为当前物体创建尚不存在的初始变量。
fn initialize_variables(
    variables: &mut HashMap<String, f32>,
    nodes: &[crate::blueprint::model::BlueprintNode],
    input: &BlueprintInput,
) {
    for node in nodes {
        if let BlueprintNodeKind::NumberVariable {
            name,
            initial_value,
        } = &node.kind
        {
            if !name.trim().is_empty() {
                variables.entry(name.clone()).or_insert(*initial_value);
            }
        }
        if let BlueprintNodeKind::PluginNode {
            plugin_id,
            node_type,
            behavior: PluginBehavior::NumberVariable,
            variable_name,
            value,
            ..
        } = &node.kind
        {
            if input.plugin_node_is_authorized(
                plugin_id,
                node_type,
                &PluginBehavior::NumberVariable,
            ) && !variable_name.trim().is_empty()
            {
                variables.entry(variable_name.clone()).or_insert(*value);
            }
        }
    }
}

/// 根据全局变量声明创建尚不存在的工程级变量。
fn initialize_global_variables(
    variables: &mut HashMap<String, f32>,
    nodes: &[crate::blueprint::model::BlueprintNode],
) {
    for node in nodes {
        if let BlueprintNodeKind::GlobalNumberVariable {
            name,
            initial_value,
        } = &node.kind
        {
            if !name.trim().is_empty() {
                variables.entry(name.clone()).or_insert(*initial_value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blueprint::model::{
        Blueprint, BlueprintConnection, BlueprintNode, BlueprintNodeKind,
    };
    use crate::game_ui::{UiElement, UiElementKind};

    /// 创建供插件和高级蓝图测试使用的简单游戏物体。
    fn test_object_with_blueprint(blueprint: Blueprint) -> GameObject {
        GameObject {
            id: 1,
            x: 0.0,
            y: 0.0,
            width: 32.0,
            height: 32.0,
            layer_index: 0,
            image_path: String::new(),
            audio_path: String::new(),
            animation_path: String::new(),
            animation_playing: true,
            collider: None,
            blueprint,
            blueprint_file: "blueprint_1.json".to_owned(),
            variables: HashMap::new(),
            persistent: false,
        }
    }

    /// 验证W键事件通过连线执行坐标修改节点。
    #[test]
    fn w_key_moves_object_up() {
        let blueprint = Blueprint {
            slide2d_engine: "SLIDE2D_BLUEPRINT".to_owned(),
            nodes: vec![
                BlueprintNode {
                    id: 1,
                    x: 0.0,
                    y: 0.0,
                    kind: BlueprintNodeKind::KeyPressed {
                        key: "KeyW".to_owned(),
                    },
                },
                BlueprintNode {
                    id: 2,
                    x: 250.0,
                    y: 0.0,
                    kind: BlueprintNodeKind::ModifyPosition {
                        delta_x: 0.0,
                        delta_y: -200.0,
                    },
                },
            ],
            connections: vec![BlueprintConnection {
                from_node_id: 1,
                to_node_id: 2,
                from_port: 0,
            }],
            next_node_id: 3,
        };
        let mut game_objects = vec![GameObject {
            id: 1,
            x: 100.0,
            y: 100.0,
            width: 50.0,
            height: 50.0,
            layer_index: 0,
            image_path: String::new(),
            audio_path: String::new(),
            animation_path: String::new(),
            animation_playing: true,
            collider: None,
            blueprint,
            blueprint_file: "blueprint_1.json".to_owned(),
            variables: std::collections::HashMap::new(),
            persistent: false,
        }];

        let mut input = BlueprintInput::new();
        input.set_key_down("KeyW".to_owned(), true);
        update_blueprints(&mut game_objects, &input, 1.0);

        assert_eq!(game_objects[0].y, -100.0);
    }

    /// 验证不同键盘节点只响应各自配置的按键。
    #[test]
    fn configured_key_controls_event() {
        let mut input = BlueprintInput::new();
        input.set_key_down("Space".to_owned(), true);

        assert!(event_is_active(
            &BlueprintNodeKind::KeyPressed {
                key: "Space".to_owned()
            },
            &input,
            1,
        ));
        assert!(!event_is_active(
            &BlueprintNodeKind::KeyPressed {
                key: "KeyW".to_owned()
            },
            &input,
            1,
        ));
    }

    /// 验证帧更新事件可以设置当前物体中的数值变量。
    #[test]
    fn frame_updated_sets_declared_variable() {
        let blueprint = Blueprint {
            slide2d_engine: "SLIDE2D_BLUEPRINT".to_owned(),
            nodes: vec![
                BlueprintNode {
                    id: 1,
                    x: 0.0,
                    y: 0.0,
                    kind: BlueprintNodeKind::FrameUpdated,
                },
                BlueprintNode {
                    id: 2,
                    x: 200.0,
                    y: 0.0,
                    kind: BlueprintNodeKind::SetVariable {
                        name: "score".to_owned(),
                        value: 10.0,
                    },
                },
                BlueprintNode {
                    id: 3,
                    x: 400.0,
                    y: 0.0,
                    kind: BlueprintNodeKind::NumberVariable {
                        name: "score".to_owned(),
                        initial_value: 0.0,
                    },
                },
            ],
            connections: vec![BlueprintConnection {
                from_node_id: 1,
                to_node_id: 2,
                from_port: 0,
            }],
            next_node_id: 4,
        };
        let mut object = GameObject {
            id: 1,
            x: 0.0,
            y: 0.0,
            width: 50.0,
            height: 50.0,
            layer_index: 0,
            image_path: String::new(),
            audio_path: String::new(),
            animation_path: String::new(),
            animation_playing: true,
            collider: None,
            blueprint,
            blueprint_file: "blueprint_1.json".to_owned(),
            variables: std::collections::HashMap::new(),
            persistent: false,
        };

        update_blueprints(
            std::slice::from_mut(&mut object),
            &BlueprintInput::new(),
            1.0 / 60.0,
        );

        assert_eq!(object.variables.get("score"), Some(&10.0));
    }

    /// 验证按钮点击事件只在指定UI ID被点击时输出UI命令。
    #[test]
    fn button_click_event_emits_ui_command() {
        let blueprint = Blueprint {
            slide2d_engine: "SLIDE2D_BLUEPRINT".to_owned(),
            nodes: vec![
                BlueprintNode {
                    id: 1,
                    x: 0.0,
                    y: 0.0,
                    kind: BlueprintNodeKind::ButtonClicked { ui_id: 7 },
                },
                BlueprintNode {
                    id: 2,
                    x: 200.0,
                    y: 0.0,
                    kind: BlueprintNodeKind::SetUiText {
                        ui_id: 8,
                        content: "100分".to_owned(),
                    },
                },
            ],
            connections: vec![BlueprintConnection {
                from_node_id: 1,
                to_node_id: 2,
                from_port: 0,
            }],
            next_node_id: 3,
        };
        let mut object = GameObject {
            id: 1,
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
            layer_index: 0,
            image_path: String::new(),
            audio_path: String::new(),
            animation_path: String::new(),
            animation_playing: true,
            collider: None,
            blueprint,
            blueprint_file: String::new(),
            variables: HashMap::new(),
            persistent: false,
        };
        let mut input = BlueprintInput::new();
        input.clicked_ui_ids.insert(7);
        let commands = update_blueprints(std::slice::from_mut(&mut object), &input, 0.0);

        assert_eq!(commands.len(), 1);
        assert!(matches!(
            &commands[0],
            UiCommand::SetText { ui_id: 8, content } if content == "100分"
        ));
    }

    /// 验证If节点只沿满足条件的端口继续执行。
    #[test]
    fn if_node_uses_true_branch() {
        let mut blueprint = Blueprint::new();
        blueprint.add_frame_updated_node();
        blueprint.add_kind(BlueprintNodeKind::IfCondition {
            variable_name: "score".to_owned(),
            comparison: crate::blueprint::model::ComparisonOperator::GreaterOrEqual,
            compare_value: 10.0,
            use_global: true,
        });
        blueprint.add_kind(BlueprintNodeKind::SetGlobalVariable {
            name: "passed".to_owned(),
            value: 1.0,
        });
        blueprint.add_kind(BlueprintNodeKind::SetGlobalVariable {
            name: "failed".to_owned(),
            value: 1.0,
        });
        blueprint.connect_from_port(1, 0, 2);
        blueprint.connect_from_port(2, 1, 3);
        blueprint.connect_from_port(2, 2, 4);
        let mut object = GameObject {
            id: 1,
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
            layer_index: 0,
            image_path: String::new(),
            audio_path: String::new(),
            animation_path: String::new(),
            animation_playing: true,
            collider: None,
            blueprint,
            blueprint_file: String::new(),
            variables: HashMap::new(),
            persistent: false,
        };
        let mut globals = HashMap::from([("score".to_owned(), 20.0)]);
        let mut runtime_state = BlueprintRuntimeState::new();
        update_blueprints_with_state(
            std::slice::from_mut(&mut object),
            &BlueprintInput::new(),
            0.0,
            &mut globals,
            &mut runtime_state,
        );
        assert_eq!(globals.get("passed"), Some(&1.0));
        assert!(!globals.contains_key("failed"));
    }

    /// 验证官方拾取插件节点仅在插件启用且Actor被点击时走成功分支。
    #[test]
    fn pickup_plugin_node_uses_success_branch() {
        let mut blueprint = Blueprint::new();
        blueprint.add_frame_updated_node();
        blueprint.add_kind(BlueprintNodeKind::PluginNode {
            plugin_id: crate::plugins::OFFICIAL_PICKUP_PLUGIN_ID.to_owned(),
            node_type: "pickup_check".to_owned(),
            display_name: "拾取判定".to_owned(),
            description: "测试".to_owned(),
            category: PluginNodeCategory::Logic,
            behavior: PluginBehavior::PickupCheck,
            variable_name: "picked".to_owned(),
            value: 1.0,
        });
        blueprint.add_kind(BlueprintNodeKind::SetVariable {
            name: "success".to_owned(),
            value: 1.0,
        });
        blueprint.connect_from_port(1, 0, 2);
        blueprint.connect_from_port(2, 1, 3);
        let mut object = test_object_with_blueprint(blueprint);
        let mut input = BlueprintInput::new();
        input.clicked_object_ids.insert(object.id);
        let root = unique_plugin_test_directory("pickup_authorized");
        let registry = PluginRegistry::load_new_project(root.clone());
        input.authorize_plugins(&registry);
        let mut globals = HashMap::new();
        let mut state = BlueprintRuntimeState::new();
        update_blueprints_with_state(
            std::slice::from_mut(&mut object),
            &input,
            0.016,
            &mut globals,
            &mut state,
        );
        assert_eq!(object.variables.get("picked"), Some(&1.0));
        assert_eq!(object.variables.get("success"), Some(&1.0));
        let _ = std::fs::remove_dir_all(root);
    }

    /// 蓝图不能把manifest中的拾取行为伪造成更高权限的全局变量写入行为。
    #[test]
    fn forged_plugin_behavior_is_rejected() {
        let mut blueprint = Blueprint::new();
        blueprint.add_frame_updated_node();
        blueprint.add_kind(BlueprintNodeKind::PluginNode {
            plugin_id: crate::plugins::OFFICIAL_PICKUP_PLUGIN_ID.to_owned(),
            node_type: "pickup_check".to_owned(),
            display_name: "伪造节点".to_owned(),
            description: "测试".to_owned(),
            category: PluginNodeCategory::Action,
            behavior: PluginBehavior::SetGlobalVariable,
            variable_name: "forged".to_owned(),
            value: 99.0,
        });
        blueprint.connect(1, 2);
        let mut object = test_object_with_blueprint(blueprint);
        let root = unique_plugin_test_directory("forged_behavior");
        let registry = PluginRegistry::load_new_project(root.clone());
        let mut input = BlueprintInput::new();
        input.authorize_plugins(&registry);
        let mut globals = HashMap::new();
        let mut state = BlueprintRuntimeState::new();

        update_blueprints_with_state(
            std::slice::from_mut(&mut object),
            &input,
            0.016,
            &mut globals,
            &mut state,
        );

        assert!(!globals.contains_key("forged"));
        let _ = std::fs::remove_dir_all(root);
    }

    /// 插件事件同样必须匹配manifest，不能仅依赖蓝图内嵌behavior触发。
    #[test]
    fn forged_plugin_event_behavior_is_rejected() {
        let mut blueprint = Blueprint::new();
        blueprint.add_kind(BlueprintNodeKind::PluginNode {
            plugin_id: crate::plugins::OFFICIAL_PICKUP_PLUGIN_ID.to_owned(),
            node_type: "pickup_check".to_owned(),
            display_name: "伪造事件".to_owned(),
            description: "测试".to_owned(),
            category: PluginNodeCategory::Event,
            behavior: PluginBehavior::SceneLoadedEvent,
            variable_name: String::new(),
            value: 0.0,
        });
        blueprint.add_kind(BlueprintNodeKind::SetVariable {
            name: "triggered".to_owned(),
            value: 1.0,
        });
        blueprint.connect(1, 2);
        let mut object = test_object_with_blueprint(blueprint);
        let root = unique_plugin_test_directory("forged_event");
        let registry = PluginRegistry::load_new_project(root.clone());
        let mut input = BlueprintInput::new();
        input.authorize_plugins(&registry);

        update_blueprints(std::slice::from_mut(&mut object), &input, 0.016);

        assert!(!object.variables.contains_key("triggered"));
        let _ = std::fs::remove_dir_all(root);
    }

    /// 验证包含帧更新事件的物体即使在视口外也不会被错误休眠。
    #[test]
    fn frame_updated_blueprint_never_sleeps() {
        let mut blueprint = Blueprint::new();
        blueprint.add_frame_updated_node();
        blueprint.add_kind(BlueprintNodeKind::SetVariable {
            name: "ran".to_owned(),
            value: 1.0,
        });
        blueprint.connect_from_port(1, 0, 2);
        let mut object = test_object_with_blueprint(blueprint);
        let mut input = BlueprintInput::new();
        input.dormant_blueprints_enabled = true;
        let mut globals = HashMap::new();
        let mut state = BlueprintRuntimeState::new();
        update_blueprints_with_state(
            std::slice::from_mut(&mut object),
            &input,
            0.016,
            &mut globals,
            &mut state,
        );
        assert_eq!(object.variables.get("ran"), Some(&1.0));
    }

    #[test]
    fn audio_nodes_emit_owner_scoped_commands() {
        let mut blueprint = Blueprint::new();
        blueprint.add_frame_updated_node();
        blueprint.add_kind(BlueprintNodeKind::PlaySound {
            path: "audio/jump.wav".to_owned(),
            volume: 0.5,
        });
        blueprint.add_kind(BlueprintNodeKind::StopSound);
        blueprint.connect(1, 2);
        blueprint.connect(2, 3);
        let mut object = test_object_with_blueprint(blueprint);
        let mut globals = HashMap::new();
        let mut state = BlueprintRuntimeState::new();

        let commands = update_blueprints_with_state(
            std::slice::from_mut(&mut object),
            &BlueprintInput::new(),
            0.016,
            &mut globals,
            &mut state,
        );

        assert!(commands.iter().any(|command| matches!(
            command,
            RuntimeCommand::PlaySound { owner_id: 1, path, volume }
                if path == "audio/jump.wav" && *volume == 0.5
        )));
        assert!(commands
            .iter()
            .any(|command| matches!(command, RuntimeCommand::StopSound { owner_id: 1 })));
    }

    #[test]
    fn ui_blueprint_variables_persist_between_frames() {
        let mut blueprint = Blueprint::new();
        blueprint.add_kind(BlueprintNodeKind::ButtonClicked { ui_id: 7 });
        blueprint.add_kind(BlueprintNodeKind::SetVariable {
            name: "clicks".to_owned(),
            value: 1.0,
        });
        blueprint.add_kind(BlueprintNodeKind::NumberVariable {
            name: "clicks".to_owned(),
            initial_value: 0.0,
        });
        blueprint.connect(1, 2);
        let element = UiElement {
            id: 7,
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 30.0,
            layer_index: 0,
            visible: true,
            kind: UiElementKind::Button {
                text: "test".to_owned(),
            },
            blueprint,
            persistent: false,
        };
        let mut input = BlueprintInput::new();
        input.clicked_ui_ids.insert(7);
        let mut globals = HashMap::new();
        let mut state = BlueprintRuntimeState::new();
        update_ui_blueprints_with_state(
            std::slice::from_ref(&element),
            &input,
            0.016,
            &mut globals,
            &mut state,
        );
        input.clicked_ui_ids.clear();
        update_ui_blueprints_with_state(
            std::slice::from_ref(&element),
            &input,
            0.016,
            &mut globals,
            &mut state,
        );

        assert_eq!(
            state
                .variables(ui_blueprint_owner_id(element.id))
                .and_then(|variables| variables.get("clicks")),
            Some(&1.0)
        );
    }

    #[test]
    fn execution_limit_is_queryable_as_diagnostic() {
        let mut blueprint = Blueprint::new();
        blueprint.add_frame_updated_node();
        blueprint.add_kind(BlueprintNodeKind::ModifyPosition {
            delta_x: 0.0,
            delta_y: 0.0,
        });
        blueprint.add_kind(BlueprintNodeKind::ModifyPosition {
            delta_x: 0.0,
            delta_y: 0.0,
        });
        blueprint.connect(1, 2);
        blueprint.connect(2, 3);
        blueprint.connect(3, 2);
        let mut object = test_object_with_blueprint(blueprint);
        let mut globals = HashMap::new();
        let mut state = BlueprintRuntimeState::new();
        update_blueprints_with_state(
            std::slice::from_mut(&mut object),
            &BlueprintInput::new(),
            0.016,
            &mut globals,
            &mut state,
        );

        assert!(state
            .diagnostics(object.id)
            .iter()
            .any(|diagnostic| diagnostic.code == "execution_limit_exceeded"));
    }

    fn unique_plugin_test_directory(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "slide2d_vm_{label}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ))
    }
}
