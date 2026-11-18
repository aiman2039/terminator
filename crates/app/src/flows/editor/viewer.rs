//! How the flow editor's graph shows in `egui-snarl`: each node is one
//! step with its fields in the node. The dot on the left edge is "after"
//! and the dot on the right edge is "then": a wire from the right dot of A
//! to the left dot of B means that B needs A. The "After" row of a node
//! shows the same wires as chips, so a wire can be made and removed
//! without a drag.
use super::model::{Node, free_id, input, output};
use crate::appearance;
use egui_snarl::{
    InPin, NodeId, OutPin, Snarl,
    ui::{AnyPins, PinInfo, PinPlacement, SnarlPin, SnarlStyle, SnarlViewer, WireLayer},
};

/// Width of the text fields in a node.
const FIELD_WIDTH: f32 = 240.0;
const ID_WIDTH: f32 = 170.0;
/// The dots are large so that a wire is easy to start and to drop.
const PIN_SIZE: f32 = 14.0;
const WIRE_WIDTH: f32 = 2.5;

/// The editor's colors, from the app theme.
#[derive(Clone, Copy)]
pub(super) struct Colors {
    pub(super) canvas: egui::Color32,
    pub(super) node: egui::Color32,
    pub(super) border: egui::Color32,
    /// Wires, dots and the outline of a step.
    pub(super) step: egui::Color32,
    /// The outline and the label of a gate.
    pub(super) gate: egui::Color32,
}

/// The viewer of one frame.
pub(super) struct Viewer {
    pub(super) colors: Colors,
    /// The canvas rectangle, on the first frame of a flow only: the view
    /// then moves and zooms out until all nodes show.
    pub(super) fit: Option<egui::Rect>,
}

/// The graph style: dots on the node edges, thick wires behind the nodes,
/// a quiet grid, no collapse button.
pub(super) fn style(colors: Colors) -> SnarlStyle {
    SnarlStyle {
        collapsible: Some(false),
        header_drag_space: Some(egui::Vec2::ZERO),
        pin_placement: Some(PinPlacement::Edge),
        pin_size: Some(PIN_SIZE),
        pin_fill: Some(colors.node),
        pin_stroke: Some(egui::Stroke::new(2.0, colors.step)),
        wire_width: Some(WIRE_WIDTH),
        wire_layer: Some(WireLayer::BehindNodes),
        bg_frame: Some(egui::Frame::new().fill(colors.canvas)),
        bg_pattern_stroke: Some(egui::Stroke::new(1.0, colors.border.gamma_multiply(0.7))),
        min_scale: Some(0.3),
        max_scale: Some(1.5),
        ..SnarlStyle::new()
    }
}

/// The size of a step node, for the first view of a flow.
const NODE_SIZE: egui::Vec2 = egui::vec2(310.0, 230.0);
const FIT_MARGIN: f32 = 40.0;

/// The view in which all nodes show in `canvas`, at most at full size.
pub(super) fn fit_view(
    snarl: &Snarl<Node>,
    canvas: egui::Rect,
) -> Option<egui::emath::TSTransform> {
    let mut all = egui::Rect::NOTHING;
    for (position, _) in snarl.nodes_pos() {
        all = all.union(egui::Rect::from_min_size(position, NODE_SIZE));
    }
    if !all.is_finite() || !canvas.is_finite() || canvas.width() < 1.0 || canvas.height() < 1.0 {
        return None;
    }
    let all = all.expand(FIT_MARGIN);
    let scaling = (canvas.width() / all.width())
        .min(canvas.height() / all.height())
        .clamp(0.2, 1.0);
    let (from, to) = (all.center(), canvas.center());
    let offset = egui::vec2(to.x - from.x * scaling, to.y - from.y * scaling);
    Some(egui::emath::TSTransform::new(offset, scaling))
}

