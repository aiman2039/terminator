//! The flow editor's graph: one node per step, one wire per `needs`. It
//! converts to and from the flow file's draft form; the checks are the
//! file format's own (`terminator_core::flow::Draft::check`).
use egui_snarl::{InPinId, NodeId, OutPinId, Snarl};
use std::collections::HashMap;
use terminator_core::flow::{Draft, DraftStep, Flow, StepKind};

pub(crate) use super::plan::plan_text;

/// The condition and the loop of a step or a gate, as the user typed them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Branch {
    /// `GATE.OPTION`, or empty: the step always runs.
    pub(crate) when: String,
    /// The step that this one runs again from, or empty: no loop.
    pub(crate) loop_to: String,
    /// The most rounds of the loop; written only with `loop_to`.
    pub(crate) max_rounds: u32,
}

impl Default for Branch {
    fn default() -> Self {
        Self {
            when: String::new(),
            loop_to: String::new(),
            max_rounds: 3,
        }
    }
}

/// One node of the editor. Text fields hold what the user typed; an empty
/// optional field is left out of the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Node {
    /// A command step: a new terminal with `command`, then `prompt`.
    Step {
        id: String,
        command: String,
        prompt: String,
        cwd: String,
        worktree: String,
        branch: Branch,
    },
    /// A gate: the run waits for the user's approval of `message`, with
    /// one of `options` (comma-separated, as typed) when it has some.
    Gate {
        id: String,
        message: String,
        options: String,
        branch: Branch,
    },
}

impl Node {
    pub(crate) fn step(id: &str, command: &str, prompt: &str) -> Self {
        Self::Step {
            id: id.to_owned(),
            command: command.to_owned(),
            prompt: prompt.to_owned(),
            cwd: String::new(),
            worktree: String::new(),
            branch: Branch::default(),
        }
    }

    pub(crate) fn gate(id: &str, message: &str) -> Self {
        Self::Gate {
            id: id.to_owned(),
            message: message.to_owned(),
            options: String::new(),
            branch: Branch::default(),
        }
    }

    pub(crate) fn branch(&self) -> &Branch {
        match self {
            Self::Step { branch, .. } | Self::Gate { branch, .. } => branch,
        }
    }

    pub(crate) fn branch_mut(&mut self) -> &mut Branch {
        match self {
            Self::Step { branch, .. } | Self::Gate { branch, .. } => branch,
        }
    }

    /// The options of a gate, as the file has them; none for a step.
    pub(crate) fn options(&self) -> Vec<String> {
        match self {
            Self::Gate { options, .. } => split_options(options),
            Self::Step { .. } => Vec::new(),
        }
    }

    pub(crate) fn id(&self) -> &str {
        match self {
            Self::Step { id, .. } | Self::Gate { id, .. } => id,
        }
    }

    pub(crate) fn id_mut(&mut self) -> &mut String {
        match self {
            Self::Step { id, .. } | Self::Gate { id, .. } => id,
        }
    }

    pub(crate) fn is_gate(&self) -> bool {
        matches!(self, Self::Gate { .. })
    }
}

/// Distance between the columns of an automatic layout: a card is about
/// 220 wide.
pub(crate) const COLUMN: i32 = 320;
/// Distance between the rows of an automatic layout when the card heights
/// are not known yet: more than the tallest card (see `cards`).
pub(crate) const ROW: i32 = 300;
/// Space between two cards of one column in a tidy layout.
const GAP: f32 = 40.0;

/// The pin of a node's `needs` side and of its "then" side.
pub(crate) fn input(node: NodeId) -> InPinId {
    InPinId { node, input: 0 }
}

pub(crate) fn output(node: NodeId) -> OutPinId {
    OutPinId { node, output: 0 }
}

/// A place for each step of a flow that has none in its file: a column per
/// depth in the order (a step is one column after the last step it
/// needs), a row per step in that column.
fn auto_places(flow: &Flow) -> Vec<[i32; 2]> {
    let index: HashMap<&str, usize> = flow
        .steps
        .iter()
        .enumerate()
        .map(|(at, step)| (step.id.as_str(), at))
        .collect();
    let needs: Vec<Vec<usize>> = flow
        .steps
        .iter()
        .map(|step| {
            step.needs
                .iter()
                .filter_map(|needs| index.get(needs.as_str()).copied())
                .collect()
        })
        .collect();
    let mut rows: HashMap<i32, i32> = HashMap::new();
    depths(&needs)
        .into_iter()
        .map(|column| {
            let row = rows.entry(column).or_insert(0);
            let place = [column.saturating_mul(COLUMN), row.saturating_mul(ROW)];
            *row = row.saturating_add(1);
            place
        })
        .collect()
}

