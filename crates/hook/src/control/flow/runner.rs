//! The run loop of `terminator-hook flow run`. It starts each step when
//! the steps it needs are done, and it records the run in the state file.
//! It never closes or restarts a session.
use super::{
    super::{
        agents::latest_mark,
        lookup::{Ctl, NewSession},
    },
    ANSWERS, APPROVE, CANCEL, REJECT, agent,
    progress::{Phase, Progress, Wait, progress, with_results},
};
use crate::output::Output;
use anyhow::{Result, bail};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use terminator_core::{
    flow::{Flow, Outcome, RunState, STATE_FILE, Step, StepKind, StepStatus, control},
    *,
};

const POLL: Duration = Duration::from_millis(500);
/// Longest wait for the agent's first hook report.
const START_LIMIT: Duration = Duration::from_mins(2);
/// Seconds between the agent's first hook report and the prompt, so the
/// agent's input line is there.
const SETTLE_SECS: u64 = 2;
/// Shown in the Flows views while the prompt waits for the agent to start.
const WAITS_FOR_HOOK: &str = "Waits for the agent's first hook report (2 minutes at most). \
     The agent's hooks must be installed for this Terminator.";

/// How a running step ended, or `None` while it runs.
type End = Option<Result<(), String>>;

pub(super) struct Runner {
    paths: Paths,
    project: Project,
    flow: Flow,
    /// The run folder.
    dir: PathBuf,
    state: RunState,
    /// Running steps, with the time the phase started.
    phases: BTreeMap<String, (Phase, Instant)>,
    /// The last state file text.
    saved: Vec<u8>,
    /// The state changed after the last save.
    changed: bool,
    /// Ctrl-C, SIGTERM or SIGHUP (its terminal closed): cancel the run.
    interrupted: Arc<AtomicBool>,
}

impl Runner {
    /// The runner of flow file `file` (`flow` is its content) in the new
    /// run folder `dir`.
    pub(super) fn new(
        paths: Paths,
        project: Project,
        (flow, file): (Flow, &str),
        dir: PathBuf,
    ) -> Result<Self> {
        let interrupted = Arc::new(AtomicBool::new(false));
        for signal in [
            signal_hook::consts::SIGINT,
            signal_hook::consts::SIGTERM,
            signal_hook::consts::SIGHUP,
        ] {
            signal_hook::flag::register(signal, Arc::clone(&interrupted))?;
        }
        Ok(Self {
            state: RunState::new(&flow, file, &project.id),
            paths,
            project,
            flow,
            dir,
            phases: BTreeMap::new(),
            saved: Vec::new(),
            changed: true,
            interrupted,
        })
    }

    /// Runs until every step is done, a step stops the run, or the user
    /// cancels. A cancel starts no new step and leaves sessions alone.
    /// The run ends (`ended`) also when the loop fails; a runner that dies
    /// without a write lets its lock go, so readers see it gone.
    pub(super) fn run(mut self) -> Result<Outcome> {
        std::fs::create_dir_all(self.dir.join(ANSWERS))?;
        // Before the first state file write: a reader never sees this run
        // without its runner's lock.
        let _lock = control::hold_runner(&self.dir)?;
        let looped = self.run_loop();
        if let Err(error) = &looped {
            self.state
                .abort(&format!("The flow runner stopped: {error:#}"));
        }
        self.state.ended = true;
        self.changed = true;
        let saved = self.save();
        looped?;
        saved?;
        Ok(self.flow.outcome(&self.state))
    }

    fn run_loop(&mut self) -> Result<()> {
        self.save()?;
        let mut ready = self.ready();
        loop {
            if self.interrupted.load(Ordering::Relaxed) || self.dir.join(CANCEL).exists() {
                self.state.cancel();
                self.changed = true;
                break;
            }
            for step in &ready {
                self.start(step);
            }
            self.poll_running();
            self.sync_questions();
            self.save()?;
            // The run is active while a step runs or can start.
            ready = self.ready();
            let running = self
                .state
                .steps
                .values()
                .any(|run| run.status == StepStatus::Running);
            if ready.is_empty() && !running {
                break;
            }
            std::thread::sleep(POLL);
        }
        Ok(())
    }

    fn ready(&self) -> Vec<Step> {
        self.flow.ready(&self.state).into_iter().cloned().collect()
    }