/// One labeled field row of a node.
fn field(ui: &mut egui::Ui, (label, hint): (&str, &str), text: &mut String, rows: usize) {
    ui.weak(label);
    let edit = if rows > 1 {
        egui::TextEdit::multiline(text).desired_rows(rows)
    } else {
        egui::TextEdit::singleline(text)
    };
    ui.add(edit.desired_width(FIELD_WIDTH).hint_text(hint));
    ui.end_row();
}

/// Adds a step or a gate at `pos` and gives its node.
fn add_node(snarl: &mut Snarl<Node>, pos: egui::Pos2, gate: bool) -> NodeId {
    if gate {
        let id = free_id(snarl, "gate");
        snarl.insert_node(pos, Node::gate(&id, ""))
    } else {
        let id = free_id(snarl, "step");
        snarl.insert_node(pos, Node::step(&id, "claude", ""))
    }
}

/// The menu of one node: on a right-click and on its "more" button.
fn node_menu(ui: &mut egui::Ui, node: NodeId, snarl: &mut Snarl<Node>) {
    if ui.button("Duplicate").clicked() {
        if let Some(info) = snarl.get_node_info(node) {
            let mut copy = info.value.clone();
            let id = free_id(snarl, copy.id());
            *copy.id_mut() = id;
            let pos = egui::pos2(info.pos.x + 40.0, info.pos.y + 40.0);
            snarl.insert_node(pos, copy);
        }
        ui.close();
    }
    if ui.button("Remove wires").clicked() {
        snarl.drop_inputs(input(node));
        snarl.drop_outputs(output(node));
        ui.close();
    }
    ui.separator();
    if ui.button("Remove").clicked() {
        snarl.remove_node(node);
        ui.close();
    }
}

