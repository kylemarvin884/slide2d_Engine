use egui::epaint::CubicBezierShape;
use egui::{Color32, Id, Pos2, Rect, Sense, Stroke, Vec2};

use crate::app_state::AppState;
use crate::blueprint::model::{
    canonical_key_name, key_display_name, Blueprint, BlueprintNodeCategory, BlueprintNodeKind,
    ComparisonOperator, AVAILABLE_KEYS,
};
use crate::game_ui::UiElementKind;
use crate::localization::{localize_message, tr, tr_args};

const NODE_WIDTH: f32 = 210.0;
const NODE_HEIGHT: f32 = 90.0;
const PORT_RADIUS: f32 = 7.0;
const PORT_SNAP_DISTANCE: f32 = 36.0;
const MIN_BLUEPRINT_ZOOM: f32 = 0.35;
const MAX_BLUEPRINT_ZOOM: f32 = 2.0;

/// 当前蓝图所属对象的类型，用于过滤不适用的新增节点。
#[derive(Clone, Copy, PartialEq)]
enum BlueprintOwnerKind {
    WorldObject,
    ImageObject,
    AudioObject,
    UiText,
    UiButton,
    UiProgressBar,
    UiImagePanel,
}

impl BlueprintOwnerKind {
    /// 返回蓝图工具栏中显示的持有者类型名称。
    fn display_name(self) -> String {
        match self {
            Self::WorldObject => tr("owner.world"),
            Self::ImageObject => tr("owner.image"),
            Self::AudioObject => tr("owner.audio"),
            Self::UiText => tr("owner.ui_text"),
            Self::UiButton => tr("owner.ui_button"),
            Self::UiProgressBar => tr("owner.ui_progress"),
            Self::UiImagePanel => tr("owner.ui_image"),
        }
    }

    /// 判断当前是否为任意UI元素蓝图。
    fn is_ui(self) -> bool {
        matches!(
            self,
            Self::UiText | Self::UiButton | Self::UiProgressBar | Self::UiImagePanel
        )
    }
}

/// 在给定egui区域中绘制蓝图内容，可供IDE标签页和独立窗口共同使用。
pub fn draw_blueprint_contents(ui: &mut egui::Ui, app_state: &mut AppState) {
    let owner_ui_id = app_state.blueprint_ui_id;
    let owner_kind = if let Some(ui_id) = app_state.blueprint_ui_id {
        match app_state
            .ui_elements
            .iter()
            .find(|element| element.id == ui_id)
            .map(|element| &element.kind)
        {
            Some(UiElementKind::Text { .. }) => BlueprintOwnerKind::UiText,
            Some(UiElementKind::Button { .. }) => BlueprintOwnerKind::UiButton,
            Some(UiElementKind::ProgressBar { .. }) => BlueprintOwnerKind::UiProgressBar,
            Some(UiElementKind::ImagePanel { .. }) => BlueprintOwnerKind::UiImagePanel,
            None => {
                app_state.close_blueprint_owner();
                return;
            }
        }
    } else if let Some(object_id) = app_state.blueprint_object_id {
        match app_state
            .game_objects
            .iter()
            .find(|object| object.id == object_id)
        {
            Some(object) if !object.audio_path.is_empty() => BlueprintOwnerKind::AudioObject,
            Some(object) if !object.image_path.is_empty() || !object.animation_path.is_empty() => {
                BlueprintOwnerKind::ImageObject
            }
            Some(_) => BlueprintOwnerKind::WorldObject,
            None => {
                app_state.close_blueprint_owner();
                return;
            }
        }
    } else {
        ui.centered_and_justified(|ui| {
            ui.label(tr("blueprint.open_hint"));
        });
        return;
    };

    let mut pending_output = app_state.pending_blueprint_output;
    let mut selected_node_id = app_state.selected_blueprint_node_id;
    let mut selected_node_ids = app_state.selected_blueprint_node_ids.clone();
    let mut pending_output_port = app_state.pending_blueprint_output_port;
    let mut view_offset = Vec2::new(
        app_state.blueprint_view_offset_x,
        app_state.blueprint_view_offset_y,
    );
    let mut view_zoom = app_state.blueprint_view_zoom;
    let plugin_nodes = app_state.plugin_registry.enabled_nodes();
    let blueprint = if let Some(ui_id) = app_state.blueprint_ui_id {
        let index = match app_state
            .ui_elements
            .iter()
            .position(|element| element.id == ui_id)
        {
            Some(index) => index,
            None => {
                app_state.close_blueprint_owner();
                return;
            }
        };
        &mut app_state.ui_elements[index].blueprint
    } else if let Some(object_id) = app_state.blueprint_object_id {
        let index = match app_state
            .game_objects
            .iter()
            .position(|object| object.id == object_id)
        {
            Some(index) => index,
            None => {
                app_state.close_blueprint_owner();
                return;
            }
        };
        &mut app_state.game_objects[index].blueprint
    } else {
        ui.centered_and_justified(|ui| {
            ui.label(tr("blueprint.open_hint"));
        });
        return;
    };
    // 节点画布首先占用整个剩余工作区，工具栏和属性窗口都作为覆盖层绘制。
    let canvas_rect = draw_node_canvas(
        ui,
        blueprint,
        &mut pending_output,
        &mut pending_output_port,
        &mut selected_node_id,
        &mut selected_node_ids,
        &mut view_offset,
        &mut view_zoom,
    );
    let reset_view = draw_floating_toolbar(
        ui.ctx(),
        blueprint,
        view_zoom,
        canvas_rect,
        owner_kind,
        owner_ui_id,
        &mut selected_node_ids,
        &plugin_nodes,
    );
    if reset_view {
        view_offset = Vec2::ZERO;
        view_zoom = 1.0;
    }
    draw_floating_node_properties(ui.ctx(), blueprint, selected_node_id, canvas_rect);
    app_state.pending_blueprint_output = pending_output;
    app_state.pending_blueprint_output_port = pending_output_port;
    app_state.selected_blueprint_node_id = selected_node_id;
    app_state.selected_blueprint_node_ids = selected_node_ids;
    app_state.blueprint_view_offset_x = view_offset.x;
    app_state.blueprint_view_offset_y = view_offset.y;
    app_state.blueprint_view_zoom = view_zoom;
}

