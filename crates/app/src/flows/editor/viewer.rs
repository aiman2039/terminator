//! How the flow editor's graph shows in `egui-snarl`: each node is a
//! compact, read-only card of one step (`cards`). The dot on the left edge
//! is "after" and the dot on the right edge is "then": a wire from the
//! right dot of A to the left dot of B means that B needs A. A click on a
//! card selects it, and the inspector beside the canvas edits it. In a
//! run, the dots and wires take the color of their source step's state.
pub(super) use super::canvas::{Sizes, add_node, duplicate, fit_view, style};
use super::inspector::CheckError;
use super::model::{Node, input, output, would_cycle};
use super::style::Colors;
use crate::appearance;
use crate::flows::status::{StepStyle, chip, step_style};
use egui_snarl::{
    InPin, NodeId, OutPin, Snarl,
    ui::{AnyPins, PinInfo, SnarlPin, SnarlViewer},
};
use std::collections::{BTreeMap, HashMap};
use terminator_core::flow::{StepRun, StepStatus};

/// Width of a card's content.
pub(super) const CARD_WIDTH: f32 = 196.0;

/// What the user did on the canvas this frame.
#[derive(Default)]
pub(super) struct Out {
    /// The session of the step whose terminal the user asked for.
    pub(super) go: Option<String>,
    /// The gate that the user answered, with the answer.
    pub(super) answer: Option<(String, crate::flows::gates::GateChoice)>,
    /// The question step whose typed answer the user sent.
    pub(super) send: Option<String>,
    /// The failed step that the user asked to start again.
    pub(super) retry: Option<String>,
    /// The card that the user clicked or asked to edit.
    pub(super) select: Option<NodeId>,
    /// A primary click landed on a card.
    pub(super) on_card: bool,
    /// The user asked for Fit to view, or Tidy layout.
    pub(super) refit: bool,
    pub(super) tidy: bool,
}

/// The viewer of one frame.
pub(super) struct Viewer<'a> {
    pub(super) colors: Colors,
    /// The steps of the newest run of this flow, by step ID: each card
    /// shows its step's state.
    pub(super) run: Option<&'a BTreeMap<String, StepRun>>,
    /// The shown run's runner lives: a failed step has Retry.
    pub(super) live: bool,
    /// The stall badge of each step that has one, by step ID.
    pub(super) stalls: HashMap<String, String>,
    /// Typed answers of the shown run's questions, by step ID.
    pub(super) drafts: &'a mut HashMap<String, String>,
    pub(super) selected: Option<NodeId>,
    /// The first check error of the graph, on the card it concerns.
    pub(super) error: Option<&'a CheckError>,
    /// The measured card sizes: written each frame, read by Fit.
    pub(super) sizes: &'a mut Sizes,
    /// The canvas rectangle when the view must move and zoom until all
    /// cards show: on the first frames of a flow, on Fit to view and after
    /// a resize of the canvas. With it, the width of the inspector at the
    /// right of the canvas (see [`fit_view`]).
    pub(super) fit: Option<(egui::Rect, f32)>,
    pub(super) out: Out,
}