/// The "After" row: one chip per step that this node needs (a click
/// removes that wire), and "+" with the steps that it can need.
fn after_row(ui: &mut egui::Ui, node: NodeId, snarl: &mut Snarl<Node>) {
    let needs: Vec<_> = snarl
        .in_pin(input(node))
        .remotes
        .iter()
        .filter_map(|from| Some((*from, snarl.get_node(from.node)?.id().to_owned())))
        .collect();
    let others: Vec<_> = snarl
        .node_ids()
        .filter(|(other, _)| *other != node && !needs.iter().any(|(from, _)| from.node == *other))
        .map(|(other, value)| (other, value.id().to_owned()))
        .collect();
    ui.weak("After");
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(FIELD_WIDTH);
        for (from, id) in &needs {
            let chip = ui
                .button(format!("{id} ×"))
                .on_hover_text("Remove this wire");
            if chip.clicked() {
                snarl.disconnect(*from, input(node));
            }
        }
        if needs.is_empty() {
            ui.weak("the start");
        }
        if !others.is_empty() {
            ui.menu_button("+", |ui| {
                for (other, id) in &others {
                    if ui.button(id).clicked() {
                        snarl.connect(output(*other), input(node));
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("Run this step after…");
        }
    });
    ui.end_row();
}

impl SnarlViewer<Node> for Viewer {
    fn current_transform(
        &mut self,
        to_global: &mut egui::emath::TSTransform,
        snarl: &mut Snarl<Node>,
    ) {
        if let Some(view) = self.fit.take().and_then(|canvas| fit_view(snarl, canvas)) {
            *to_global = view;
        }
    }

    fn title(&mut self, node: &Node) -> String {
        node.id().to_owned()
    }

    fn node_frame(
        &mut self,
        default: egui::Frame,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        snarl: &Snarl<Node>,
    ) -> egui::Frame {
        let gate = snarl.get_node(node).is_some_and(Node::is_gate);
        let (width, color) = if gate {
            (1.5, self.colors.gate)
        } else {
            (1.0, self.colors.border)
        };
        default
            .fill(self.colors.node)
            .corner_radius(appearance::RADIUS_LARGE)
            .stroke(egui::Stroke::new(width, color))
    }

    fn show_header(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) {
        // A label that takes clicks would hide the node's right-click menu.
        ui.style_mut().interaction.selectable_labels = false;
        let Some(value) = snarl.get_node_mut(node) else {
            return;
        };
        if value.is_gate() {
            ui.colored_label(self.colors.gate, "GATE");
        } else {
            ui.colored_label(self.colors.step, "STEP");
        }
        ui.add(
            egui::TextEdit::singleline(value.id_mut())
                .desired_width(ID_WIDTH)
                .hint_text("ID: a-z, 0-9, - or _"),
        );
        appearance::icon_menu_button(ui, "EllipsisVertical", |ui| node_menu(ui, node, snarl))
            .response
            .on_hover_text("More");
    }

    fn inputs(&mut self, _node: &Node) -> usize {
        1
    }

    fn show_input(
        &mut self,
        _pin: &InPin,
        _ui: &mut egui::Ui,
        _snarl: &mut Snarl<Node>,
    ) -> impl SnarlPin + 'static {
        PinInfo::circle().with_wire_color(self.colors.step)
    }

    fn outputs(&mut self, _node: &Node) -> usize {
        1
    }

    fn show_output(
        &mut self,
        _pin: &OutPin,
        _ui: &mut egui::Ui,
        _snarl: &mut Snarl<Node>,
    ) -> impl SnarlPin + 'static {
        PinInfo::circle().with_wire_color(self.colors.step)
    }

    fn has_body(&mut self, _node: &Node) -> bool {
        true
    }

    fn show_body(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) {
        ui.style_mut().interaction.selectable_labels = false;
        egui::Grid::new(("flow-node", node))
            .num_columns(2)
            .show(ui, |ui| {
                match snarl.get_node_mut(node) {
                    Some(Node::Step {
                        command,
                        prompt,
                        cwd,
                        worktree,
                        ..
                    }) => {
                        field(ui, ("Command", "For example claude"), command, 1);
                        field(ui, ("Prompt", "Optional"), prompt, 2);
                        field(ui, ("Folder", "In the project (optional)"), cwd, 1);
                        field(ui, ("Worktree", "New name (optional)"), worktree, 1);
                    }
                    Some(Node::Gate { message, .. }) => {
                        field(ui, ("Question", "What you approve"), message, 2);
                    }
                    None => return,
                }
                after_row(ui, node, snarl);
            });
    }

    fn has_graph_menu(&mut self, _pos: egui::Pos2, _snarl: &mut Snarl<Node>) -> bool {
        true
    }

    fn show_graph_menu(&mut self, pos: egui::Pos2, ui: &mut egui::Ui, snarl: &mut Snarl<Node>) {
        if ui.button("Add step").clicked() {
            add_node(snarl, pos, false);
            ui.close();
        }
        if ui.button("Add gate").clicked() {
            add_node(snarl, pos, true);
            ui.close();
        }
    }

    /// A wire that is dropped on the empty canvas makes the next (or the
    /// previous) node there, with the wire to it.
    fn has_dropped_wire_menu(&mut self, _src_pins: AnyPins, _snarl: &mut Snarl<Node>) -> bool {
        true
    }

    fn show_dropped_wire_menu(
        &mut self,
        pos: egui::Pos2,
        ui: &mut egui::Ui,
        src_pins: AnyPins,
        snarl: &mut Snarl<Node>,
    ) {
        let gate = if ui.button("New step here").clicked() {
            false
        } else if ui.button("New gate here").clicked() {
            true
        } else {
            return;
        };
        let node = add_node(snarl, pos, gate);
        match src_pins {
            AnyPins::Out(pins) => {
                for pin in pins {
                    snarl.connect(*pin, input(node));
                }
            }
            AnyPins::In(pins) => {
                for pin in pins {
                    snarl.connect(output(node), *pin);
                }
            }
        }
        ui.close();
    }

    fn has_node_menu(&mut self, _node: &Node) -> bool {
        true
    }

    fn show_node_menu(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) {
        node_menu(ui, node, snarl);
    }

    /// A step cannot need itself. A cycle through other steps is found by
    /// Save, with the name of a step in it.
    fn connect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<Node>) {
        if from.id.node != to.id.node {
            snarl.connect(from.id, to.id);
        }
    }
}