/// 绘制添加节点和清除连线的工具按钮。
fn draw_toolbar(
    ui: &mut egui::Ui,
    blueprint: &mut Blueprint,
    view_zoom: f32,
    owner_kind: BlueprintOwnerKind,
    owner_ui_id: Option<u64>,
    selected_node_ids: &mut Vec<u64>,
    plugin_nodes: &[(String, crate::plugins::PluginNodeDefinition)],
) -> bool {
    let mut reset_view = false;
    ui.horizontal(|ui| {
        ui.label(tr_args(
            "blueprint.current",
            &[("value", owner_kind.display_name())],
        ));
        ui.menu_button(tr("blueprint.event_nodes"), |ui| {
            if ui.button(tr("node.frame_updated")).clicked() {
                blueprint.add_frame_updated_node();
                ui.close_menu();
            }
            if ui.button(tr("node.key_pressed")).clicked() {
                blueprint.add_key_pressed_node();
                ui.close_menu();
            }
            if ui.button(tr("node.timer")).clicked() {
                blueprint.add_kind(BlueprintNodeKind::Timer {
                    delay_seconds: 1.0,
                    repeat: false,
                });
                ui.close_menu();
            }
            if ui.button(tr("node.object_clicked")).clicked() {
                blueprint.add_kind(BlueprintNodeKind::ObjectClicked);
                ui.close_menu();
            }
            if ui.button(tr("node.scene_loaded")).clicked() {
                blueprint.add_kind(BlueprintNodeKind::SceneLoaded);
                ui.close_menu();
            }
            if matches!(
                owner_kind,
                BlueprintOwnerKind::WorldObject | BlueprintOwnerKind::ImageObject
            ) {
                if ui.button(tr("node.collision")).clicked() {
                    blueprint.add_collision_triggered_node();
                    ui.close_menu();
                }
            }
            if owner_kind == BlueprintOwnerKind::UiButton {
                if ui.button(tr("node.button_clicked")).clicked() {
                    blueprint.add_button_clicked_node();
                    set_last_ui_node_target(blueprint, owner_ui_id);
                    ui.close_menu();
                }
            }
        });
        ui.menu_button(tr("blueprint.logic_nodes"), |ui| {
            if ui.button(tr("node.if")).clicked() {
                blueprint.add_kind(BlueprintNodeKind::IfCondition {
                    variable_name: "score".to_owned(),
                    comparison: ComparisonOperator::GreaterOrEqual,
                    compare_value: 100.0,
                    use_global: true,
                });
                ui.close_menu();
            }
            if ui.button(tr("node.compare_variables")).clicked() {
                blueprint.add_kind(BlueprintNodeKind::CompareVariables {
                    left_name: "score".to_owned(),
                    right_name: "target".to_owned(),
                    comparison: ComparisonOperator::GreaterOrEqual,
                    use_global: true,
                });
                ui.close_menu();
            }
        });
        ui.menu_button(tr("blueprint.action_nodes"), |ui| {
            match owner_kind {
                BlueprintOwnerKind::WorldObject => {
                    if ui.button(tr("node.move")).clicked() {
                        blueprint.add_modify_position_node();
                        ui.close_menu();
                    }
                    if ui.button(tr("node.detect_tile")).clicked() {
                        blueprint.add_detect_tile_node();
                        ui.close_menu();
                    }
                }
                BlueprintOwnerKind::ImageObject => {
                    if ui.button(tr("node.move")).clicked() {
                        blueprint.add_modify_position_node();
                        ui.close_menu();
                    }
                    if ui.button(tr("node.switch_animation")).clicked() {
                        blueprint.add_switch_animation_node();
                        ui.close_menu();
                    }
                    if ui.button(tr("node.pause_animation")).clicked() {
                        blueprint.add_pause_animation_node();
                        ui.close_menu();
                    }
                    if ui.button(tr("node.play_animation")).clicked() {
                        blueprint.add_play_animation_node();
                        ui.close_menu();
                    }
                    if ui.button(tr("node.detect_tile")).clicked() {
                        blueprint.add_detect_tile_node();
                        ui.close_menu();
                    }
                }
                BlueprintOwnerKind::AudioObject => {
                    if ui.button(tr("node.play_sound")).clicked() {
                        blueprint.add_play_sound_node();
                        ui.close_menu();
                    }
                    if ui.button(tr("node.stop_sound")).clicked() {
                        blueprint.add_stop_sound_node();
                        ui.close_menu();
                    }
                }
                BlueprintOwnerKind::UiText => {
                    if ui.button(tr("node.set_text")).clicked() {
                        blueprint.add_set_ui_text_node();
                        set_last_ui_node_target(blueprint, owner_ui_id);
                        ui.close_menu();
                    }
                    if ui.button(tr("node.set_visible")).clicked() {
                        blueprint.add_set_ui_visible_node();
                        set_last_ui_node_target(blueprint, owner_ui_id);
                        ui.close_menu();
                    }
                }
                BlueprintOwnerKind::UiButton => {
                    if ui.button(tr("blueprint.set_button_text")).clicked() {
                        blueprint.add_set_ui_text_node();
                        set_last_ui_node_target(blueprint, owner_ui_id);
                        ui.close_menu();
                    }
                    if ui.button(tr("node.set_visible")).clicked() {
                        blueprint.add_set_ui_visible_node();
                        set_last_ui_node_target(blueprint, owner_ui_id);
                        ui.close_menu();
                    }
                }
                BlueprintOwnerKind::UiProgressBar => {
                    if ui.button(tr("node.set_progress")).clicked() {
                        blueprint.add_set_ui_progress_node();
                        set_last_ui_node_target(blueprint, owner_ui_id);
                        ui.close_menu();
                    }
                    if ui.button(tr("node.set_visible")).clicked() {
                        blueprint.add_set_ui_visible_node();
                        set_last_ui_node_target(blueprint, owner_ui_id);
                        ui.close_menu();
                    }
                }
                BlueprintOwnerKind::UiImagePanel => {
                    if ui.button(tr("node.set_visible")).clicked() {
                        blueprint.add_set_ui_visible_node();
                        set_last_ui_node_target(blueprint, owner_ui_id);
                        ui.close_menu();
                    }
                }
            }

            // 设置变量对场景物体和音频物体有意义，UI蓝图不显示该模块。
            if !owner_kind.is_ui() && ui.button(tr("node.set_variable")).clicked() {
                blueprint.add_set_variable_node();
                ui.close_menu();
            }
            if ui.button(tr("node.switch_scene")).clicked() {
                blueprint.add_kind(BlueprintNodeKind::SwitchScene {
                    scene_name: "场景1".to_owned(),
                });
                ui.close_menu();
            }
            if !owner_kind.is_ui() && ui.button(tr("node.spawn")).clicked() {
                blueprint.add_kind(BlueprintNodeKind::SpawnObject {
                    template_object_id: 1,
                    x: 100.0,
                    y: 100.0,
                });
                ui.close_menu();
            }
            if !owner_kind.is_ui() && ui.button(tr("node.destroy")).clicked() {
                blueprint.add_kind(BlueprintNodeKind::DestroyObject { object_id: 0 });
                ui.close_menu();
            }
            if ui.button(tr("node.set_global")).clicked() {
                blueprint.add_kind(BlueprintNodeKind::SetGlobalVariable {
                    name: "score".to_owned(),
                    value: 0.0,
                });
                ui.close_menu();
            }
        });
        if !owner_kind.is_ui() {
            ui.menu_button(tr("blueprint.variable_nodes"), |ui| {
                if ui.button(tr("node.number_variable")).clicked() {
                    blueprint.add_number_variable_node();
                    ui.close_menu();
                }
                if ui.button(tr("node.global_variable")).clicked() {
                    blueprint.add_kind(BlueprintNodeKind::GlobalNumberVariable {
                        name: "score".to_owned(),
                        initial_value: 0.0,
                    });
                    ui.close_menu();
                }
            });
        }
        if ui.button(tr("blueprint.clear_connections")).clicked() {
            blueprint.connections.clear();
        }
        if ui.button(tr("blueprint.copy")).clicked() && !selected_node_ids.is_empty() {
            let mut clipboard = Blueprint::new();
            clipboard.nodes = blueprint
                .nodes
                .iter()
                .filter(|node| selected_node_ids.contains(&node.id))
                .cloned()
                .collect();
            clipboard.connections = blueprint
                .connections
                .iter()
                .filter(|connection| {
                    selected_node_ids.contains(&connection.from_node_id)
                        && selected_node_ids.contains(&connection.to_node_id)
                })
                .cloned()
                .collect();
            ui.ctx().data_mut(|data| {
                data.insert_temp(Id::new("slide2d_blueprint_clipboard"), clipboard)
            });
        }
        if ui.button(tr("blueprint.paste")).clicked() {
            if let Some(clipboard) = ui
                .ctx()
                .data(|data| data.get_temp::<Blueprint>(Id::new("slide2d_blueprint_clipboard")))
            {
                *selected_node_ids = blueprint.paste_blueprint(&clipboard, 32.0);
            }
        }
        if ui.button(tr("blueprint.delete_many")).clicked() {
            blueprint.remove_nodes(selected_node_ids);
            selected_node_ids.clear();
        }
        if ui.button(tr("blueprint.group")).clicked() && !selected_node_ids.is_empty() {
            blueprint.add_kind(BlueprintNodeKind::NodeGroup {
                title: "节点分组".to_owned(),
                collapsed: false,
                node_ids: selected_node_ids.clone(),
            });
        }
        ui.menu_button(tr("blueprint.plugin_nodes"), |ui| {
            if plugin_nodes.is_empty() {
                ui.label(tr("blueprint.no_plugins"));
            }
            for (plugin_id, definition) in plugin_nodes {
                if ui.button(&definition.display_name).clicked() {
                    blueprint.add_plugin_node(plugin_id.clone(), definition.clone());
                    ui.close_menu();
                }
            }
        });
        if ui.button(tr("blueprint.reset_view")).clicked() {
            reset_view = true;
        }
        ui.label(tr_args(
            "blueprint.zoom",
            &[("value", format!("{:.0}", view_zoom * 100.0))],
        ));
        ui.label(tr("blueprint.connection_help"));
    });
    reset_view
}