/// The column of each step: one after the last step that it needs. A
/// step of a cycle stops growing after one pass per step.
fn depths(needs: &[Vec<usize>]) -> Vec<i32> {
    let mut depth = vec![0_i32; needs.len()];
    for _ in 0..needs.len() {
        for (at, before) in needs.iter().enumerate() {
            let after = before
                .iter()
                .filter_map(|needs| depth.get(*needs))
                .map(|depth| depth.saturating_add(1).min(1_000))
                .max()
                .unwrap_or(0);
            if let Some(slot) = depth.get_mut(at) {
                *slot = after;
            }
        }
    }
    depth
}

/// Tidy layout: a column per depth, and in each column the cards in their
/// order from top to bottom, each `GAP` below the card above it. `size`
/// gives a card's measured size; a card with none counts as `ROW` high.
pub(crate) fn tidy(snarl: &mut Snarl<Node>, size: impl Fn(NodeId) -> Option<egui::Vec2>) {
    let nodes: Vec<(NodeId, egui::Pos2)> = snarl
        .nodes_pos_ids()
        .map(|(node, position, _)| (node, position))
        .collect();
    let at = |node: NodeId| nodes.iter().position(|(other, _)| *other == node);
    let needs: Vec<Vec<usize>> = nodes
        .iter()
        .map(|(node, _)| {
            let remotes = snarl.in_pin(input(*node)).remotes;
            remotes.iter().filter_map(|from| at(from.node)).collect()
        })
        .collect();
    let depth = depths(&needs);
    let mut order: Vec<usize> = (0..nodes.len()).collect();
    order.sort_by(|a, b| {
        let key = |index: &usize| {
            let column = depth.get(*index).copied().unwrap_or(0);
            let y = nodes.get(*index).map_or(0.0, |(_, pos)| pos.y);
            (column, y)
        };
        let ((left, top), (right, bottom)) = (key(a), key(b));
        left.cmp(&right).then(top.total_cmp(&bottom))
    });
    let mut tops: HashMap<i32, f32> = HashMap::new();
    for index in order {
        let (Some((node, _)), Some(column)) = (nodes.get(index), depth.get(index)) else {
            continue;
        };
        let top = tops.entry(*column).or_insert(0.0);
        let height = size(*node).map_or(position([0, ROW]).y, |size| size.y);
        let place = egui::pos2(position([column.saturating_mul(COLUMN), 0]).x, *top);
        *top += height + GAP;
        if let Some(info) = snarl.get_node_info_mut(*node) {
            info.pos = place;
        }
    }
}

/// Moves cards down until no card covers another: a card that a card
/// above it (by its measured size) reaches moves to `GAP` below that card.
/// Cards with no measured size stay. True when a card moved.
pub(crate) fn push_apart(
    snarl: &mut Snarl<Node>,
    size: impl Fn(NodeId) -> Option<egui::Vec2>,
) -> bool {
    let mut cards: Vec<(NodeId, egui::Rect)> = snarl
        .nodes_pos_ids()
        .filter_map(|(node, position, _)| {
            Some((node, egui::Rect::from_min_size(position, size(node)?)))
        })
        .collect();
    cards.sort_by(|a, b| a.1.top().total_cmp(&b.1.top()));
    let mut placed: Vec<egui::Rect> = Vec::with_capacity(cards.len());
    let mut moved = false;
    for (node, mut rect) in cards {
        // Each move goes down only, so one pass per placed card ends it.
        for _ in 0..=placed.len() {
            let Some(above) = placed.iter().find(|above| above.intersects(rect)) else {
                break;
            };
            rect = rect.translate(egui::vec2(0.0, above.bottom() + GAP - rect.top()));
            moved = true;
            if let Some(info) = snarl.get_node_info_mut(node) {
                info.pos = rect.min;
            }
        }
        placed.push(rect);
    }
    moved
}

/// True when `to` may not run after `from`: the same step, or `from`
/// already runs after `to` (the wire would close a cycle).
pub(crate) fn would_cycle(snarl: &Snarl<Node>, from: NodeId, to: NodeId) -> bool {
    let mut stack = vec![to];
    let mut seen = std::collections::HashSet::new();
    while let Some(node) = stack.pop() {
        if node == from {
            return true;
        }
        if seen.insert(node) {
            let next = snarl.out_pin(output(node)).remotes;
            stack.extend(next.iter().map(|pin| pin.node));
        }
    }
    false
}

/// A graph position from a saved place.
#[expect(
    clippy::cast_precision_loss,
    reason = "Editor coordinates are small whole numbers."
)]
fn position(place: [i32; 2]) -> egui::Pos2 {
    let [x, y] = place;
    egui::pos2(x as f32, y as f32)
}

