//! How the flow editor's graph shows in `egui-snarl`: each node is a card
//! of one step. The dot on the left edge is "after" and the dot on the
//! right edge is "then": a wire from the right dot of A to the left dot of
//! B means that B needs A. The "After" row of a card shows the same wires
//! as chips, so a wire can be made and removed without a drag. A click on
//! the prompt (or the question) opens the card's dialog with large fields.
pub(super) use super::canvas::{add_node, fit_view, style};
use super::model::{Node, free_id, input, output};
use crate::appearance;
use crate::flows::panel::step_icon;
use egui_snarl::{
    InPin, NodeId, OutPin, Snarl,
    ui::{AnyPins, PinInfo, SnarlPin, SnarlViewer},
};
use std::collections::BTreeMap;
use terminator_core::flow::{StepRun, StepStatus};

/// Width of the fields in a card.
const FIELD_WIDTH: f32 = 250.0;
const ID_WIDTH: f32 = 130.0;
/// Lines and characters of a prompt that a card shows.
const PREVIEW_LINES: usize = 4;
const PREVIEW_CHARS: usize = 220;

/// The editor's colors, from the app theme.
#[derive(Clone, Copy)]
pub(super) struct Colors {
    pub(super) canvas: egui::Color32,
    pub(super) node: egui::Color32,
    pub(super) border: egui::Color32,
    /// Wires, dots and the label of a step.
    pub(super) step: egui::Color32,
    /// The outline and the label of a gate.
    pub(super) gate: egui::Color32,
    /// A step that failed in the shown run.
    pub(super) failed: egui::Color32,
}

/// The viewer of one frame.
pub(super) struct Viewer<'a> {
    pub(super) colors: Colors,
    /// The steps of the newest run of this flow, by step ID: each card
    /// shows its step's state.
    pub(super) run: Option<&'a BTreeMap<String, StepRun>>,
    /// Out: the session of the step whose terminal the user asked for.
    pub(super) go: Option<String>,
    /// Out: the gate that the user approved (true) or rejected.
    pub(super) answer: Option<(String, bool)>,
    /// The canvas rectangle when the view must move and zoom until all
    /// nodes show: on the first frame of a flow, and on Fit to view.
    pub(super) fit: Option<egui::Rect>,
    /// Out: the node whose dialog the user asked for.
    pub(super) edit: Option<NodeId>,
    /// Out: the user asked for Fit to view.
    pub(super) refit: bool,
}

/// The small label above a field.
fn caption(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).small().weak());
}

/// The start of `text` that a card shows.
fn preview(text: &str) -> String {
    let lines: Vec<&str> = text.trim().lines().take(PREVIEW_LINES).collect();
    let mut shown: String = lines.join("\n").chars().take(PREVIEW_CHARS).collect();
    if shown.len() < text.trim().len() {
        shown.push('…');
    }
    shown
}