/// 将刚创建的UI节点目标自动设置为当前UI持有者ID。
fn set_last_ui_node_target(blueprint: &mut Blueprint, owner_ui_id: Option<u64>) {
    let ui_id = match owner_ui_id {
        Some(id) => id,
        None => return,
    };
    let node = match blueprint.nodes.last_mut() {
        Some(node) => node,
        None => return,
    };
    match &mut node.kind {
        BlueprintNodeKind::ButtonClicked { ui_id: target }
        | BlueprintNodeKind::SetUiText { ui_id: target, .. }
        | BlueprintNodeKind::SetUiProgress { ui_id: target, .. }
        | BlueprintNodeKind::SetUiVisible { ui_id: target, .. } => *target = ui_id,
        _ => {}
    }
}

/// 将蓝图工具栏覆盖在网格上方，使工具按钮不会缩小蓝图地图。
fn draw_floating_toolbar(
    context: &egui::Context,
    blueprint: &mut Blueprint,
    view_zoom: f32,
    canvas_rect: Rect,
    owner_kind: BlueprintOwnerKind,
    owner_ui_id: Option<u64>,
    selected_node_ids: &mut Vec<u64>,
    plugin_nodes: &[(String, crate::plugins::PluginNodeDefinition)],
) -> bool {
    let mut reset_view = false;
    egui::Area::new(Id::new("blueprint_floating_toolbar"))
        .order(egui::Order::Foreground)
        .fixed_pos(canvas_rect.left_top() + Vec2::new(12.0, 12.0))
        .show(context, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                reset_view = draw_toolbar(
                    ui,
                    blueprint,
                    view_zoom,
                    owner_kind,
                    owner_ui_id,
                    selected_node_ids,
                    plugin_nodes,
                );
            });
        });
    reset_view
}