    /// Writes the state file when the state changed.
    fn save(&mut self) -> Result<()> {
        if !self.changed {
            return Ok(());
        }
        let text = serde_json::to_vec_pretty(&self.state)?;
        if text != self.saved {
            atomic_write(&self.dir.join(STATE_FILE), &text)?;
            self.saved = text;
        }
        self.changed = false;
        Ok(())
    }

    fn start(&mut self, step: &Step) {
        let started = match &step.kind {
            StepKind::Gate { message } => {
                self.announce_gate(step, message);
                Ok(Phase::Gate)
            }
            StepKind::Command { .. } => self.start_command(step),
        };
        match started {
            Ok(phase) => {
                self.state.set(&step.id, StepStatus::Running);
                self.changed = true;
                self.phases.insert(step.id.clone(), (phase, Instant::now()));
                Output::line_or_drop(format_args!("{}: started", step.id));
            }
            Err(error) => self.finish(&step.id, Some(Err(format!("{error:#}")))),
        }
    }

    /// A new terminal with the command typed and Enter pressed: the Run
    /// of the flow is the user's consent for that Enter.
    fn start_command(&mut self, step: &Step) -> Result<Phase> {
        let StepKind::Command {
            command,
            prompt,
            cwd,
            worktree,
        } = &step.kind
        else {
            bail!("Not a command step");
        };
        let ctl = Ctl::connect(&self.paths)?;
        let mut project = self.project.clone();
        if let Some(name) = worktree {
            project = ctl.step_worktree(&project, name)?;
        }
        let session = ctl.create_session(NewSession {
            project: &project,
            cwd: cwd.as_ref().map(|cwd| project.path.join(cwd)),
            file: None,
        })?;
        // The step's own commands (`flow context`, `flow report`, `flow
        // ask`) find their run by this session. The state file must have
        // it before the command can run.
        if let Some(run) = self.state.steps.get_mut(&step.id) {
            run.session = Some(session.id.clone());
        }
        self.state.set(&step.id, StepStatus::Running);
        self.changed = true;
        self.save()?;
        self.type_line(&session.id, command)?;
        // Without a GUI the session runs in the background.
        let _ = ui_control::rpc(
            &self.paths,
            ui_control::Request::ShowSession {
                session: session.id.clone(),
                anchor: None,
                split: None,
            },
        );
        let Some(prompt) = prompt else {
            return Ok(Phase::Command(Wait::Working { since: None }));
        };
        // Handoff: the result notes of the steps this one needs.
        let results = step.needs.iter().filter_map(|needs| {
            let note = self.state.steps.get(needs)?.note.as_deref()?;
            Some((needs.as_str(), note))
        });
        let prompt = with_results(&self.flow.full_prompt(prompt), results);
        self.set_waits(&step.id, Some(WAITS_FOR_HOOK));
        Ok(Phase::Command(Wait::Starting { prompt }))
    }

    /// What the step waits for, in the state file: the Flows views show it.
    fn set_waits(&mut self, id: &str, text: Option<&str>) {
        if let Some(run) = self.state.steps.get_mut(id) {
            run.waits_for = text.map(str::to_owned);
            self.changed = true;
        }
        // A failed write only loses this display text.
        let _ = self.save();
    }

    /// Types one line, then presses Enter in a second write (as `ctl send
    /// --enter` does).
    fn type_line(&self, session: &str, line: &str) -> Result<()> {
        // The session can be newer than an older snapshot.
        Ctl::connect(&self.paths)?.type_and_enter(session, line.as_bytes().to_vec())
    }

    fn announce_gate(&self, step: &Step, message: &str) {
        let run = control::run_name(&self.dir);
        Output::line_or_drop(format_args!(
            "{id}: gate: {message}\n  terminator-hook flow approve {run} {id}\n  terminator-hook flow reject {run} {id}",
            id = step.id
        ));
        let session = step
            .needs
            .iter()
            .find_map(|needs| self.state.steps.get(needs)?.session.clone());
        if let Some(session) = session {
            self.notify(session, "Flow gate", message);
        }
    }

    /// A terminal notice is a help only; an older daemon has none.
    fn notify(&self, session: String, what: &str, body: &str) {
        let _ = rpc(
            &self.paths,
            Request::TerminalNotify {
                session,
                title: format!("{what}: {}", self.flow.name),
                body: body.to_owned(),
            },
        );
    }

