//! Run Markdown report (`report.md`) and append-only JSONL run event stream
//! (`events.jsonl`) for scripts (`flow wait` / `flow events`).
use super::{
    RunState, StepRun,
    control::{RunError, RunStatus, run_name},
};
use serde::{Deserialize, Serialize};
use std::{
    fmt::Write as _,
    fs::OpenOptions,
    io::Write as _,
    path::{Path, PathBuf},
};

/// File name of the exported Markdown summary inside an ended run folder.
pub const REPORT_FILE: &str = "report.md";
/// File name of the append-only JSONL event stream inside a run folder.
pub const EVENTS_FILE: &str = "events.jsonl";

/// One structured lifecycle event recorded in `RUN/events.jsonl`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunEvent {
    pub at: u64,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Append one event line to `RUN/events.jsonl` (best-effort; never blocks a run).
pub fn append_event(dir: &Path, kind: &str, step: Option<&str>, detail: Option<&str>) {
    let event = RunEvent {
        at: crate::now(),
        kind: kind.to_owned(),
        step: step.map(str::to_owned),
        detail: detail.map(str::to_owned),
    };
    let Ok(mut line) = serde_json::to_string(&event) else {
        return;
    };
    line.push('\n');
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(EVENTS_FILE))
    {
        let _ = file.write_all(line.as_bytes());
    }
}

/// Read all recorded events from `RUN/events.jsonl`.
pub fn read_events(dir: &Path) -> Vec<RunEvent> {
    let Ok(text) = std::fs::read_to_string(dir.join(EVENTS_FILE)) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// Format a Markdown summary of a run: status, inputs, steps, notes,
/// outputs, attempts, choices, racers, and durations.
pub fn run_report_markdown(dir: &Path, state: &RunState, status: RunStatus) -> String {
    let id = run_name(dir);
    let mut out = format!("# Flow run: {} (`{id}`)\n\n", state.flow);
    let _ = writeln!(
        out,
        "- **Flow file:** `.terminator/flows/{}.toml`",
        state.file
    );
    let _ = writeln!(out, "- **Status:** {}", status.label());
    if !state.inputs.is_empty() {
        out.push_str("\n## Inputs\n\n");
        for (name, value) in &state.inputs {
            let _ = writeln!(out, "- `{name}` = `{value}`");
        }
    }
    out.push_str("\n## Steps\n\n");
    for step_id in &state.order {
        let Some(step) = state.steps.get(step_id) else {
            continue;
        };
        write_step_section(&mut out, step_id, step);
    }
    out
}

fn write_step_section(out: &mut String, id: &str, step: &StepRun) {
    let _ = writeln!(out, "### `{id}` — {:?}\n", step.status);
    let mut meta = vec![format!("attempt {}", step.attempt())];
    if let Some(round) = step.round_label() {
        meta.push(round);
    }
    if let Some(choice) = &step.choice {
        meta.push(format!("choice: {choice}"));
    }
    if let Some(port) = step.port {
        meta.push(format!("port: {port}"));
    }
    if let (Some(start), Some(end)) = (step.started, step.finished) {
        let secs = end.saturating_sub(start);
        meta.push(format!("duration: {secs}s"));
    }
    let _ = writeln!(out, "- {}", meta.join(" · "));
    if let Some(note) = &step.note {
        let _ = writeln!(out, "- **Note:** {note}");
    }
    if !step.outputs.is_empty() {
        let _ = writeln!(out, "- **Outputs:** {}", step.outputs.join(", "));
    }
    if !step.racers.is_empty() {
        out.push_str("- **Racers:**\n");
        for r in &step.racers {
            let agent = r.agent.as_deref().unwrap_or("default");
            let diff = match (r.added, r.deleted) {
                (Some(a), Some(d)) => format!(" (+{a} −{d})"),
                _ => String::new(),
            };
            let note = r
                .note
                .as_deref()
                .map(|n| format!(" — {n}"))
                .unwrap_or_default();
            let _ = writeln!(
                out,
                "  - `#{} ({agent})` `{}`: {:?}{diff}{note}",
                r.index, r.worktree, r.status
            );
        }
    }
    out.push('\n');
}

/// Write `RUN/report.md` for an ended run on explicit user action.
pub fn write_run_report(dir: &Path) -> Result<PathBuf, RunError> {
    let view = super::control::read_run(dir)?;
    if !view.status.ended() {
        return Err(RunError::NotEnded);
    }
    let text = run_report_markdown(dir, &view.state, view.status);
    let path = dir.join(REPORT_FILE);
    crate::atomic_write(&path, text.as_bytes())?;
    Ok(path)
}