/// The saved place of a graph position.
#[expect(
    clippy::cast_possible_truncation,
    reason = "Clamped to a range that an i32 holds."
)]
fn place(position: egui::Pos2) -> [i32; 2] {
    let whole = |value: f32| value.round().clamp(-1_000_000.0, 1_000_000.0) as i32;
    [whole(position.x), whole(position.y)]
}

/// The options typed in a gate's field: comma-separated names.
pub(crate) fn split_options(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|option| !option.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The graph of a checked flow.
pub(crate) fn graph_of(flow: &Flow) -> Snarl<Node> {
    let mut snarl = Snarl::new();
    let mut ids = HashMap::new();
    for (step, auto) in flow.steps.iter().zip(auto_places(flow)) {
        let branch = Branch {
            when: step
                .when
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            loop_to: step
                .loop_back
                .as_ref()
                .map(|back| back.to.clone())
                .unwrap_or_default(),
            max_rounds: step.loop_back.as_ref().map_or(3, |back| back.max_rounds),
        };
        let node = match &step.kind {
            StepKind::Command {
                command,
                prompt,
                cwd,
                worktree,
            } => Node::Step {
                id: step.id.clone(),
                command: command.clone(),
                prompt: prompt.clone().unwrap_or_default(),
                cwd: cwd.clone().unwrap_or_default(),
                worktree: worktree.clone().unwrap_or_default(),
                branch,
            },
            StepKind::Gate { message, options } => Node::Gate {
                id: step.id.clone(),
                message: message.clone(),
                options: options.join(", "),
                branch,
            },
        };
        let id = snarl.insert_node(position(step.at.unwrap_or(auto)), node);
        ids.insert(step.id.as_str(), id);
    }
    for step in &flow.steps {
        for needs in &step.needs {
            if let (Some(from), Some(to)) = (ids.get(needs.as_str()), ids.get(step.id.as_str())) {
                snarl.connect(output(*from), input(*to));
            }
        }
    }
    snarl
}

fn optional(text: &str) -> Option<String> {
    Some(text.trim().to_owned()).filter(|text| !text.is_empty())
}

/// A draft without the cards' places: what a run of the flow does.
pub(crate) fn content_of(mut draft: Draft) -> Draft {
    for step in &mut draft.steps {
        step.at = None;
    }
    draft
}

/// The draft of the graph: what Save checks and writes.
pub(crate) fn draft_of(name: &str, snarl: &Snarl<Node>) -> Draft {
    let steps = snarl.nodes_pos_ids().map(|(node, position, value)| {
        let mut needs: Vec<String> = snarl
            .in_pin(input(node))
            .remotes
            .iter()
            .filter_map(|from| snarl.get_node(from.node))
            .map(|from| from.id().trim().to_owned())
            .collect();
        needs.sort_unstable();
        needs.dedup();
        let branch = value.branch();
        let loop_to = optional(&branch.loop_to);
        let mut step = DraftStep {
            id: value.id().trim().to_owned(),
            needs,
            at: Some(place(position)),
            options: value.options(),
            when: optional(&branch.when),
            max_rounds: loop_to.is_some().then_some(branch.max_rounds),
            loop_to,
            ..DraftStep::default()
        };
        match value {
            Node::Step {
                command,
                prompt,
                cwd,
                worktree,
                ..
            } => {
                step.command = Some(command.trim().to_owned());
                step.prompt = optional(prompt);
                step.cwd = optional(cwd);
                step.worktree = optional(worktree);
            }
            Node::Gate { message, .. } => step.gate = Some(message.trim().to_owned()),
        }
        step
    });
    Draft::new(name.trim(), steps.collect())
}

/// An ID that no node has: `base`, then `base-2`, `base-3`, …
pub(crate) fn free_id(snarl: &Snarl<Node>, base: &str) -> String {
    let taken = |id: &str| snarl.nodes().any(|node| node.id() == id);
    if !taken(base) {
        return base.to_owned();
    }
    (2_u32..10_000)
        .map(|count| format!("{base}-{count}"))
        .find(|id| !taken(id))
        .unwrap_or_else(|| base.to_owned())
}

/// The graph of a new flow: plan, a gate, then build.
pub(crate) fn starter() -> Snarl<Node> {
    let mut snarl = Snarl::new();
    let plan = snarl.insert_node(
        position([0, 0]),
        Node::step("plan", "claude", "Write the plan in PLAN.md."),
    );
    let approve = snarl.insert_node(
        position([COLUMN, 0]),
        Node::gate("approve", "Is the plan correct?"),
    );
    let build = snarl.insert_node(
        position([COLUMN.saturating_mul(2), 0]),
        Node::step("build", "claude", "Do the plan in PLAN.md."),
    );
    snarl.connect(output(plan), input(approve));
    snarl.connect(output(approve), input(build));
    snarl
}