    /// Copies each running step's open question (`flow ask`) into the run
    /// state, and raises a notice for a new one.
    fn sync_questions(&mut self) {
        let mut asked = Vec::new();
        for (id, run) in &mut self.state.steps {
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

    /// Polls each running step. Only a command step needs the daemon's
    /// snapshot; a gate needs only its answer file.
    fn poll_running(&mut self) {
        let commands = self
            .phases
            .values()
            .any(|(phase, _)| matches!(phase, Phase::Command(_)));
        // A daemon that does not answer now can answer later.
        let snapshot = commands.then(|| Ctl::snapshot(&self.paths).ok()).flatten();
        let running: Vec<String> = self.phases.keys().cloned().collect();
        for id in running {
            let end = self.poll(&id, snapshot.as_ref());
            self.finish(&id, end);
        }
    }

    fn poll(&mut self, id: &str, snapshot: Option<&State>) -> End {
        if matches!(self.phases.get(id)?.0, Phase::Gate) {
            return self.gate_answer(id);
        }
        let snapshot = snapshot?;
        // The step's own report (`flow report`) is explicit, so it comes
        // before the hook state and before an ended terminal.
        if let Some(report) = agent::read_report(&self.dir, id) {
            return Some(if report.done {
                // The note of a done step is its result for later steps.
                if let Some(run) = self.state.steps.get_mut(id) {
                    run.note = report.note;
                }
                Ok(())
            } else {
                Err(report
                    .note
                    .unwrap_or_else(|| "The step reported a failure".to_owned()))
            });
        }
        self.poll_command(id, snapshot)
    }

    /// One poll of a command step by its hook state. Sends the prompt when
    /// the agent's first hook report is there.
    fn poll_command(&mut self, id: &str, snapshot: &State) -> End {
        let session = self.session_of(id);
        let (Phase::Command(wait), started) = self.phases.get(id)? else {
            return None;
        };
        match progress(snapshot, &session, wait) {
            Progress::Done => Some(Ok(())),
            Progress::Failed(note) => Some(Err(note.to_owned())),
            Progress::SendPrompt => {
                let Wait::Starting { prompt } = wait else {
                    return None;
                };
                let since = latest_mark(snapshot, &session);
                let reported = since.as_ref().map_or(0, |mark| mark.0);
                if now() < reported.saturating_add(SETTLE_SECS) {
                    return None;
                }
                if let Err(error) = self.type_line(&session, &one_line(prompt)) {
                    return Some(Err(format!("{error:#}")));
                }
                let working = Phase::Command(Wait::Working { since });
                self.phases.insert(id.to_owned(), (working, Instant::now()));
                self.set_waits(id, None);
                None
            }
            Progress::Pending
                if matches!(wait, Wait::Starting { .. }) && started.elapsed() > START_LIMIT =>
            {
                Some(Err(
                    "No agent hook report in 120 seconds. Install the agent's hooks in Settings. \
                     An agent that reports only after its first prompt (Codex) needs the prompt \
                     in its command, not in `prompt`."
                        .to_owned(),
                ))
            }
            Progress::Pending => None,
        }
    }

    fn session_of(&self, id: &str) -> String {
        self.state
            .steps
            .get(id)
            .and_then(|run| run.session.clone())
            .unwrap_or_default()
    }

    /// The answer that `flow approve` or `flow reject` wrote.
    fn gate_answer(&self, id: &str) -> End {
        let answer = std::fs::read_to_string(self.dir.join(ANSWERS).join(id)).ok()?;
        match answer.trim() {
            APPROVE => Some(Ok(())),
            REJECT => Some(Err("Rejected".to_owned())),
            _ => None,
        }
    }

    fn finish(&mut self, id: &str, end: End) {
        match end {
            None => return,
            Some(Ok(())) => {
                self.state.set(id, StepStatus::Done);
                Output::line_or_drop(format_args!("{id}: done"));
            }
            Some(Err(note)) => {
                Output::line_or_drop(format_args!("{id}: failed: {note}"));
                self.state.fail(id, note);
            }
        }
        self.changed = true;
        self.phases.remove(id);
        if let Some(run) = self.state.steps.get_mut(id) {
            run.waits_for = None;
        }
    }
}
