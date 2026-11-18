//! What a step's own agent (or shell command) tells its run: context,
//! report and ask. Each one is a file in the run folder that the runner
//! reads; nothing here types into a terminal or starts anything. The
//! user's side is `terminator_core::flow::control`.
use super::read_state;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use terminator_core::{
    atomic_write,
    flow::{
        RUN_DIR, RunState, StepStatus,
        control::{QUESTIONS, REPLIES, bounded},
    },
};

/// Folder of step reports in a run folder: one file per step.
const REPORTS: &str = "reports";
const POLL: Duration = Duration::from_millis(500);

/// The running step of one terminal.
pub(super) struct Own {
    pub(super) dir: PathBuf,
    pub(super) step: String,
    pub(super) state: RunState,
}

/// What a step said about itself with `flow report`.
#[derive(Serialize, Deserialize)]
pub(super) struct Report {
    pub(super) done: bool,
    #[serde(default)]
    pub(super) note: Option<String>,
}

fn write(dir: &Path, folder: &str, step: &str, bytes: &[u8]) -> Result<()> {
    std::fs::create_dir_all(dir.join(folder))?;
    Ok(atomic_write(&dir.join(folder).join(step), bytes)?)
}

/// The step that runs in `session` (default: this terminal).
pub(super) fn own_step(data: &Path, session: Option<String>) -> Result<Own> {
    let session = match session {
        Some(session) => session,
        None => std::env::var("TERMINATOR_SESSION_ID")
            .context("This is not a Terminator terminal; give --session")?,
    };
    let runs = std::fs::read_dir(data.join(RUN_DIR))
        .into_iter()
        .flatten()
        .flatten();
    for dir in runs.map(|entry| entry.path()) {
        let Ok(state) = read_state(&dir) else {
            continue;
        };
        let step = state
            .steps
            .iter()
            .find(|(_, run)| {
                run.status == StepStatus::Running && run.session.as_deref() == Some(&session)
            })
            .map(|(id, _)| id.clone());
        if let Some(step) = step.filter(|_| !state.ended) {
            return Ok(Own { dir, step, state });
        }
    }
    bail!("This terminal is not a running step of a flow")
}

/// The run, the step, and the steps it needs with their sessions, so an
/// agent can read their output with `ctl read`.
pub(super) fn context(own: &Own) -> Value {
    let needs = own
        .state
        .steps
        .get(&own.step)
        .map(|run| run.needs.clone())
        .unwrap_or_default();
    let needs: Vec<Value> = needs
        .iter()
        .map(|id| {
            let run = own.state.steps.get(id);
            json!({
                "step": id,
                "status": run.map(|run| run.status),
                "session": run.and_then(|run| run.session.clone()),
                "note": run.and_then(|run| run.note.clone()),
            })
        })
        .collect();
    json!({
        "run": own.dir.file_name().map(|name| name.to_string_lossy()),
        "flow": own.state.flow,
        "step": own.step,
        "needs": needs,
    })
}

/// Records the step's own result; the runner ends the step with it.
pub(super) fn report(own: &Own, done: bool, note: Option<&str>) -> Result<Value> {
    let report = Report {
        done,
        note: note.map(bounded).transpose()?.map(str::to_owned),
    };
    write(&own.dir, REPORTS, &own.step, &serde_json::to_vec(&report)?)?;
    Ok(json!({"step": own.step, "reported": if done { "done" } else { "failed" }}))
}

pub(super) fn read_report(dir: &Path, step: &str) -> Option<Report> {
    serde_json::from_slice(&std::fs::read(dir.join(REPORTS).join(step)).ok()?).ok()
}

/// Asks the user and waits for the answer (`flow answer`, or the GUI).
/// The question shows in the run state; no terminal gets input.
pub(super) fn ask(own: &Own, question: &str, timeout: Duration) -> Result<Value> {
    let question = bounded(question)?;
    let asked = own.dir.join(QUESTIONS).join(&own.step);
    let reply = own.dir.join(REPLIES).join(&own.step);
    // An older reply is not an answer to this question.
    let _ = std::fs::remove_file(&reply);
    write(&own.dir, QUESTIONS, &own.step, question.as_bytes())?;
    let deadline = Instant::now().checked_add(timeout);
    loop {
        if let Ok(answer) = std::fs::read_to_string(&reply) {
            let _ = std::fs::remove_file(&asked);
            let _ = std::fs::remove_file(&reply);
            return Ok(json!({"step": own.step, "answer": answer}));
        }
        if deadline.is_some_and(|at| Instant::now() >= at) {
            let _ = std::fs::remove_file(&asked);
            bail!("No answer in {} seconds", timeout.as_secs());
        }
        std::thread::sleep(POLL);
    }
}
