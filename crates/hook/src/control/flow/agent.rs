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
pub(super) const REPORTS: &str = "reports";
const POLL: Duration = Duration::from_millis(500);

/// The running step (and optional 1-based racer index) of one terminal.
pub(super) struct Own {
    pub(super) dir: PathBuf,
    pub(super) step: String,
    pub(super) racer: Option<u32>,
    pub(super) state: RunState,
}

/// What a step said about itself with `flow report`.
#[derive(Serialize, Deserialize)]
pub(super) struct Report {
    pub(super) done: bool,
    #[serde(default)]
    pub(super) note: Option<String>,
    #[serde(default)]
    pub(super) outputs: Vec<String>,
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
        for (step, run) in &view.state.steps {
            if run.status != StepStatus::Running {
                continue;
            }
            if run.session.as_deref() == Some(&session) {
                let step = step.clone();
                return Ok(Own {
                    dir,
                    step,
                    racer: None,
                    state: view.state,
                });
            }
            if let Some(racer) = run
                .racers
                .iter()
                .find(|r| r.status == StepStatus::Running && r.session.as_deref() == Some(&session))
            {
                let step = step.clone();
                let index = racer.index;
                return Ok(Own {
                    dir,
                    step,
                    racer: Some(index),
                    state: view.state,
                });
            }
        }
    }
    bail!("This terminal is not a running step of a flow")
}

/// The run, the step, and the steps it needs with their sessions, so an
/// agent can read their output with `ctl read`.
pub(super) fn context(own: &Own) -> Value {
    let current = own.state.steps.get(&own.step);
    let needs = current.map(|run| run.needs.clone()).unwrap_or_default();
    let needs: Vec<Value> = needs
        .iter()
        .map(|id| {
            let run = own.state.steps.get(id);
            json!({
                "step": id,
                "status": run.map(|run| run.status),
                "session": run.and_then(|run| run.session.clone()),
                "note": run.and_then(|run| run.note.clone()),
                "outputs": run.map(|run| run.outputs.clone()).unwrap_or_default(),
                "choice": run.and_then(|run| run.choice.clone()),
            })
        })
        .collect();
    json!({
        "run": control::run_name(&own.dir),
        "flow": own.state.flow,
        "step": own.step,
        "racer": own.racer,
        "rules": own.state.rules,
        "attempt": current.map_or(1, terminator_core::flow::StepRun::attempt),
        "round": current.map_or(1, terminator_core::flow::StepRun::round),
        "max_rounds": current.map_or(0, |run| run.max_rounds),
        "loop_note": current.and_then(|run| run.loop_note.clone()),
        "inbox_count": current.map_or(0, |run| run.inbox),
        "port": current.and_then(|run| run.port),
        "inputs": own.state.inputs,
        "needs": needs,
    })
}

/// Reads the messages stored in this step's mailbox (`flow send`).
pub(super) fn inbox(own: &Own) -> Value {
    let messages = control::read_inbox(&own.dir, &own.step);
    json!({
        "run": control::run_name(&own.dir),
        "step": own.step,
        "messages": messages,
    })
}

fn report_key(step: &str, racer: Option<u32>) -> String {
    match racer {
        Some(index) => format!("{step}--racer-{index}"),
        None => step.to_owned(),
    }
}

/// Records the step's own result; the runner ends the step with it.
pub(super) fn report(
    own: &Own,
    done: bool,
    note: Option<&str>,
    outputs: &[String],
) -> Result<Value> {
    if outputs.len() > control::MAX_OUTPUTS {
        bail!("At most {} output paths are allowed", control::MAX_OUTPUTS);
    }
    let outputs: Vec<String> = outputs
        .iter()
        .map(|path| control::output_path_ok(path))
        .collect::<std::result::Result<_, _>>()?;
    let report = Report {
        done,
        note: note.map(bounded).transpose()?.map(str::to_owned),
        outputs: outputs.clone(),
    };
    let key = report_key(&own.step, own.racer);
    write(&own.dir, REPORTS, &key, &serde_json::to_vec(&report)?)?;
    Ok(json!({
        "step": own.step,
        "reported": if done { "done" } else { "failed" },
        "outputs": outputs,
    }))
}

pub(super) fn read_report(dir: &Path, step: &str) -> Option<Report> {
    serde_json::from_slice(&std::fs::read(dir.join(REPORTS).join(step)).ok()?).ok()
}

pub(super) fn read_racer_report(dir: &Path, step: &str, racer: u32) -> Option<Report> {
    let key = report_key(step, Some(racer));
    serde_json::from_slice(&std::fs::read(dir.join(REPORTS).join(key)).ok()?).ok()
}

/// Parameters of `flow propose`.
pub(super) struct ProposeInput {
    pub(super) id: String,
    pub(super) command: String,
    pub(super) prompt: Option<String>,
    pub(super) cwd: Option<String>,
    pub(super) worktree: Option<String>,
    pub(super) needs: Vec<String>,
}

/// Proposes a new command step to be added to this running flow.
pub(super) fn propose(own: &Own, input: ProposeInput) -> Result<Value> {
    let needs = if input.needs.is_empty() {
        vec![own.step.clone()]
    } else {
        input.needs
    };
    let proposal = terminator_core::flow::ProposedStep {
        id: input.id.clone(),
        command: input.command,
        prompt: input.prompt,
        cwd: input.cwd,
        worktree: input.worktree,
        needs,
        proposed_by: own.step.clone(),
    };
    control::write_proposal(&own.dir, &proposal)?;
    Ok(json!({
        "run": control::run_name(&own.dir),
        "proposed": input.id,
        "by": own.step,
    }))
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