/// 绘制节点画布、节点连线，并处理节点拖动和端口点击。
fn draw_node_canvas(
    ui: &mut egui::Ui,
    blueprint: &mut Blueprint,
    pending_output: &mut Option<u64>,
    pending_output_port: &mut u8,
    selected_node_id: &mut Option<u64>,
    selected_node_ids: &mut Vec<u64>,
    view_offset: &mut Vec2,
    view_zoom: &mut f32,
) -> Rect {
    let available_size = ui.available_size();
    let (canvas_rect, canvas_response) = ui.allocate_exact_size(available_size, Sense::click());
    let painter = ui.painter_at(canvas_rect);
    painter.rect_filled(canvas_rect, 3.0, Color32::from_rgb(35, 38, 44));
    painter.text(
        canvas_rect.right_bottom() - Vec2::new(12.0, 10.0),
        egui::Align2::RIGHT_BOTTOM,
        "Made by Slide2D",
        egui::FontId::proportional(13.0),
        Color32::from_white_alpha(70),
    );
    handle_blueprint_navigation(ui, canvas_rect, view_offset, view_zoom);
    draw_blueprint_grid(&painter, canvas_rect, *view_offset, *view_zoom);

    let pointer_position = ui.input(|input| input.pointer.hover_pos());
    let hidden_node_ids: std::collections::HashSet<u64> = blueprint
        .nodes
        .iter()
        .filter_map(|node| match &node.kind {
            BlueprintNodeKind::NodeGroup {
                collapsed: true,
                node_ids,
                ..
            } => Some(node_ids.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    let alt_primary_clicked = ui.input(|input| {
        input.modifiers.alt && input.pointer.button_clicked(egui::PointerButton::Primary)
    });
    let mut connection_to_remove = None;
    for (connection_index, connection) in blueprint.connections.iter().enumerate() {
        if hidden_node_ids.contains(&connection.from_node_id)
            || hidden_node_ids.contains(&connection.to_node_id)
        {
            continue;
        }
        let from_position = node_output_position(
            blueprint,
            connection.from_node_id,
            connection.from_port,
            canvas_rect,
            *view_offset,
            *view_zoom,
        );
        let to_position = node_input_position(
            blueprint,
            connection.to_node_id,
            canvas_rect,
            *view_offset,
            *view_zoom,
        );
        if let (Some(from), Some(to)) = (from_position, to_position) {
            draw_connection_curve(&painter, from, to, Color32::from_rgb(240, 190, 65));
            if alt_primary_clicked {
                if let Some(pointer_position) = pointer_position {
                    if point_is_near_connection(pointer_position, from, to, 9.0) {
                        connection_to_remove = Some(connection_index);
                    }
                }
            }
        }
    }
    if let Some(connection_index) = connection_to_remove {
        blueprint.remove_connection(connection_index);
        *pending_output = None;
    }

    let snapped_node_id = find_snap_target(
        blueprint,
        *pending_output,
        pointer_position,
        canvas_rect,
        *view_offset,
        *view_zoom,
    );

    let mut node_to_remove = None;
    let mut node_to_disconnect = None;
    for node in &mut blueprint.nodes {
        if hidden_node_ids.contains(&node.id) {
            continue;
        }
        let node_position = blueprint_to_screen(
            Pos2::new(node.x, node.y),
            canvas_rect,
            *view_offset,
            *view_zoom,
        );
        let node_rect = Rect::from_min_size(
            node_position,
            Vec2::new(NODE_WIDTH * *view_zoom, NODE_HEIGHT * *view_zoom),
        );
        let node_response = ui.interact(
            node_rect,
            Id::new(("blueprint_node", node.id)),
            Sense::click_and_drag(),
        );
        if node_response.dragged_by(egui::PointerButton::Primary) {
            // egui 0.28的drag_delta是当前帧鼠标增量，因此应逐帧累加。
            // 除以视图缩放后，节点在任意缩放比例下都会紧跟鼠标。
            let delta = node_response.drag_delta();
            node.x += delta.x / *view_zoom;
            node.y += delta.y / *view_zoom;
        }
        if node_response.clicked() {
            *selected_node_id = Some(node.id);
            let control_down = ui.input(|input| input.modifiers.ctrl);
            if control_down {
                if selected_node_ids.contains(&node.id) {
                    selected_node_ids.retain(|id| *id != node.id);
                } else {
                    selected_node_ids.push(node.id);
                }
            } else {
                selected_node_ids.clear();
                selected_node_ids.push(node.id);
            }
        }
        node_response.context_menu(|ui| {
            ui.label(tr_args(
                "blueprint.node_id",
                &[("value", node.id.to_string())],
            ));
            ui.separator();
            if ui.button(tr("blueprint.disconnect")).clicked() {
                node_to_disconnect = Some(node.id);
                ui.close_menu();
            }
            if ui
                .button(
                    egui::RichText::new(tr("blueprint.delete_node"))
                        .color(Color32::from_rgb(235, 90, 90)),
                )
                .clicked()
            {
                node_to_remove = Some(node.id);
                ui.close_menu();
            }
        });

        let node_color = match node.kind.category() {
            BlueprintNodeCategory::Event => Color32::from_rgb(115, 65, 145),
            BlueprintNodeCategory::Logic => Color32::from_rgb(185, 115, 35),
            BlueprintNodeCategory::Action => Color32::from_rgb(45, 105, 145),
            BlueprintNodeCategory::Variable => Color32::from_rgb(45, 135, 85),
        };
        painter.rect_filled(node_rect, 6.0, Color32::from_rgb(58, 62, 70));
        painter.rect_filled(
            Rect::from_min_max(
                node_rect.min,
                Pos2::new(node_rect.right(), node_rect.top() + 28.0 * *view_zoom),
            ),
            6.0,
            node_color,
        );
        painter.rect_stroke(
            node_rect,
            6.0,
            Stroke::new(1.0_f32, Color32::from_gray(145)),
        );
        painter.text(
            node_rect.left_top() + Vec2::new(10.0, 6.0) * *view_zoom,
            egui::Align2::LEFT_TOP,
            node_title(&node.kind),
            egui::FontId::proportional(16.0 * *view_zoom),
            Color32::WHITE,
        );
        painter.text(
            node_rect.left_top() + Vec2::new(10.0, 42.0) * *view_zoom,
            egui::Align2::LEFT_TOP,
            node_description(&node.kind),
            egui::FontId::proportional(14.0 * *view_zoom),
            Color32::LIGHT_GRAY,
        );

        let has_input = node.kind.has_execution_input();
        let has_output = node.kind.has_execution_output();
        if has_input {
            let input_position = Pos2::new(node_rect.left(), node_rect.center().y);
            let input_rect = Rect::from_center_size(input_position, Vec2::splat(PORT_RADIUS * 3.0));
            ui.interact(
                input_rect,
                Id::new(("blueprint_input", node.id)),
                Sense::hover(),
            );
            let input_is_snapped = snapped_node_id == Some(node.id);
            let input_color = if input_is_snapped {
                Color32::WHITE
            } else {
                Color32::from_rgb(240, 190, 65)
            };
            let input_radius = if input_is_snapped {
                PORT_RADIUS + 3.0
            } else {
                PORT_RADIUS
            };
            painter.circle_filled(input_position, input_radius, input_color);
        }
        if has_output {
            let output_ports: Vec<u8> = if node.kind.uses_branch_outputs() {
                vec![1, 2]
            } else {
                vec![0]
            };
            for port in output_ports {
                let output_y = if port == 1 {
                    node_rect.top() + NODE_HEIGHT * 0.38 * *view_zoom
                } else if port == 2 {
                    node_rect.top() + NODE_HEIGHT * 0.72 * *view_zoom
                } else {
                    node_rect.center().y
                };
                let output_position = Pos2::new(node_rect.right(), output_y);
                let output_rect =
                    Rect::from_center_size(output_position, Vec2::splat(PORT_RADIUS * 3.0));
                let output_response = ui.interact(
                    output_rect,
                    Id::new(("blueprint_output", node.id, port)),
                    Sense::drag(),
                );
                painter.circle_filled(
                    output_position,
                    PORT_RADIUS,
                    Color32::from_rgb(240, 190, 65),
                );
                if output_response.drag_started_by(egui::PointerButton::Primary) {
                    *pending_output = Some(node.id);
                    *pending_output_port = port;
                }
            }
        }
    }

    if let Some(node_id) = node_to_disconnect {
        blueprint.disconnect_node(node_id);
        *pending_output = None;
    }
    if let Some(node_id) = node_to_remove {
        blueprint.remove_node(node_id);
        if *selected_node_id == Some(node_id) {
            *selected_node_id = None;
        }
        *pending_output = None;
    }

    // 拉线期间，曲线末端实时跟随鼠标；进入吸附范围后改用输入端中心。
    if let Some(from_node_id) = *pending_output {
        if let Some(from_position) = node_output_position(
            blueprint,
            from_node_id,
            *pending_output_port,
            canvas_rect,
            *view_offset,
            *view_zoom,
        ) {
            let preview_end = match snapped_node_id {
                Some(node_id) => {
                    node_input_position(blueprint, node_id, canvas_rect, *view_offset, *view_zoom)
                }
                None => pointer_position,
            };
            if let Some(preview_end) = preview_end {
                draw_connection_curve(
                    &painter,
                    from_position,
                    preview_end,
                    Color32::from_rgb(255, 220, 95),
                );
            }
        }
    }

    // 松开左键时结束拖线。在输入端吸附范围内则连接，否则取消。
    let primary_released =
        ui.input(|input| input.pointer.button_released(egui::PointerButton::Primary));
    if primary_released {
        if let (Some(from_node_id), Some(to_node_id)) = (*pending_output, snapped_node_id) {
            blueprint.connect_from_port(from_node_id, *pending_output_port, to_node_id);
        }
        *pending_output = None;
    } else if canvas_response.clicked() {
        *pending_output = None;
    }
    canvas_rect
}

/// 查找鼠标附近可以连接的执行节点输入端。
fn find_snap_target(
    blueprint: &Blueprint,
    from_node_id: Option<u64>,
    pointer_position: Option<Pos2>,
    canvas_rect: Rect,
    view_offset: Vec2,
    view_zoom: f32,
) -> Option<u64> {
    let from_node_id = from_node_id?;
    let pointer_position = pointer_position?;
    let from_node = blueprint
        .nodes
        .iter()
        .find(|node| node.id == from_node_id)?;
    if !from_node.kind.has_execution_output() {
        return None;
    }

    let mut nearest_node_id = None;
    let mut nearest_distance = PORT_SNAP_DISTANCE;
    for node in &blueprint.nodes {
        if !node.kind.has_execution_input() {
            continue;
        }
        let input_position = blueprint_to_screen(
            Pos2::new(node.x, node.y + NODE_HEIGHT * 0.5),
            canvas_rect,
            view_offset,
            view_zoom,
        );
        let distance = input_position.distance(pointer_position);
        if distance <= nearest_distance {
            nearest_distance = distance;
            nearest_node_id = Some(node.id);
        }
    }
    nearest_node_id
}

/// 绘制类似UE节点编辑器的平滑三次贝塞尔连线。
fn draw_connection_curve(painter: &egui::Painter, start: Pos2, end: Pos2, color: Color32) {
    let control_distance = ((end.x - start.x).abs() * 0.5).max(60.0);
    let control_start = Pos2::new(start.x + control_distance, start.y);
    let control_end = Pos2::new(end.x - control_distance, end.y);
    let curve = CubicBezierShape::from_points_stroke(
        [start, control_start, control_end, end],
        false,
        Color32::TRANSPARENT,
        Stroke::new(3.0_f32, color),
    );
    painter.add(curve);
}

/// 判断鼠标是否靠近一条贝塞尔连线，用于Alt加左键删除单条连线。
fn point_is_near_connection(pointer: Pos2, start: Pos2, end: Pos2, maximum_distance: f32) -> bool {
    let control_distance = ((end.x - start.x).abs() * 0.5).max(60.0);
    let control_start = Pos2::new(start.x + control_distance, start.y);
    let control_end = Pos2::new(end.x - control_distance, end.y);
    let mut previous_point = start;

    // 将贝塞尔曲线分成24条短线段，命中效果足够平滑且代码容易理解。
    for step in 1..=24 {
        let time = step as f32 / 24.0;
        let current_point = cubic_bezier_point(start, control_start, control_end, end, time);
        if distance_to_line_segment(pointer, previous_point, current_point) <= maximum_distance {
            return true;
        }
        previous_point = current_point;
    }
    false
}

/// 计算三次贝塞尔曲线在指定时间位置的坐标。
fn cubic_bezier_point(start: Pos2, control1: Pos2, control2: Pos2, end: Pos2, time: f32) -> Pos2 {
    let reverse_time = 1.0 - time;
    let start_weight = reverse_time * reverse_time * reverse_time;
    let control1_weight = 3.0 * reverse_time * reverse_time * time;
    let control2_weight = 3.0 * reverse_time * time * time;
    let end_weight = time * time * time;
    Pos2::new(
        start.x * start_weight
            + control1.x * control1_weight
            + control2.x * control2_weight
            + end.x * end_weight,
        start.y * start_weight
            + control1.y * control1_weight
            + control2.y * control2_weight
            + end.y * end_weight,
    )
}

/// 计算一个点到有限线段的最短距离。
fn distance_to_line_segment(point: Pos2, line_start: Pos2, line_end: Pos2) -> f32 {
    let line = line_end - line_start;
    let length_squared = line.length_sq();
    if length_squared <= f32::EPSILON {
        return point.distance(line_start);
    }
    let projection = ((point - line_start).dot(line) / length_squared).clamp(0.0, 1.0);
    let nearest_point = line_start + line * projection;
    point.distance(nearest_point)
}

/// 绘制蓝图背景网格。
fn draw_blueprint_grid(painter: &egui::Painter, rect: Rect, view_offset: Vec2, view_zoom: f32) {
    let spacing = 24.0 * view_zoom;
    let stroke = Stroke::new(1.0_f32, Color32::from_rgb(48, 52, 60));
    let mut x = rect.left() + view_offset.x.rem_euclid(spacing);
    while x < rect.right() {
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            stroke,
        );
        x += spacing;
    }
    let mut y = rect.top() + view_offset.y.rem_euclid(spacing);
    while y < rect.bottom() {
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            stroke,
        );
        y += spacing;
    }
}

/// 处理蓝图无限画布的右键平移和滚轮缩放。
fn handle_blueprint_navigation(
    ui: &egui::Ui,
    canvas_rect: Rect,
    view_offset: &mut Vec2,
    view_zoom: &mut f32,
) {
    let pointer_position = match ui.input(|input| input.pointer.hover_pos()) {
        Some(position) if canvas_rect.contains(position) => position,
        _ => return,
    };

    if ui.input(|input| input.pointer.secondary_down()) {
        *view_offset += ui.input(|input| input.pointer.delta());
        ui.output_mut(|output| output.cursor_icon = egui::CursorIcon::Grabbing);
    }

    let scroll_delta = ui.input(|input| input.smooth_scroll_delta.y);
    if scroll_delta.abs() < 0.01 {
        return;
    }
    let world_before_zoom =
        screen_to_blueprint(pointer_position, canvas_rect, *view_offset, *view_zoom);
    let multiplier = (scroll_delta * 0.0015).exp();
    *view_zoom = (*view_zoom * multiplier).clamp(MIN_BLUEPRINT_ZOOM, MAX_BLUEPRINT_ZOOM);
    view_offset.x = pointer_position.x - canvas_rect.left() - world_before_zoom.x * *view_zoom;
    view_offset.y = pointer_position.y - canvas_rect.top() - world_before_zoom.y * *view_zoom;
}

/// 将蓝图世界坐标转换为当前画布中的屏幕坐标。
fn blueprint_to_screen(
    world_position: Pos2,
    canvas_rect: Rect,
    view_offset: Vec2,
    view_zoom: f32,
) -> Pos2 {
    Pos2::new(
        canvas_rect.left() + view_offset.x + world_position.x * view_zoom,
        canvas_rect.top() + view_offset.y + world_position.y * view_zoom,
    )
}

/// 将画布屏幕坐标转换回蓝图世界坐标。
fn screen_to_blueprint(
    screen_position: Pos2,
    canvas_rect: Rect,
    view_offset: Vec2,
    view_zoom: f32,
) -> Pos2 {
    Pos2::new(
        (screen_position.x - canvas_rect.left() - view_offset.x) / view_zoom,
        (screen_position.y - canvas_rect.top() - view_offset.y) / view_zoom,
    )
}

/// 绘制当前选中蓝图节点的参数编辑区域。
fn draw_node_properties(
    ui: &mut egui::Ui,
    blueprint: &mut Blueprint,
    selected_node_id: Option<u64>,
) {
    ui.vertical(|ui| {
        ui.set_min_width(210.0);
        ui.heading(tr("blueprint.node_properties"));
        let selected_id = match selected_node_id {
            Some(id) => id,
            None => {
                ui.label(tr("blueprint.properties_hint"));
                return;
            }
        };
        let node = match blueprint
            .nodes
            .iter_mut()
            .find(|node| node.id == selected_id)
        {
            Some(node) => node,
            None => return,
        };
        match &mut node.kind {
            BlueprintNodeKind::PluginNode {
                plugin_id,
                node_type,
                description,
                behavior,
                variable_name,
                value,
                ..
            } => {
                ui.label(format!("Slide2D Plugin：{plugin_id}"));
                ui.label(tr_args(
                    "blueprint.plugin_type",
                    &[("value", node_type.clone())],
                ));
                ui.label(description.as_str());
                ui.label(tr_args(
                    "blueprint.plugin_behavior",
                    &[("value", plugin_behavior_name(behavior).to_owned())],
                ));
                ui.label(tr("blueprint.variable_name"));
                ui.text_edit_singleline(variable_name);
                if !matches!(behavior, crate::plugins::PluginBehavior::PickupCheck) {
                    ui.add(egui::DragValue::new(value).prefix("数值："));
                }
                ui.small(tr("blueprint.plugin_no_code"));
            }
            BlueprintNodeKind::FrameUpdated => {
                ui.label(tr("blueprint.category_event"));
                ui.label(tr("blueprint.type_frame"));
                ui.label(tr("blueprint.frame_help"));
            }
            BlueprintNodeKind::CollisionTriggered => {
                ui.label(tr("blueprint.category_event"));
                ui.label(tr("blueprint.type_collision"));
                ui.label(tr("blueprint.collision_help"));
            }
            BlueprintNodeKind::KeyPressed { key } => {
                ui.label(tr("blueprint.category_event"));
                ui.label(tr("blueprint.type_key"));
                ui.label(tr("blueprint.key"));
                *key = canonical_key_name(key);
                egui::ComboBox::from_id_source(("blueprint_key_selector", node.id))
                    .selected_text(key_display_name(key))
                    .width(180.0)
                    .show_ui(ui, |ui| {
                        for (key_name, display_name) in AVAILABLE_KEYS {
                            ui.selectable_value(key, (*key_name).to_owned(), *display_name);
                        }
                    });
                ui.label(tr("blueprint.key_help"));
            }
            BlueprintNodeKind::ButtonClicked { ui_id } => {
                ui.label(tr("blueprint.category_event"));
                ui.label(tr("blueprint.type_button"));
                ui.add(egui::DragValue::new(ui_id).prefix("按钮UI ID："));
            }
            BlueprintNodeKind::Timer {
                delay_seconds,
                repeat,
            } => {
                ui.label(tr("blueprint.category_event"));
                ui.add(
                    egui::DragValue::new(delay_seconds)
                        .speed(0.1)
                        .range(0.001..=86400.0)
                        .prefix("延迟秒数："),
                );
                ui.checkbox(repeat, tr("blueprint.repeat"));
            }
            BlueprintNodeKind::ObjectClicked => {
                ui.label(tr("blueprint.object_click_help"));
            }
            BlueprintNodeKind::SceneLoaded => {
                ui.label(tr("blueprint.scene_load_help"));
            }
            BlueprintNodeKind::IfCondition {
                variable_name,
                comparison,
                compare_value,
                use_global,
            } => {
                ui.text_edit_singleline(variable_name);
                draw_comparison_selector(ui, node.id, comparison);
                ui.add(egui::DragValue::new(compare_value).prefix("比较值："));
                ui.checkbox(use_global, tr("blueprint.use_global"));
                ui.label(tr("blueprint.branch_help"));
            }
            BlueprintNodeKind::CompareVariables {
                left_name,
                right_name,
                comparison,
                use_global,
            } => {
                ui.text_edit_singleline(left_name);
                draw_comparison_selector(ui, node.id, comparison);
                ui.text_edit_singleline(right_name);
                ui.checkbox(use_global, tr("blueprint.use_global"));
            }
            BlueprintNodeKind::ModifyPosition { delta_x, delta_y } => {
                ui.label(tr("blueprint.category_action"));
                ui.label(tr("blueprint.type_move"));
                ui.label(tr("blueprint.delta_x"));
                ui.add(egui::DragValue::new(delta_x).speed(1.0));
                ui.label(tr("blueprint.delta_y"));
                ui.add(egui::DragValue::new(delta_y).speed(1.0));
            }
            BlueprintNodeKind::SetVariable { name, value } => {
                ui.label(tr("blueprint.category_action"));
                ui.label(tr("blueprint.type_set_variable"));
                ui.label(tr("blueprint.variable_name"));
                ui.text_edit_singleline(name);
                ui.label(tr("blueprint.set_value"));
                ui.add(egui::DragValue::new(value).speed(1.0));
            }
            BlueprintNodeKind::PlaySound { path, volume } => {
                ui.label(tr("blueprint.category_action"));
                ui.label(tr("blueprint.type_play_sound"));
                ui.label(tr("blueprint.sound_path"));
                ui.text_edit_singleline(path);
                ui.label(tr("blueprint.volume"));
                ui.add(egui::Slider::new(volume, 0.0..=1.0));
                ui.label(tr("blueprint.audio_help"));
            }
            BlueprintNodeKind::StopSound => {
                ui.label(tr("blueprint.category_action"));
                ui.label(tr("blueprint.type_stop_sound"));
                ui.label(tr("blueprint.stop_sound_help"));
            }
            BlueprintNodeKind::SwitchAnimation { animation_path } => {
                ui.label(tr("blueprint.category_action"));
                ui.label(tr("blueprint.type_switch_animation"));
                ui.label(tr("blueprint.animation_path"));
                ui.text_edit_singleline(animation_path);
            }
            BlueprintNodeKind::PauseAnimation => {
                ui.label(tr("blueprint.category_action"));
                ui.label(tr("blueprint.type_pause_animation"));
            }
            BlueprintNodeKind::PlayAnimation => {
                ui.label(tr("blueprint.category_action"));
                ui.label(tr("blueprint.type_play_animation"));
            }
            BlueprintNodeKind::DetectTile { variable_name } => {
                ui.label(tr("blueprint.category_action"));
                ui.label(tr("blueprint.type_detect_tile"));
                ui.label(tr("blueprint.tile_variable"));
                ui.text_edit_singleline(variable_name);
                ui.label(tr("blueprint.no_tile"));
            }
            BlueprintNodeKind::SetUiText { ui_id, content } => {
                ui.label(tr("blueprint.category_action"));
                ui.add(egui::DragValue::new(ui_id).prefix(tr("blueprint.text_ui_id")));
                ui.label(tr("blueprint.new_text"));
                ui.text_edit_multiline(content);
            }
            BlueprintNodeKind::SetUiProgress { ui_id, value } => {
                ui.label(tr("blueprint.category_action"));
                ui.add(egui::DragValue::new(ui_id).prefix("进度条UI ID："));
                ui.add(egui::DragValue::new(value).prefix(tr("blueprint.new_value")));
            }
            BlueprintNodeKind::SetUiVisible { ui_id, visible } => {
                ui.label(tr("blueprint.category_action"));
                ui.add(egui::DragValue::new(ui_id).prefix(tr("blueprint.ui_id")));
                ui.checkbox(visible, tr("blueprint.visible"));
            }
            BlueprintNodeKind::SwitchScene { scene_name } => {
                ui.label(tr("blueprint.target_scene"));
                ui.text_edit_singleline(scene_name);
            }
            BlueprintNodeKind::SpawnObject {
                template_object_id,
                x,
                y,
            } => {
                ui.add(
                    egui::DragValue::new(template_object_id).prefix(tr("blueprint.template_id")),
                );
                ui.add(egui::DragValue::new(x).prefix("X："));
                ui.add(egui::DragValue::new(y).prefix("Y："));
            }
            BlueprintNodeKind::DestroyObject { object_id } => {
                ui.add(egui::DragValue::new(object_id).prefix(tr("blueprint.object_id")));
                ui.label(tr("blueprint.current_object_zero"));
            }
            BlueprintNodeKind::SetGlobalVariable { name, value } => {
                ui.text_edit_singleline(name);
                ui.add(egui::DragValue::new(value).prefix("数值："));
            }
            BlueprintNodeKind::NumberVariable {
                name,
                initial_value,
            } => {
                ui.label(tr("blueprint.category_variable"));
                ui.label(tr("blueprint.type_number"));
                ui.label(tr("blueprint.variable_name"));
                ui.text_edit_singleline(name);
                ui.label(tr("blueprint.initial_value"));
                ui.add(egui::DragValue::new(initial_value).speed(1.0));
                ui.label(tr("blueprint.object_variable_help"));
            }
            BlueprintNodeKind::GlobalNumberVariable {
                name,
                initial_value,
            } => {
                ui.label(tr("blueprint.category_global"));
                ui.text_edit_singleline(name);
                ui.add(egui::DragValue::new(initial_value).prefix(tr("blueprint.initial_value")));
            }
            BlueprintNodeKind::NodeGroup {
                title,
                collapsed,
                node_ids,
            } => {
                ui.text_edit_singleline(title);
                ui.checkbox(collapsed, tr("blueprint.collapse_group"));
                ui.label(tr_args(
                    "blueprint.group_count",
                    &[("value", node_ids.len().to_string())],
                ));
            }
        }
    });
}

/// 绘制比较方式下拉框。
fn draw_comparison_selector(ui: &mut egui::Ui, node_id: u64, comparison: &mut ComparisonOperator) {
    egui::ComboBox::from_id_source(("comparison", node_id))
        .selected_text(comparison.display_name())
        .show_ui(ui, |ui| {
            for value in [
                ComparisonOperator::Equal,
                ComparisonOperator::NotEqual,
                ComparisonOperator::Greater,
                ComparisonOperator::GreaterOrEqual,
                ComparisonOperator::Less,
                ComparisonOperator::LessOrEqual,
            ] {
                ui.selectable_value(comparison, value, value.display_name());
            }
        });
}

/// 将节点属性显示为覆盖在蓝图地图上的浮动窗口，不占用画布布局空间。
fn draw_floating_node_properties(
    context: &egui::Context,
    blueprint: &mut Blueprint,
    selected_node_id: Option<u64>,
    canvas_rect: Rect,
) {
    if selected_node_id.is_none() {
        return;
    }

    egui::Window::new(format!(
        "Slide2D Blueprint - {}",
        tr("blueprint.node_properties")
    ))
    .id(Id::new("blueprint_node_properties"))
    .default_pos(canvas_rect.right_top() + Vec2::new(-250.0, 70.0))
    .default_width(230.0)
    .collapsible(true)
    .resizable(false)
    .show(context, |ui| {
        draw_node_properties(ui, blueprint, selected_node_id);
    });
}

/// 返回节点标题。
fn node_title(kind: &BlueprintNodeKind) -> String {
    let (category, name) = match kind {
        BlueprintNodeKind::PluginNode { .. } => {
            ("blueprint.plugin_nodes", "node.plugin_declarative")
        }
        BlueprintNodeKind::FrameUpdated => ("category.event", "node.frame_updated"),
        BlueprintNodeKind::KeyPressed { .. } => ("category.event", "node.key_pressed"),
        BlueprintNodeKind::CollisionTriggered => ("category.event", "node.collision"),
        BlueprintNodeKind::ButtonClicked { .. } => ("category.event", "node.button_clicked"),
        BlueprintNodeKind::Timer { .. } => ("category.event", "node.timer"),
        BlueprintNodeKind::ObjectClicked => ("category.event", "node.object_clicked"),
        BlueprintNodeKind::SceneLoaded => ("category.event", "node.scene_loaded"),
        BlueprintNodeKind::IfCondition { .. } => ("category.logic", "node.if"),
        BlueprintNodeKind::CompareVariables { .. } => ("category.logic", "node.compare_variables"),
        BlueprintNodeKind::ModifyPosition { .. } => ("category.action", "node.move"),
        BlueprintNodeKind::SetVariable { .. } => ("category.action", "node.set_variable"),
        BlueprintNodeKind::PlaySound { .. } => ("category.action", "node.play_sound"),
        BlueprintNodeKind::StopSound => ("category.action", "node.stop_sound"),
        BlueprintNodeKind::SwitchAnimation { .. } => ("category.action", "node.switch_animation"),
        BlueprintNodeKind::PauseAnimation => ("category.action", "node.pause_animation"),
        BlueprintNodeKind::PlayAnimation => ("category.action", "node.play_animation"),
        BlueprintNodeKind::DetectTile { .. } => ("category.action", "node.detect_tile"),
        BlueprintNodeKind::SetUiText { .. } => ("category.action", "node.set_text"),
        BlueprintNodeKind::SetUiProgress { .. } => ("category.action", "node.set_progress"),
        BlueprintNodeKind::SetUiVisible { .. } => ("category.action", "node.set_visible"),
        BlueprintNodeKind::SwitchScene { .. } => ("category.action", "node.switch_scene"),
        BlueprintNodeKind::SpawnObject { .. } => ("category.action", "node.spawn"),
        BlueprintNodeKind::DestroyObject { .. } => ("category.action", "node.destroy"),
        BlueprintNodeKind::SetGlobalVariable { .. } => ("category.action", "node.set_global"),
        BlueprintNodeKind::NumberVariable { .. } => ("category.variable", "node.number_variable"),
        BlueprintNodeKind::GlobalNumberVariable { .. } => {
            ("category.variable", "node.global_variable")
        }
        BlueprintNodeKind::NodeGroup { .. } => ("category.group", "blueprint.group"),
    };
    format!("{}: {}", tr(category), tr(name))
}

/// 返回显示在节点内部的简短参数说明。
fn node_description(kind: &BlueprintNodeKind) -> String {
    let description = match kind {
        BlueprintNodeKind::PluginNode {
            display_name,
            description,
            ..
        } => {
            format!("{display_name}：{description}")
        }
        BlueprintNodeKind::FrameUpdated => "每个逻辑帧触发".to_owned(),
        BlueprintNodeKind::KeyPressed { key } => format!("按键：{}", key_display_name(key)),
        BlueprintNodeKind::CollisionTriggered => "与其他物体发生碰撞时触发".to_owned(),
        BlueprintNodeKind::ButtonClicked { ui_id } => format!("按钮UI ID：{ui_id}"),
        BlueprintNodeKind::Timer {
            delay_seconds,
            repeat,
        } => format!(
            "{delay_seconds:.2}秒{}",
            if *repeat { "，循环" } else { "" }
        ),
        BlueprintNodeKind::ObjectClicked => "点击当前物体时触发".to_owned(),
        BlueprintNodeKind::SceneLoaded => "场景加载后触发一次".to_owned(),
        BlueprintNodeKind::IfCondition {
            variable_name,
            comparison,
            compare_value,
            ..
        } => format!(
            "{variable_name} {} {compare_value}",
            comparison.display_name()
        ),
        BlueprintNodeKind::CompareVariables {
            left_name,
            right_name,
            comparison,
            ..
        } => format!("{left_name} {} {right_name}", comparison.display_name()),
        BlueprintNodeKind::ModifyPosition { delta_x, delta_y } => {
            format!("X {delta_x:.0}/秒，Y {delta_y:.0}/秒")
        }
        BlueprintNodeKind::SetVariable { name, value } => {
            format!("{name} = {value:.1}")
        }
        BlueprintNodeKind::PlaySound { path, volume } => {
            let file_name = std::path::Path::new(path)
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("未选择音效");
            format!("{file_name}，音量 {volume:.2}")
        }
        BlueprintNodeKind::StopSound => "停止当前物体音效".to_owned(),
        BlueprintNodeKind::SwitchAnimation { animation_path } => {
            let name = std::path::Path::new(animation_path)
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("未选择动画");
            format!("切换到：{name}")
        }
        BlueprintNodeKind::PauseAnimation => "暂停当前动画".to_owned(),
        BlueprintNodeKind::PlayAnimation => "继续播放当前动画".to_owned(),
        BlueprintNodeKind::DetectTile { variable_name } => {
            format!("脚下瓦片 -> {variable_name}")
        }
        BlueprintNodeKind::SetUiText { ui_id, content } => {
            format!("UI {ui_id} = {content}")
        }
        BlueprintNodeKind::SetUiProgress { ui_id, value } => {
            format!("UI {ui_id} = {value:.1}")
        }
        BlueprintNodeKind::SetUiVisible { ui_id, visible } => {
            format!("UI {ui_id} -> {}", if *visible { "显示" } else { "隐藏" })
        }
        BlueprintNodeKind::SwitchScene { scene_name } => format!("切换到：{scene_name}"),
        BlueprintNodeKind::SpawnObject {
            template_object_id,
            x,
            y,
        } => format!("模板 {template_object_id} -> ({x:.0}, {y:.0})"),
        BlueprintNodeKind::DestroyObject { object_id } => format!("销毁ID：{object_id}"),
        BlueprintNodeKind::SetGlobalVariable { name, value } => format!("{name} = {value:.1}"),
        BlueprintNodeKind::NumberVariable {
            name,
            initial_value,
        } => format!("{name}，初始值 {initial_value:.1}"),
        BlueprintNodeKind::GlobalNumberVariable {
            name,
            initial_value,
        } => format!("全局 {name}，初始值 {initial_value:.1}"),
        BlueprintNodeKind::NodeGroup {
            title, node_ids, ..
        } => format!("{title}，{}个节点", node_ids.len()),
    };
    localize_message(&description)
}

/// 返回插件Runtime白名单行为的中文名称。
fn plugin_behavior_name(behavior: &crate::plugins::PluginBehavior) -> &'static str {
    match behavior {
        crate::plugins::PluginBehavior::SceneLoadedEvent => "场景加载事件",
        crate::plugins::PluginBehavior::ObjectClickedEvent => "物体点击事件",
        crate::plugins::PluginBehavior::PickupCheck => "拾取判定",
        crate::plugins::PluginBehavior::SetObjectVariable => "物体变量赋值",
        crate::plugins::PluginBehavior::SetGlobalVariable => "全局变量赋值",
        crate::plugins::PluginBehavior::MoveHorizontal => "物理横向移动",
        crate::plugins::PluginBehavior::NumberVariable => "数值变量",
    }
}