impl<'a> Viewer<'a> {
    /// The state of a card's step in the shown run.
    fn step_run(&self, node: Option<&Node>) -> Option<&'a StepRun> {
        self.run?.get(node?.id().trim())
    }

    /// The color of a step's state in the shown run.
    fn run_color(&self, step: &StepRun) -> egui::Color32 {
        match step.status {
            _ if step.question.is_some() => self.colors.gate,
            StepStatus::Running if step.gate.is_some() => self.colors.gate,
            StepStatus::Running | StepStatus::Done => self.colors.step,
            StepStatus::Failed | StepStatus::Cancelled => self.colors.failed,
            StepStatus::Waiting => self.colors.border,
        }
    }

    /// The run row of a card: the state of its step, Approve and Reject
    /// for a gate that waits, and the way to the step's terminal.
    fn run_row(&mut self, ui: &mut egui::Ui, id: &str, step: &StepRun) {
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(FIELD_WIDTH);
            let (_, state) = step_icon(step);
            ui.label(
                egui::RichText::new(state)
                    .small()
                    .color(self.run_color(step)),
            );
            if step.status == StepStatus::Running && step.gate.is_some() {
                if ui.small_button("Approve").clicked() {
                    self.answer = Some((id.to_owned(), true));
                }
                if ui.small_button("Reject").clicked() {
                    self.answer = Some((id.to_owned(), false));
                }
            }
            if let Some(session) = &step.session
                && ui
                    .small_button("Show terminal")
                    .on_hover_text("Go to the terminal of this step")
                    .clicked()
            {
                self.go = Some(session.clone());
            }
        });
        if let Some(note) = &step.note {
            ui.label(egui::RichText::new(preview(note)).small().weak());
        }
    }

    /// A long text as a block that opens the card's dialog on a click.
    fn text_block(&self, ui: &mut egui::Ui, text: &str, empty: &str) -> bool {
        let shown = if text.trim().is_empty() {
            egui::RichText::new(empty).weak()
        } else {
            egui::RichText::new(preview(text))
        };
        let block = egui::Frame::new()
            .fill(self.colors.canvas)
            .stroke(egui::Stroke::new(1.0, self.colors.border))
            .corner_radius(appearance::RADIUS_MEDIUM)
            .inner_margin(egui::Margin::symmetric(8, 6))
            .show(ui, |ui| {
                ui.set_width(FIELD_WIDTH - 16.0);
                ui.add(egui::Label::new(shown).wrap().selectable(false));
            });
        block
            .response
            .interact(egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Edit in a large field")
            .clicked()
    }

    /// The fields of a card. True when the user asked for the dialog.
    fn fields(&self, ui: &mut egui::Ui, value: &mut Node) -> bool {
        match value {
            Node::Step {
                command,
                prompt,
                cwd,
                worktree,
                ..
            } => {
                caption(ui, "Command");
                ui.add(
                    egui::TextEdit::singleline(command)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(FIELD_WIDTH)
                        .hint_text("For example claude"),
                );
                caption(ui, "Prompt");
                let edit = self.text_block(ui, prompt, "No prompt. Click to write one.");
                let places = [("Folder", cwd), ("Worktree", worktree)];
                for (label, text) in places {
                    if !text.trim().is_empty() {
                        ui.label(egui::RichText::new(format!("{label}: {}", text.trim())).small());
                    }
                }
                edit
            }
            Node::Gate { message, .. } => {
                caption(ui, "You approve");
                self.text_block(ui, message, "No question. Click to write one.")
            }
        }
    }

    /// The menu of one card: on a right-click and on its "more" button.
    fn node_menu(&mut self, ui: &mut egui::Ui, node: NodeId, snarl: &mut Snarl<Node>) {
        if appearance::menu_action(ui, true, "Edit…", "Pencil").clicked() {
            self.edit = Some(node);
        }
        if appearance::menu_action(ui, true, "Duplicate", "Copy").clicked()
            && let Some(info) = snarl.get_node_info(node)
        {
            let mut copy = info.value.clone();
            let id = free_id(snarl, copy.id());
            *copy.id_mut() = id;
            let pos = egui::pos2(info.pos.x + 40.0, info.pos.y + 40.0);
            snarl.insert_node(pos, copy);
        }
        if appearance::menu_action(ui, true, "Remove wires", "Link").clicked() {
            snarl.drop_inputs(input(node));
            snarl.drop_outputs(output(node));
        }
        ui.separator();
        if appearance::menu_danger_action(ui, true, "Remove", "Trash").clicked() {
            snarl.remove_node(node);
        }
    }
}

/// The "After" row: one chip per step that this card needs (a click
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
    ui.horizontal_wrapped(|ui| {
        ui.set_max_width(FIELD_WIDTH);
        for (from, id) in &needs {
            let chip = ui
                .small_button(format!("{id} ×"))
                .on_hover_text("Remove this wire");
            if chip.clicked() {
                snarl.disconnect(*from, input(node));
            }
        }
        if needs.is_empty() {
            ui.label(egui::RichText::new("the start").small().weak());
        }
        if !others.is_empty() {
            appearance::compact_menu_button(ui, "+", |ui| {
                for (other, id) in &others {
                    if appearance::menu_action(ui, true, id, "Plus").clicked() {
                        snarl.connect(output(*other), input(node));
                    }
                }
            })
            .response
            .on_hover_text("Run this step after…");
        }
    });
}

