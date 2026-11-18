//! The run loop of `terminator-hook flow run`. It starts each step when
//! the steps it needs are done, and it records the run in the state file.
//! It never closes or restarts a session.
use super::{
    super::{
        agents::latest_mark,
        lookup::{Ctl, NewSession},
    },
    ANSWERS, APPROVE, CANCEL, REJECT, agent,
    progress::{Phase, Progress, one_line, progress, with_results},
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
    flow::{Flow, Outcome, RunState, STATE_FILE, Step, StepKind, StepStatus},
    *,
};

const POLL: Duration = Duration::from_millis(500);
/// Longest wait for the agent's first hook report.
const START_LIMIT: Duration = Duration::from_mins(2);
/// Seconds between the agent's first hook report and the prompt, so the
/// agent's input line is there.
const SETTLE_SECS: u64 = 2;

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
    /// Ctrl-C or SIGTERM: cancel the run.
    interrupted: Arc<AtomicBool>,
}

impl Runner {
    pub(super) fn new(paths: Paths, project: Project, flow: Flow, dir: PathBuf) -> Result<Self> {
        let interrupted = Arc::new(AtomicBool::new(false));
        for signal in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM] {
            signal_hook::flag::register(signal, Arc::clone(&interrupted))?;
        }
        Ok(Self {
            state: RunState::new(&flow, &project.id),
            paths,
            project,
            flow,
            dir,
            phases: BTreeMap::new(),
            saved: Vec::new(),
            interrupted,
        })
    }

    /// Runs until every step is done, a step stops the run, or the user
    /// cancels. A cancel starts no new step and leaves sessions alone.
    pub(super) fn run(mut self) -> Result<RunState> {
        std::fs::create_dir_all(self.dir.join(ANSWERS))?;
        self.save()?;
        loop {
            if self.interrupted.load(Ordering::Relaxed) || self.dir.join(CANCEL).exists() {
                self.state.cancel();
                break;
            }
            let ready: Vec<Step> = self.flow.ready(&self.state).into_iter().cloned().collect();
            for step in &ready {
                self.start(step);
            }
            // A daemon that does not answer now can answer later.
            if let Ok(snapshot) = Ctl::snapshot(&self.paths) {
                let running: Vec<String> = self.phases.keys().cloned().collect();
                for id in running {
                    let end = self.poll(&id, &snapshot);
                    self.finish(&id, end);
                }
            }
            self.sync_questions();
            self.save()?;
            if self.flow.outcome(&self.state) != Outcome::Active {
                break;
            }
            std::thread::sleep(POLL);
        }
        self.state.ended = true;
        self.save()?;
        Ok(self.state)
    }

    fn save(&mut self) -> Result<()> {
        let text = serde_json::to_vec_pretty(&self.state)?;
        if text != self.saved {
            atomic_write(&self.dir.join(STATE_FILE), &text)?;
            self.saved = text;
        }
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
                self.phases.insert(step.id.clone(), (phase, Instant::now()));
                Output::line(format_args!("{}: started", step.id));
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
            project = ctl.spawn_worktree(&project, name)?;
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
        self.save()?;
        self.type_line(&session.id, command)?;
        // Without a GUI the session runs in the background.
        if ui_control::rpc(&self.paths, ui_control::Request::Ping).is_ok() {
            let _ = ui_control::rpc(
                &self.paths,
                ui_control::Request::ShowSession {
                    session: session.id.clone(),
                    anchor: None,
                    split: None,
                },
            );
        }
        let phase = match prompt {
            Some(prompt) => {
                // Handoff: the result notes of the steps this one needs.
                let results = step.needs.iter().filter_map(|needs| {
                    let note = self.state.steps.get(needs)?.note.as_deref()?;
                    Some((needs.as_str(), note))
                });
                Phase::Starting {
                    prompt: with_results(&self.flow.full_prompt(prompt), results),
                }
            }
            None => Phase::Working { since: None },
        };
        Ok(phase)
    }

    /// Types one line, then presses Enter in a second write (as `ctl send
    /// --enter` does).
    fn type_line(&self, session: &str, line: &str) -> Result<()> {
        // The session can be newer than an older snapshot.
        Ctl::connect(&self.paths)?.type_and_enter(session, line.as_bytes().to_vec())
    }

    fn announce_gate(&self, step: &Step, message: &str) {
        let run = self.run_name();
        Output::line(format_args!(
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
                .then(|| terminator_core::flow::control::question(&self.dir, id))
                .flatten();
            if question == run.question {
                continue;
            }
            if let (Some(question), Some(session)) = (&question, &run.session) {
                asked.push((id.clone(), session.clone(), question.clone()));
            }
            run.question = question;
        }
        let run = self.run_name();
        for (id, session, question) in asked {
            Output::line(format_args!(
                "{id}: question: {question}\n  terminator-hook flow answer {run} {id} \"...\""
            ));
            self.notify(session, "Flow question", &question);
        }
    }

    fn run_name(&self) -> String {
        self.dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    fn poll(&mut self, id: &str, snapshot: &State) -> End {
        let (phase, _) = self.phases.get(id)?.clone();
        // The step's own report (`flow report`) is explicit, so it comes
        // before the hook state and before an ended terminal.
        if let Some(report) = agent::read_report(&self.dir, id).filter(|_| phase != Phase::Gate) {
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
        match &phase {
            Phase::Gate => self.gate_answer(id),
            Phase::Starting { prompt } => self.poll_start(id, prompt, snapshot),
            Phase::Working { .. } => match progress(snapshot, &self.session_of(id), &phase) {
                Progress::Done => Some(Ok(())),
                Progress::Failed(note) => Some(Err(note.to_owned())),
                Progress::Pending | Progress::SendPrompt => None,
            },
        }
    }

    fn session_of(&self, id: &str) -> String {
        self.state
            .steps
            .get(id)
            .and_then(|run| run.session.clone())
            .unwrap_or_default()
    }

    /// Sends the prompt when the agent's first hook report is there.
    fn poll_start(&mut self, id: &str, prompt: &str, snapshot: &State) -> End {
        let session = self.session_of(id);
        let (phase, started) = self.phases.get(id)?.clone();
        match progress(snapshot, &session, &phase) {
            Progress::SendPrompt => {
                let since = latest_mark(snapshot, &session);
                let reported = since.as_ref().map_or(0, |mark| mark.0);
                if now() < reported.saturating_add(SETTLE_SECS) {
                    return None;
                }
                if let Err(error) = self.type_line(&session, &one_line(prompt)) {
                    return Some(Err(format!("{error:#}")));
                }
                self.phases
                    .insert(id.to_owned(), (Phase::Working { since }, Instant::now()));
                None
            }
            Progress::Pending if started.elapsed() > START_LIMIT => Some(Err(
                "No agent hook report in 120 seconds. Install the agent's hooks in Settings. \
                 An agent that reports only after its first prompt (Codex) needs the prompt \
                 in its command, not in `prompt`."
                    .to_owned(),
            )),
            Progress::Failed(note) => Some(Err(note.to_owned())),
            Progress::Pending | Progress::Done => None,
        }
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
                Output::line(format_args!("{id}: done"));
            }
            Some(Err(note)) => {
                Output::line(format_args!("{id}: failed: {note}"));
                self.state.fail(id, note);
            }
        }
        self.phases.remove(id);
    }
}