/// 查找节点输出端口在屏幕中的位置。
fn node_output_position(
    blueprint: &Blueprint,
    node_id: u64,
    port: u8,
    canvas_rect: Rect,
    view_offset: Vec2,
    view_zoom: f32,
) -> Option<Pos2> {
    let node = blueprint.nodes.iter().find(|node| node.id == node_id)?;
    Some(blueprint_to_screen(
        Pos2::new(
            node.x + NODE_WIDTH,
            node.y
                + if port == 1 {
                    NODE_HEIGHT * 0.38
                } else if port == 2 {
                    NODE_HEIGHT * 0.72
                } else {
                    NODE_HEIGHT * 0.5
                },
        ),
        canvas_rect,
        view_offset,
        view_zoom,
    ))
}

/// 查找节点输入端口在屏幕中的位置。
fn node_input_position(
    blueprint: &Blueprint,
    node_id: u64,
    canvas_rect: Rect,
    view_offset: Vec2,
    view_zoom: f32,
) -> Option<Pos2> {
    let node = blueprint.nodes.iter().find(|node| node.id == node_id)?;
    Some(blueprint_to_screen(
        Pos2::new(node.x, node.y + NODE_HEIGHT * 0.5),
        canvas_rect,
        view_offset,
        view_zoom,
    ))
}
