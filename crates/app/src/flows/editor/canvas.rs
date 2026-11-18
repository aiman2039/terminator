//! The canvas of the flow editor's graph: its style, the view that shows
//! all nodes, and new nodes.
use super::model::{Node, free_id};
use super::viewer::Colors;
use egui_snarl::{
    NodeId, Snarl,
    ui::{PinPlacement, SnarlStyle, WireLayer},
};

/// The dots are large so that a wire is easy to start and to drop.
const PIN_SIZE: f32 = 14.0;
const WIRE_WIDTH: f32 = 2.5;

/// The graph style: dots on the card edges, thick wires behind the cards,
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
        bg_pattern_stroke: Some(egui::Stroke::new(1.0, colors.border)),
        min_scale: Some(0.3),
        max_scale: Some(1.5),
        ..SnarlStyle::new()
    }
}

/// The size of a step card, for the first view of a flow.
const NODE_SIZE: egui::Vec2 = egui::vec2(300.0, 250.0);
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

/// Adds a step or a gate at `pos` and gives its node.
pub(super) fn add_node(snarl: &mut Snarl<Node>, pos: egui::Pos2, gate: bool) -> NodeId {
    if gate {
        let id = free_id(snarl, "gate");
        snarl.insert_node(pos, Node::gate(&id, ""))
    } else {
        let id = free_id(snarl, "step");
        snarl.insert_node(pos, Node::step(&id, "claude", ""))
    }
}
