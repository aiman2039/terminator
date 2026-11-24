//! The canvas of the flow editor's graph: its style, the view that shows
//! all nodes, and new nodes.
use super::model::{Node, free_id};
use super::style::Colors;
use egui_snarl::{
    NodeId, Snarl,
    ui::{PinPlacement, SnarlStyle, WireLayer},
};
use std::collections::HashMap;

/// The dots are large so that a wire is easy to start and to drop.
const PIN_SIZE: f32 = 14.0;
const WIRE_WIDTH: f32 = 2.5;

/// The graph style: dots on the card edges, thick wires behind the cards,
/// a quiet grid, no collapse button. Each pin sets its own fill and wire
/// color by the state of its step (see `viewer`).
pub(super) fn style(colors: Colors) -> SnarlStyle {
    SnarlStyle {
        collapsible: Some(false),
        header_drag_space: Some(egui::Vec2::ZERO),
        pin_placement: Some(PinPlacement::Edge),
        pin_size: Some(PIN_SIZE),
        pin_fill: Some(colors.node),
        pin_stroke: Some(egui::Stroke::new(2.0, colors.border)),
        wire_width: Some(WIRE_WIDTH),
        wire_layer: Some(WireLayer::BehindNodes),
        bg_frame: Some(egui::Frame::new().fill(colors.canvas)),
        bg_pattern_stroke: Some(egui::Stroke::new(1.0, colors.border)),
        min_scale: Some(0.3),
        max_scale: Some(1.5),
        ..SnarlStyle::new()
    }
}

/// The size of a card that has not painted yet: about a compact card.
pub(super) const NODE_SIZE: egui::Vec2 = egui::vec2(230.0, 190.0);
const FIT_MARGIN: f32 = 40.0;
/// Fit to view never zooms out past this: below it the card text is too
/// small to read. A larger flow then shows its first cards and pans.
pub(super) const FIT_MIN_ZOOM: f32 = 0.60;

/// The measured card sizes of an editor, from the last frame.
pub(super) type Sizes = HashMap<NodeId, egui::Vec2>;

/// The graph rectangle of every card: its measured size, or
/// [`NODE_SIZE`] for a card that has not painted yet.
pub(super) fn bounds(snarl: &Snarl<Node>, sizes: &Sizes) -> egui::Rect {
    let mut all = egui::Rect::NOTHING;
    for (node, position, _) in snarl.nodes_pos_ids() {
        let size = sizes.get(&node).copied().unwrap_or(NODE_SIZE);
        all = all.union(egui::Rect::from_min_size(position, size));
    }
    all
}

/// The view in which all cards show in `canvas`, at most at full size and
/// at least at [`FIT_MIN_ZOOM`]. `inspector` is the width of the inspector
/// at the right of the canvas: a flow that fits the whole pane at
/// [`FIT_MIN_ZOOM`] also fits the canvas beside the inspector, with a
/// smaller zoom, so no card is cut off behind the inspector.
pub(super) fn fit_view(
    snarl: &Snarl<Node>,
    sizes: &Sizes,
    (canvas, inspector): (egui::Rect, f32),
) -> Option<egui::emath::TSTransform> {
    let all = bounds(snarl, sizes);
    if !all.is_finite() || !canvas.is_finite() || canvas.width() < 1.0 || canvas.height() < 1.0 {
        return None;
    }
    let all = all.expand(FIT_MARGIN);
    let pane = canvas.width() + inspector.max(0.0);
    let floor_x = if all.width() * FIT_MIN_ZOOM <= pane {
        FIT_MIN_ZOOM * canvas.width() / pane
    } else {
        FIT_MIN_ZOOM
    };
    let scaling = (canvas.width() / all.width())
        .max(floor_x)
        .min((canvas.height() / all.height()).max(FIT_MIN_ZOOM))
        .min(1.0);
    // Centered on each axis that fits; else the first cards (left, top)
    // show and the user pans to the rest.
    let place = |from: egui::Rangef, to: egui::Rangef| {
        if from.span() * scaling <= to.span() {
            to.center() - from.center() * scaling
        } else {
            to.min - from.min * scaling
        }
    };
    let offset = egui::vec2(
        place(all.x_range(), canvas.x_range()),
        place(all.y_range(), canvas.y_range()),
    );
    Some(egui::emath::TSTransform::new(offset, scaling))
}

/// True when every card of the graph has a measured size.
pub(super) fn measured(snarl: &Snarl<Node>, sizes: &Sizes) -> bool {
    snarl.node_ids().all(|(node, _)| sizes.contains_key(&node))
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

/// A copy of card `node` beside it, with a free ID and no wires.
pub(super) fn duplicate(snarl: &mut Snarl<Node>, node: NodeId) -> Option<NodeId> {
    let info = snarl.get_node_info(node)?;
    let mut copy = info.value.clone();
    let id = free_id(snarl, copy.id());
    *copy.id_mut() = id;
    match &mut copy {
        Node::Step { worktree, .. } => worktree.clear(),
        Node::Gate { pick, .. } => pick.clear(),
    }
    let branch = copy.branch_mut();
    branch.when.clear();
    branch.loop_to.clear();
    let pos = egui::pos2(info.pos.x + 40.0, info.pos.y + 40.0);
    Some(snarl.insert_node(pos, copy))
}