impl<'a> Viewer<'a> {
    /// The state of a card's step in the shown run.
    pub(super) fn step_run(&self, node: Option<&Node>) -> Option<&'a StepRun> {
        self.run?.get(node?.id().trim())
    }

    /// The look of a card's step: `None` without a run.
    fn style_of(&self, node: Option<&Node>) -> Option<StepStyle> {
        self.step_run(node).map(step_style)
    }

    /// The color of a wire from `node`: its state in the run, or a quiet
    /// color when the run did not reach it or there is no run.
    fn wire_color(&self, node: Option<&Node>) -> egui::Color32 {
        match self.style_of(node) {
            Some(style) => self.colors.wire(style),
            None => self.colors.weak,
        }
    }

    /// True when the check error concerns this card.
    fn has_error(&self, node: Option<&Node>) -> bool {
        let (Some(error), Some(node)) = (self.error, node) else {
            return false;
        };
        error.step.as_deref() == Some(node.id().trim())
    }

    /// The menu of one card: on a right-click and on its "more" button.
    fn node_menu(&mut self, ui: &mut egui::Ui, node: NodeId, snarl: &mut Snarl<Node>) {
        if appearance::menu_action(ui, true, "Edit…", "Pencil").clicked() {
            self.out.select = Some(node);
        }
        if appearance::menu_action(ui, true, "Duplicate", "Copy").clicked() {
            self.out.select = duplicate(snarl, node).or(self.out.select);
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

    fn pin(&self, color: egui::Color32, reached: bool) -> PinInfo {
        let fill = if reached { color } else { self.colors.node };
        PinInfo::circle()
            .with_fill(fill)
            .with_stroke(egui::Stroke::new(2.0, color))
            .with_wire_color(color)
    }
}

impl SnarlViewer<Node> for Viewer<'_> {
    fn current_transform(
        &mut self,
        to_global: &mut egui::emath::TSTransform,
        snarl: &mut Snarl<Node>,
    ) {
        if let Some(view) = self
            .fit
            .take()
            .and_then(|area| fit_view(snarl, self.sizes, area))
        {
            *to_global = view;
        }
    }

    fn menu_style(&mut self) -> fn(&mut egui::Style) {
        appearance::menu_style
    }

    /// The grid, then each loop as a dashed wire behind the cards.
    fn draw_background(
        &mut self,
        background: Option<&egui_snarl::ui::BackgroundPattern>,
        viewport: &egui::Rect,
        snarl_style: &egui_snarl::ui::SnarlStyle,
        style: &egui::Style,
        painter: &egui::Painter,
        snarl: &Snarl<Node>,
    ) {
        if let Some(background) = background {
            background.draw(viewport, snarl_style, style, painter);
        }
        super::branching::paint_loops(painter, snarl, self.sizes, self.colors.gate);
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
        // The selection first, then a check error, then the run's state.
        let active = self.style_of(value).filter(|style| style.reached);
        let (width, color) = if self.selected == Some(node) {
            (2.5, self.colors.accent)
        } else if self.has_error(value) {
            (2.0, self.colors.failed)
        } else if let Some(style) = active {
            (2.0, self.colors.of_style(style))
        } else if gate {
            (1.5, self.colors.gate.gamma_multiply(0.7))
        } else {
            (1.0, self.colors.border)
        };
        default
            .fill(self.colors.node)
            .corner_radius(appearance::RADIUS_LARGE)
            .inner_margin(egui::Margin::symmetric(12, 8))
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
        let Some(value) = snarl.get_node(node) else {
            return;
        };
        let style = self.style_of(Some(value));
        let error = self.has_error(Some(value));
        let (icon, color) = if value.is_gate() {
            ("ShieldQuestion", self.colors.gate)
        } else {
            ("Terminal", self.colors.step)
        };
        let id = value.id().trim().to_owned();
        ui.horizontal(|ui| {
            ui.set_width(CARD_WIDTH);
            let size = egui::Vec2::splat(appearance::TOOLBAR_ICON);
            ui.add(
                egui::Image::new(crate::icons::source(icon))
                    .tint(color)
                    .fit_to_exact_size(size),
            );
            let shown = if id.is_empty() {
                egui::RichText::new("no ID").weak()
            } else {
                egui::RichText::new(&id).strong()
            };
            ui.add(egui::Label::new(shown).truncate());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                appearance::icon_menu_button(ui, "EllipsisVertical", |ui| {
                    self.node_menu(ui, node, snarl);
                })
                .response
                .on_hover_text("More");
                if error {
                    chip(ui, "error", self.colors.failed);
                } else if let Some(style) = style {
                    chip(ui, style.chip, self.colors.of_style(style)).on_hover_text(style.label);
                }
            });
        });
    }

    fn inputs(&mut self, _node: &Node) -> usize {
        1
    }

    /// The dot and the wires into a step: the color of the steps it needs.
    /// Of several, the one the run reached least.
    fn show_input(
        &mut self,
        pin: &InPin,
        _ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) -> impl SnarlPin + 'static {
        let sources: Vec<Option<StepStyle>> = pin
            .remotes
            .iter()
            .map(|from| self.style_of(snarl.get_node(from.node)))
            .collect();
        let rank = |style: &Option<StepStyle>| match style {
            None => 0,
            Some(style) if !style.reached => 1,
            Some(_) => 2,
        };
        let least = sources.iter().min_by_key(|style| rank(style));
        match least {
            Some(Some(style)) => self.pin(self.colors.wire(*style), style.reached),
            _ => self.pin(self.colors.weak, false),
        }
    }

    fn outputs(&mut self, _node: &Node) -> usize {
        1
    }

    /// The dot and the wires out of a step: the color of its own state.
    fn show_output(
        &mut self,
        pin: &OutPin,
        _ui: &mut egui::Ui,
        snarl: &mut Snarl<Node>,
    ) -> impl SnarlPin + 'static {
        let value = snarl.get_node(pin.id.node);
        let reached = self.style_of(value).is_some_and(|style| style.reached);
        self.pin(self.wire_color(value), reached)
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
        let Some(value) = snarl.get_node(node).cloned() else {
            return;
        };
        let step = self.step_run(Some(&value));
        ui.vertical(|ui| {
            ui.set_width(CARD_WIDTH);
            super::cards::summary(ui, &value, self.colors);
            if let Some(step) = step {
                self.footer(ui, value.id().trim(), step);
            }
        });
    }

    /// The size of each card for Fit to view, and the click that selects
    /// it. A click on a button of the card also selects the card.
    fn final_node_rect(
        &mut self,
        node: NodeId,
        rect: egui::Rect,
        ui: &mut egui::Ui,
        _snarl: &mut Snarl<Node>,
    ) {
        self.sizes.insert(node, rect.size());
        if clicked_in(ui, rect) {
            self.out.select = Some(node);
            self.out.on_card = true;
        }
    }

    fn has_graph_menu(&mut self, _pos: egui::Pos2, _snarl: &mut Snarl<Node>) -> bool {
        true
    }

    fn show_graph_menu(&mut self, pos: egui::Pos2, ui: &mut egui::Ui, snarl: &mut Snarl<Node>) {
        if appearance::menu_action(ui, true, "Add step", "Terminal").clicked() {
            self.out.select = Some(add_node(snarl, pos, false));
        }
        if appearance::menu_action(ui, true, "Add gate", "ShieldQuestion").clicked() {
            self.out.select = Some(add_node(snarl, pos, true));
        }
        ui.separator();
        if appearance::menu_action(ui, true, "Fit to view", "Maximize").clicked() {
            self.out.refit = true;
        }
        if appearance::menu_action(ui, true, "Tidy layout", "LayoutDashboard").clicked() {
            self.out.tidy = true;
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
        } else if appearance::menu_action(ui, true, "New gate here", "ShieldQuestion").clicked() {
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
        self.out.select = Some(node);
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

    /// A wire that would close a cycle (also a step after itself) is not
    /// made.
    fn connect(&mut self, from: &OutPin, to: &InPin, snarl: &mut Snarl<Node>) {
        if !would_cycle(snarl, from.id.node, to.id.node) {
            snarl.connect(from.id, to.id);
        }
    }
}

/// A primary click this frame in `rect` of `ui`'s (zoomed) layer, with no
/// popup or window above it.
pub(super) fn clicked_in(ui: &egui::Ui, rect: egui::Rect) -> bool {
    let Some(pos) = ui.input(|input| {
        input
            .pointer
            .primary_clicked()
            .then(|| input.pointer.interact_pos())
            .flatten()
    }) else {
        return false;
    };
    let ctx = ui.ctx();
    let rect = ctx
        .layer_transform_to_global(ui.layer_id())
        .map_or(rect, |to_global| to_global.mul_rect(rect));
    rect.contains(pos)
        && ctx
            .layer_id_at(pos)
            .is_some_and(|layer| layer.order == ui.layer_id().order)
}

/// True for a step whose gate waits for the user's approval.
pub(super) fn gate_waits(step: &StepRun) -> bool {
    step.status == StepStatus::Running && step.gate.is_some()
}
