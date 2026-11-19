//! Branches and loops in the flow editor: the inspector fields of a gate's
//! options, a step's `when` condition and its loop, and the dashed wire of
//! each loop on the canvas. The file rules check them on Save.
use super::FlowEditor;
use super::canvas::{NODE_SIZE, Sizes};
use super::inspector::{Field, field};
use super::model::{Node, input};
use crate::appearance;
use egui_snarl::{NodeId, Snarl};
use std::collections::BTreeSet;
use terminator_core::flow::{MAX_ROUNDS, MIN_ROUNDS};

/// How far below the lower card a loop wire goes.
const LOOP_DROP: f32 = 56.0;
const DASH: f32 = 7.0;
const GAP: f32 = 5.0;

/// The IDs of the steps that `node` needs, directly or through others.
fn earlier(snarl: &Snarl<Node>, node: NodeId) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut next = vec![node];
    while let Some(at) = next.pop() {
        for from in snarl.in_pin(input(at)).remotes {
            if seen.insert(from.node) {
                next.push(from.node);
                if let Some(value) = snarl.get_node(from.node) {
                    found.insert(value.id().trim().to_owned());
                }
            }
        }
    }
    found
}

/// The `GATE.OPTION` conditions that `node` can have: each option of each
/// gate that it needs directly.
fn conditions(snarl: &Snarl<Node>, node: NodeId) -> Vec<String> {
    snarl
        .in_pin(input(node))
        .remotes
        .iter()
        .filter_map(|from| snarl.get_node(from.node))
        .flat_map(|gate| {
            let id = gate.id().trim().to_owned();
            gate.options()
                .into_iter()
                .map(move |option| format!("{id}.{option}"))
        })
        .collect()
}

/// A weak line with the good values of a field.
fn choices(ui: &mut egui::Ui, what: &str, values: &[String]) {
    let text = if values.is_empty() {
        format!("No {what} yet.")
    } else {
        format!("Can be: {}", values.join(", "))
    };
    ui.label(
        egui::RichText::new(text)
            .size(appearance::FONT_CAPTION)
            .weak(),
    );
    ui.add_space(4.0);
}

/// The options of a gate, the condition and the loop of the card `node`,
/// each with the check error of its field.
pub(super) fn fields<'a>(
    ui: &mut egui::Ui,
    editor: &mut FlowEditor,
    node: NodeId,
    error: impl Fn(Field) -> Option<&'a str>,
) {
    let conditions = conditions(&editor.snarl, node);
    let earlier: Vec<String> = earlier(&editor.snarl, node).into_iter().collect();
    let Some(value) = editor.snarl.get_node_mut(node) else {
        return;
    };
    if let Node::Gate { options, .. } = value {
        let hint = "For example: ship, rework (optional)";
        field(ui, ("Options", hint, 1), options, error(Field::Options));
    }
    let branch = value.branch_mut();
    field(
        ui,
        ("Runs when", "GATE.OPTION (optional)", 1),
        &mut branch.when,
        error(Field::When),
    );
    choices(ui, "gate options before this step", &conditions);
    field(
        ui,
        ("Loop back to", "An earlier step (optional)", 1),
        &mut branch.loop_to,
        error(Field::Loop),
    );
    if branch.loop_to.trim().is_empty() {
        choices(ui, "earlier steps", &earlier);
        return;
    }
    let slider = egui::Slider::new(&mut branch.max_rounds, MIN_ROUNDS..=MAX_ROUNDS).text("rounds");
    appearance::slider(ui, slider).on_hover_text(
        "When this step is done, the run starts again from that step, until this many rounds",
    );
    ui.add_space(6.0);
}

/// The cards of each loop: the loop step and the step it goes back to.
fn loops(snarl: &Snarl<Node>) -> Vec<(NodeId, NodeId, u32)> {
    let find = |id: &str| {
        snarl
            .node_ids()
            .find(|(_, value)| value.id().trim() == id)
            .map(|(node, _)| node)
    };
    snarl
        .node_ids()
        .filter_map(|(node, value)| {
            let branch = value.branch();
            let to = find(branch.loop_to.trim()).filter(|to| *to != node)?;
            Some((node, to, branch.max_rounds))
        })
        .collect()
}

/// Paints each loop as a dashed wire below the cards, from the bottom of
/// the loop step to the bottom of the step it goes back to, with an arrow
/// and "loop, N rounds". In graph space, behind the cards.
pub(super) fn paint_loops(
    painter: &egui::Painter,
    snarl: &Snarl<Node>,
    sizes: &Sizes,
    color: egui::Color32,
) {
    let rect = |node: NodeId| {
        let pos = snarl.get_node_info(node).map(|info| info.pos)?;
        let size = sizes.get(&node).copied().unwrap_or(NODE_SIZE);
        Some(egui::Rect::from_min_size(pos, size))
    };
    let stroke = egui::Stroke::new(2.0, color);
    for (from, to, rounds) in loops(snarl) {
        let (Some(from), Some(to)) = (rect(from), rect(to)) else {
            continue;
        };
        let (start, end) = (from.center_bottom(), to.center_bottom());
        let low = from.bottom().max(to.bottom()) + LOOP_DROP;
        let curve = egui::epaint::CubicBezierShape::from_points_stroke(
            [start, egui::pos2(start.x, low), egui::pos2(end.x, low), end],
            false,
            egui::Color32::TRANSPARENT,
            stroke,
        );
        let points = curve.flatten(Some(0.5));
        painter.extend(egui::Shape::dashed_line(&points, stroke, DASH, GAP));
        // The arrow points up into the step that the loop goes back to.
        let head = [
            end,
            egui::pos2(end.x - 6.0, end.y + 10.0),
            egui::pos2(end.x + 6.0, end.y + 10.0),
        ];
        painter.add(egui::Shape::convex_polygon(
            head.to_vec(),
            color,
            egui::Stroke::NONE,
        ));
        let middle = egui::pos2(f32::midpoint(start.x, end.x), low - LOOP_DROP / 4.0);
        painter.text(
            middle,
            egui::Align2::CENTER_BOTTOM,
            format!("loop, {rounds} rounds"),
            egui::FontId::proportional(appearance::FONT_CAPTION),
            color,
        );
    }
}
