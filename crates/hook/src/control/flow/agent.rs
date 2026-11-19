//! What a step's own agent (or shell command) tells its run: context,
//! report and ask. Each one is a file in the run folder that the runner
//! reads; nothing here types into a terminal or starts anything. The
//! user's side is `terminator_core::flow::control`.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use terminator_core::flow::{
    RunState, StepStatus,
    control::{self, QUESTIONS, REPLIES, RunStatus, bounded, write},
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

/// The step that runs in `session` (default: this terminal), in the
/// newest run that has one.
pub(super) fn own_step(data: &Path, session: Option<String>) -> Result<Own> {
    let session = match session {
        Some(session) => session,
        None => std::env::var("TERMINATOR_SESSION_ID")
            .context("This is not a Terminator terminal; give --session")?,
    };
    // Limit 0: only the runs that did not end.
    for (dir, state) in control::recent_runs(data, 0, &mut control::EndedRuns::new()) {
        // An ended run, or one whose runner is gone, reads no report and
        // no answer.
        let view = control::run_view(&dir, state);
        if view.status != RunStatus::Running {
            continue;
        }
        let step = view.state.steps.iter().find(|(_, run)| {
            run.status == StepStatus::Running && run.session.as_deref() == Some(&session)
        });
        if let Some((step, _)) = step {
            let step = step.clone();
            return Ok(Own {
                dir,
                step,
                state: view.state,
            });
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
        "run": control::run_name(&own.dir),
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
        // Nobody can answer a run whose runner is gone.
        if !control::alive(&own.dir) {
            let _ = std::fs::remove_file(&asked);
            bail!("The flow runner of this run is gone; no answer can come");
        }
        std::thread::sleep(POLL);
    }
}