impl SnarlViewer<Node> for Viewer<'_> {
    fn current_transform(
        &mut self,
        to_global: &mut egui::emath::TSTransform,
        snarl: &mut Snarl<Node>,
    ) {
        if let Some(view) = self.fit.take().and_then(|canvas| fit_view(snarl, canvas)) {
            *to_global = view;
        }
    }

    fn menu_style(&mut self) -> fn(&mut egui::Style) {
        appearance::menu_style
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
        let value = snarl.get_node(node);
        let gate = value.is_some_and(Node::is_gate);
        // In a run, the outline shows the state of the step.
        let active = self
            .step_run(value)
            .filter(|step| step.status != StepStatus::Waiting);
        let (width, color) = match active {
            Some(step) => (2.0, self.run_color(step)),
            None if gate => (1.5, self.colors.gate),
            None => (1.0, self.colors.border),
        };
        default
            .fill(self.colors.node)
            .corner_radius(appearance::RADIUS_LARGE)
            .stroke(egui::Stroke::new(width, color))
    }

    fn header_frame(
        &mut self,
        default: egui::Frame,
        _node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        _snarl: &Snarl<Node>,
    ) -> egui::Frame {
        // One flat card: no second box around the header.
        default
            .fill(egui::Color32::TRANSPARENT)
            .stroke(egui::Stroke::NONE)
    }

    fn show_header(
        &mut self,
        node: NodeId,
        _inputs: &[InPin],
        _outputs: &[OutPin],
        ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) {
        // A label that takes clicks would hide the card's right-click menu.
        ui.style_mut().interaction.selectable_labels = false;
        let Some(value) = snarl.get_node_mut(node) else {
            return;
        };
        let (label, color) = if value.is_gate() {
            ("GATE", self.colors.gate)
        } else {
            ("STEP", self.colors.step)
        };
        ui.label(egui::RichText::new(label).small().strong().color(color));
        ui.add(
            egui::TextEdit::singleline(value.id_mut())
                .desired_width(ID_WIDTH)
                .hint_text("ID: a-z, 0-9, - or _"),
        );
        if appearance::sidebar_action(ui, "Maximize2", "Edit in a dialog").clicked() {
            self.edit = Some(node);
        }
        appearance::icon_menu_button(ui, "EllipsisVertical", |ui| {
            self.node_menu(ui, node, snarl);
        })
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
        ui.vertical(|ui| {
            ui.set_width(FIELD_WIDTH);
            let Some(value) = snarl.get_node_mut(node) else {
                return;
            };
            if self.fields(ui, value) {
                self.edit = Some(node);
            }
            let id = value.id().trim().to_owned();
            caption(ui, "After");
            after_row(ui, node, snarl);
            if let Some(step) = self.run.and_then(|run| run.get(&id)) {
                self.run_row(ui, &id, step);
            }
        });
    }

    fn has_graph_menu(&mut self, _pos: egui::Pos2, _snarl: &mut Snarl<Node>) -> bool {
        true
    }

    fn show_graph_menu(&mut self, pos: egui::Pos2, ui: &mut egui::Ui, snarl: &mut Snarl<Node>) {
        if appearance::menu_action(ui, true, "Add step", "Terminal").clicked() {
            add_node(snarl, pos, false);
        }
        if appearance::menu_action(ui, true, "Add gate", "CircleCheck").clicked() {
            add_node(snarl, pos, true);
        }
        ui.separator();
        if appearance::menu_action(ui, true, "Fit to view", "Maximize").clicked() {
            self.refit = true;
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
        let gate = if appearance::menu_action(ui, true, "New step here", "Terminal").clicked() {
            false
        } else if appearance::menu_action(ui, true, "New gate here", "CircleCheck").clicked() {
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
        self.node_menu(ui, node, snarl);
    }

    /// A step cannot need itself. A cycle through other steps is found by
    /// Save, with the name of a step in it.
    fn connect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<Node>) {
        if from.id.node != to.id.node {
            snarl.connect(from.id, to.id);
        }
    }
}
