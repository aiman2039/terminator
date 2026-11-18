//! The flow editor's graph: one node per step, one wire per `needs`. It
//! converts to and from the flow file's draft form; the checks are the
//! file format's own (`terminator_core::flow::Draft::check`).
use egui_snarl::{InPinId, NodeId, OutPinId, Snarl};
use std::collections::HashMap;
use terminator_core::flow::{Draft, DraftStep, Flow, StepKind};

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
    },
    /// A gate: the run waits for the user's approval of `message`.
    Gate { id: String, message: String },
}

impl Node {
    pub(crate) fn step(id: &str, command: &str, prompt: &str) -> Self {
        Self::Step {
            id: id.to_owned(),
            command: command.to_owned(),
            prompt: prompt.to_owned(),
            cwd: String::new(),
            worktree: String::new(),
        }
    }

    pub(crate) fn gate(id: &str, message: &str) -> Self {
        Self::Gate {
            id: id.to_owned(),
            message: message.to_owned(),
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

/// Distance between the columns and the rows of an automatic layout.
const COLUMN: i32 = 380;
const ROW: i32 = 260;

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
    let mut depth = vec![0_i32; flow.steps.len()];
    // A checked flow has no cycle, so one pass per step is sufficient.
    for _ in 0..flow.steps.len() {
        for (at, step) in flow.steps.iter().enumerate() {
            let after = step
                .needs
                .iter()
                .filter_map(|needs| index.get(needs.as_str()))
                .filter_map(|needs| depth.get(*needs))
                .map(|depth| depth.saturating_add(1))
                .max()
                .unwrap_or(0);
            if let Some(slot) = depth.get_mut(at) {
                *slot = after;
            }
        }
    }
    let mut rows: HashMap<i32, i32> = HashMap::new();
    depth
        .into_iter()
        .map(|column| {
            let row = rows.entry(column).or_insert(0);
            let place = [column.saturating_mul(COLUMN), row.saturating_mul(ROW)];
            *row = row.saturating_add(1);
            place
        })
        .collect()
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

/// The graph of a checked flow.
pub(crate) fn graph_of(flow: &Flow) -> Snarl<Node> {
    let mut snarl = Snarl::new();
    let mut ids = HashMap::new();
    for (step, auto) in flow.steps.iter().zip(auto_places(flow)) {
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
            },
            StepKind::Gate { message } => Node::gate(&step.id, message),
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
        let mut step = DraftStep {
            id: value.id().trim().to_owned(),
            needs,
            at: Some(place(position)),
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

/// The steps in the order that they can run: a step comes after the
/// steps that it needs. Steps of a cycle come last, in file order.
fn run_order(draft: &Draft) -> Vec<&DraftStep> {
    let mut done: Vec<&DraftStep> = Vec::new();
    let mut left: Vec<&DraftStep> = draft.steps.iter().collect();
    while !left.is_empty() {
        let ready = |step: &DraftStep| {
            step.needs
                .iter()
                .all(|needs| done.iter().any(|done| done.id == *needs))
        };
        let (now, later): (Vec<_>, Vec<_>) = left.iter().copied().partition(|step| ready(step));
        if now.is_empty() {
            done.extend(later);
            break;
        }
        done.extend(now);
        left = later;
    }
    done
}

/// The flow as a plan in Markdown: for the preview and the export.
pub(crate) fn plan_text(draft: &Draft) -> String {
    use std::fmt::Write as _;
    let mut text = format!("# {}\n", draft.name);
    let rules = [
        ("Before each prompt", &draft.prompt_prefix),
        ("After each prompt", &draft.prompt_suffix),
    ];
    for (label, rule) in rules {
        if let Some(rule) = rule {
            let _ = write!(text, "\n**{label}:**\n\n{rule}\n");
        }
    }
    for (index, step) in run_order(draft).into_iter().enumerate() {
        let kind = if step.gate.is_some() { "gate" } else { "step" };
        let number = index.saturating_add(1);
        let _ = write!(text, "\n## {number}. {} ({kind})\n\n", step.id);
        if !step.needs.is_empty() {
            let _ = writeln!(text, "- After: {}", step.needs.join(", "));
        }
        let lines = [
            (
                "Command",
                step.command.as_ref().map(|line| format!("`{line}`")),
            ),
            ("Folder", step.cwd.clone()),
            ("Worktree", step.worktree.clone()),
            ("You approve", step.gate.clone()),
        ];
        for (label, value) in lines {
            if let Some(value) = value {
                let _ = writeln!(text, "- {label}: {value}");
            }
        }
        if let Some(prompt) = &step.prompt {
            let _ = write!(text, "\n{prompt}\n");
        }
    }
    text
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
