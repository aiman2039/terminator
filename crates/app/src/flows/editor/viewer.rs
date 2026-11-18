//! How the flow editor's graph shows in `egui-snarl`: each node is one
//! step with its fields in the node, with one "after" pin and one "then"
//! pin. A wire from "then" of A to "after" of B means that B needs A.
use super::model::{Node, free_id};
use egui_snarl::{
    InPin, NodeId, OutPin, Snarl,
    ui::{PinInfo, SnarlPin, SnarlViewer},
};

/// Width of the text fields in a node.
const FIELD_WIDTH: f32 = 230.0;
const ID_WIDTH: f32 = 150.0;

/// The viewer of one frame. `gate` is the outline color of a gate node.
pub(super) struct Viewer {
    pub(super) gate: egui::Color32,
    /// The canvas rectangle, on the first frame of a flow only: the view
    /// then moves and zooms out until all nodes show.
    pub(super) fit: Option<egui::Rect>,
}

/// The size of a step node, for the first view of a flow.
const NODE_SIZE: egui::Vec2 = egui::vec2(400.0, 280.0);
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

fn line(ui: &mut egui::Ui, text: &mut String, hint: &str) {
    ui.add(
        egui::TextEdit::singleline(text)
            .desired_width(FIELD_WIDTH)
            .hint_text(hint),
    );
}

fn lines(ui: &mut egui::Ui, text: &mut String, hint: &str) {
    ui.add(
        egui::TextEdit::multiline(text)
            .desired_width(FIELD_WIDTH)
            .desired_rows(2)
            .hint_text(hint),
    );
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
        if snarl.get_node(node).is_some_and(Node::is_gate) {
            default.stroke(egui::Stroke::new(default.stroke.width.max(1.5), self.gate))
        } else {
            default
        }
    }

    fn show_header(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) {
        let Some(value) = snarl.get_node_mut(node) else {
            return;
        };
        ui.weak(if value.is_gate() { "Gate" } else { "Step" });
        ui.add(
            egui::TextEdit::singleline(value.id_mut())
                .desired_width(ID_WIDTH)
                .hint_text("ID: a-z, 0-9, - or _"),
        );
    }

    fn inputs(&mut self, _node: &Node) -> usize {
        1
    }

    fn show_input(
        &mut self,
        _pin: &InPin,
        ui: &mut egui::Ui,
        _snarl: &mut Snarl<Node>,
    ) -> impl SnarlPin + 'static {
        ui.weak("after");
        PinInfo::circle()
    }

    fn outputs(&mut self, _node: &Node) -> usize {
        1
    }

    fn show_output(
        &mut self,
        _pin: &OutPin,
        ui: &mut egui::Ui,
        _snarl: &mut Snarl<Node>,
    ) -> impl SnarlPin + 'static {
        ui.weak("then");
        PinInfo::circle()
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
        let Some(value) = snarl.get_node_mut(node) else {
            return;
        };
        ui.vertical(|ui| match value {
            Node::Step {
                command,
                prompt,
                cwd,
                worktree,
                ..
            } => {
                line(ui, command, "Command, for example claude");
                lines(ui, prompt, "Prompt (optional)");
                line(ui, cwd, "Folder in the project (optional)");
                line(ui, worktree, "New worktree name (optional)");
            }
            Node::Gate { message, .. } => lines(ui, message, "Question for you"),
        });
    }

    fn has_graph_menu(&mut self, _pos: egui::Pos2, _snarl: &mut Snarl<Node>) -> bool {
        true
    }

    fn show_graph_menu(&mut self, pos: egui::Pos2, ui: &mut egui::Ui, snarl: &mut Snarl<Node>) {
        if ui.button("Add step").clicked() {
            let id = free_id(snarl, "step");
            snarl.insert_node(pos, Node::step(&id, "claude", ""));
            ui.close();
        }
        if ui.button("Add gate").clicked() {
            let id = free_id(snarl, "gate");
            snarl.insert_node(pos, Node::gate(&id, ""));
            ui.close();
        }
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
        if ui.button("Remove").clicked() {
            snarl.remove_node(node);
            ui.close();
        }
    }

    /// A step cannot need itself. A cycle through other steps is found by
    /// Save, with the name of a step in it.
    fn connect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<Node>) {
        if from.id.node != to.id.node {
            snarl.connect(from.id, to.id);
        }
    }
}
