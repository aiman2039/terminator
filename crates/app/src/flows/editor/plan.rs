//! The flow as a plan in Markdown, for the preview and the export: the
//! order as a Mermaid flowchart (a loop as a dotted arrow back), the
//! inputs, the rules, and each step with its condition and loop.
use terminator_core::flow::{Draft, DraftStep};

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

/// The order of the steps as a Mermaid flowchart: a box is a step, a
/// diamond is a gate, an arrow is "then" (with the option of a `when`),
/// and a dotted arrow is a loop back.
fn mermaid(steps: &[&DraftStep]) -> String {
    use std::fmt::Write as _;
    let node = |id: &str| steps.iter().position(|step| step.id == id);
    let mut text = String::from("flowchart LR\n");
    for (index, step) in steps.iter().enumerate() {
        let label = step.id.replace('"', "'");
        let shape = if step.gate.is_some() {
            format!("{{\"{label}\"}}")
        } else {
            format!("[\"{label}\"]")
        };
        let _ = writeln!(text, "    n{index}{shape}");
    }
    for (index, step) in steps.iter().enumerate() {
        let option = step.when.as_deref().and_then(|when| when.split_once('.'));
        for needs in &step.needs {
            let Some(from) = node(needs) else {
                continue;
            };
            match option {
                Some((gate, option)) if gate == needs => {
                    let _ = writeln!(text, "    n{from} -->|{option}| n{index}");
                }
                _ => {
                    let _ = writeln!(text, "    n{from} --> n{index}");
                }
            }
        }
        if let (Some(to), Some(rounds)) = (step.loop_to.as_deref().and_then(node), step.max_rounds)
        {
            let _ = writeln!(text, "    n{index} -.->|loop, {rounds} rounds| n{to}");
        }
    }
    text
}

/// The labeled lines of one step in the plan.
fn step_lines(step: &DraftStep) -> [(&'static str, Option<String>); 9] {
    let loop_back = step.loop_to.as_ref().map(|to| {
        let rounds = step.max_rounds.unwrap_or_default();
        format!("back to {to} when done, {rounds} rounds at most")
    });
    let racers = if let Some(count) = step.count {
        Some(format!("{count} racers"))
    } else if !step.agents.is_empty() {
        Some(step.agents.join(", "))
    } else {
        None
    };
    [
        (
            "Command",
            step.command.as_ref().map(|line| format!("`{line}`")),
        ),
        ("Folder", step.cwd.clone()),
        ("Worktree", step.worktree.clone()),
        ("Racers", racers),
        ("You approve", step.gate.clone()),
        (
            "Options",
            (!step.options.is_empty()).then(|| step.options.join(", ")),
        ),
        ("Pick winner of", step.pick.clone()),
        ("Runs when", step.when.clone()),
        ("Loop", loop_back),
    ]
}

/// The flow as a plan in Markdown, for the preview and the export: the
/// flow file's place, the order as a Mermaid flowchart, the rules, and
/// each step in the order that it can run.
pub(crate) fn plan_text(draft: &Draft, file: &str) -> String {
    use std::fmt::Write as _;
    let steps = run_order(draft);
    let mut text = format!("# {}\n", draft.name);
    let _ = write!(
        text,
        "\nFlow file: `.terminator/flows/{file}.toml` (the source; this plan is a copy).\n\
         \n```mermaid\n{}```\n",
        mermaid(&steps)
    );
    if !draft.inputs.is_empty() {
        text.push_str("\n## Inputs\n\n");
        for (name, input) in &draft.inputs {
            let about = input.description.as_deref().unwrap_or("No description");
            let value = match &input.default {
                Some(value) => format!("default `{value}`"),
                None => "required".to_owned(),
            };
            let _ = writeln!(text, "- `{name}`: {about} ({value})");
        }
    }
    if let Some(rules) = &draft.rules {
        let _ = write!(text, "\n## Rules\n\n{rules}\n");
    }
    for (index, step) in steps.into_iter().enumerate() {
        let kind = if step.gate.is_some() { "gate" } else { "step" };
        let number = index.saturating_add(1);
        let _ = write!(text, "\n## {number}. {} ({kind})\n\n", step.id);
        if !step.needs.is_empty() {
            let _ = writeln!(text, "- After: {}", step.needs.join(", "));
        }
        for (label, value) in step_lines(step) {
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
