//! `flow wait RUN --until STATE` and `flow events RUN [--follow]` for scripts.
use super::{
    agent,
    runner::{End, Runner},
};
use crate::output::Output;
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use terminator_core::{
    Request,
    flow::{
        StepStatus,
        control::{self, RunStatus},
        report,
    },
    rpc,
};

const POLL: Duration = Duration::from_millis(500);

/// Wait until run `dir` reaches `until` (`ended`, `done`, `waiting`, `stopped`).
pub(super) fn wait_run(dir: &Path, until: &str, timeout: Duration) -> Result<Value> {
    let deadline = Instant::now().checked_add(timeout);
    loop {
        let view = control::read_run(dir)?;
        let waits_for_user = view.status == RunStatus::Running
            && view.state.steps.values().any(|s| {
                s.status == StepStatus::Running && (s.gate.is_some() || s.question.is_some())
            });
        let reached = match until {
            "ended" => view.status.ended(),
            "done" => view.status == RunStatus::Done || view.status.ended(),
            "waiting" => waits_for_user || view.status.ended(),
            "stopped" => {
                matches!(view.status, RunStatus::Stopped | RunStatus::RunnerGone)
                    || view.status == RunStatus::Done
            }
            _ => bail!("Unknown wait state \"{until}\""),
        };
        if reached {
            let value = json!({
                "run": control::run_name(dir),
                "status": view.status.label(),
                "waiting_for_user": waits_for_user,
                "ended": view.status.ended(),
            });
            if until == "done" && view.status != RunStatus::Done {
                bail!(
                    "Run {} ended as {} (expected done)",
                    control::run_name(dir),
                    view.status.label()
                );
            }
            return Ok(value);
        }
        if deadline.is_some_and(|end| Instant::now() >= end) {
            bail!(
                "Timed out waiting for run {} to reach {until}",
                control::run_name(dir)
            );
        }
        std::thread::sleep(POLL);
    }
}

/// Stream recorded events from `RUN/events.jsonl` as JSON lines, optionally
/// following until the run ends.
pub(super) fn stream_events(dir: &Path, follow: bool) -> Result<()> {
    let mut emitted = 0usize;
    loop {
        let events = report::read_events(dir);
        for event in events.iter().skip(emitted) {
            Output::line(serde_json::to_string(event)?);
        }
        emitted = events.len();
        if !follow {
            return Ok(());
        }
        let view = control::read_run(dir)?;
        if view.status.ended() {
            // Drain any final events written as the run ended.
            let final_events = report::read_events(dir);
            for event in final_events.iter().skip(emitted) {
                Output::line(serde_json::to_string(event)?);
            }
            return Ok(());
        }
        std::thread::sleep(POLL);
    }
}

impl Runner {
    /// Ingest any dynamic steps proposed by running agents (`flow propose`).
    pub(super) fn ingest_proposals(&mut self) {
        for proposal in control::take_proposals(&self.dir) {
            let by = proposal.proposed_by.clone();
            match self.flow.add_proposed(&mut self.state, proposal) {
                Ok(id) => {
                    self.changed = true;
                    report::append_event(&self.dir, "step_proposed", Some(&id), Some(&by));
                    Output::line_or_drop(format_args!("{id}: proposed by {by}"));
                }
                Err(error) => {
                    Output::line_or_drop(format_args!("proposal ignored: {error}"));
                }
            }
        }
    }

    /// The end that the step's own report (`flow report`) gives, if any.
    pub(super) fn report_end(&mut self, id: &str) -> End {
        let report = agent::read_report(&self.dir, id)?;
        Some(if report.done {
            if let Some(run) = self.state.steps.get_mut(id) {
                run.note = report.note;
                run.outputs = report.outputs;
            }
            Ok(())
        } else {
            Err(report
                .note
                .unwrap_or_else(|| "The step reported a failure".to_owned()))
        })
    }

    /// Types one line, then presses Enter in a second write (as `ctl send
    /// --enter` does).
    pub(super) fn type_line(&self, session: &str, line: &str) -> Result<()> {
        super::super::lookup::Ctl::connect(&self.paths)?
            .type_and_enter(session, line.as_bytes().to_vec())
    }

    pub(super) fn session_of(&self, id: &str) -> String {
        self.state
            .steps
            .get(id)
            .and_then(|run| run.session.clone())
            .unwrap_or_default()
    }

    /// A terminal notice is a help only; an older daemon has none.
    pub(super) fn notify(&self, session: String, what: &str, body: &str) {
        let _ = rpc(
            &self.paths,
            Request::TerminalNotify {
                session,
                title: format!("{what}: {}", self.flow.name),
                body: body.to_owned(),
            },
        );
    }

    /// Copies each running step's open question (`flow ask`) and mailbox
    /// count into the run state, and raises a notice for a new question.
    pub(super) fn sync_questions(&mut self) {
        let mut asked = Vec::new();
        for (id, run) in &mut self.state.steps {
            let inbox = u32::try_from(control::read_inbox(&self.dir, id).len()).unwrap_or(u32::MAX);
            if inbox != run.inbox {
                run.inbox = inbox;
                self.changed = true;
            }
            let question = (run.status == StepStatus::Running && run.gate.is_none())
                .then(|| control::question(&self.dir, id))
                .flatten();
            if question == run.question {
                continue;
            }
            if let (Some(question), Some(session)) = (&question, &run.session) {
                asked.push((id.clone(), session.clone(), question.clone()));
            }
            run.question = question;
            self.changed = true;
        }
        let run = control::run_name(&self.dir);
        for (id, session, question) in asked {
            Output::line_or_drop(format_args!(
                "{id}: question: {question}\n  terminator-hook flow answer {run} {id} \"...\""
            ));
            self.notify(session, "Flow question", &question);
        }
    }
}

/// The state of a run with its status: a run whose runner is gone shows
/// as "stopped (runner gone)", with its open steps cancelled.
pub(super) fn status(dir: &Path) -> Result<Value> {
    let view = control::read_run(dir)?;
    let mut value = serde_json::to_value(&view.state)?;
    if let Some(fields) = value.as_object_mut() {
        fields.insert("status".to_owned(), view.status.label().into());
        let held = view.status == RunStatus::Running && view.state.held();
        fields.insert("waits_for_retry".to_owned(), held.into());
    }
    Ok(value)
}

pub(super) fn execute_approve(
    dir: &Path,
    step: &str,
    (option, pick): (Option<String>, Option<u32>),
) -> Result<Value> {
    let answer = match (option.as_deref(), pick) {
        (Some(option), _) => control::GateAnswer::Choose(option),
        (None, Some(index)) => control::GateAnswer::Pick(index),
        (None, None) => control::GateAnswer::Approve,
    };
    control::answer_gate(dir, step, answer)?;
    Ok(match (option, pick) {
        (Some(option), _) => {
            json!({"step": step, "answer": control::APPROVE, "option": option})
        }
        (None, Some(index)) => {
            json!({"step": step, "answer": control::APPROVE, "pick": index})
        }
        (None, None) => json!({"step": step, "answer": control::APPROVE}),
    })
}

/// Each flow file with its name, or the reason it is refused.
pub(super) fn list(project: &terminator_core::Project) -> Value {
    let names = terminator_core::flow::files::names(&project.path);
    let rows = names.into_iter().map(|file| {
        match terminator_core::flow::files::load(&project.path, &file) {
            Ok(flow) => json!({"file": file, "flow": flow.name, "steps": flow.steps.len()}),
            Err(error) => json!({"file": file, "error": format!("{error:#}")}),
        }
    });
    Value::Array(rows.collect())
}
