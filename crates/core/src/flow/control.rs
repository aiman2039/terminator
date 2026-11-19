//! The user's side of a flow run, shared by `terminator-hook flow` and the
//! GUI: read the runs, answer a gate, answer a step's question, cancel.
//! Each action is one file in the run folder that the runner reads. None
//! of them starts a step or types into a terminal.
use super::{RUN_DIR, RunState, STATE_FILE, StepStatus};
use std::path::{Path, PathBuf};

/// Folder of gate answers in a run folder: one file per step.
pub const ANSWERS: &str = "answers";
/// File in a run folder that cancels the run.
pub const CANCEL: &str = "cancel";
pub const APPROVE: &str = "approve";
pub const REJECT: &str = "reject";
/// Start of an answer file that chooses a gate option: `option:NAME`.
pub const CHOOSE: &str = "option:";
/// Folder of open questions (`flow ask`): one file per step.
pub const QUESTIONS: &str = "questions";
/// Folder of the user's answers to questions: one file per step.
pub const REPLIES: &str = "replies";
/// Longest note, question or answer, in bytes.
pub const MAX_TEXT: usize = 16 * 1024;
/// File in a run folder that the runner keeps locked while it lives.
pub const RUNNER_LOCK: &str = "runner.lock";
/// Folder of Retry requests: one file per step, with the attempt to start.
pub const RETRIES: &str = "retries";

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("Cannot read the run: {0}")]
    Io(#[from] std::io::Error),
    #[error("The run state file is damaged")]
    State,
    #[error("This run ended")]
    Ended,
    #[error(
        "The runner of this run is gone (its terminal closed, or the process stopped). Nothing reads this run now; remove it and run the flow again"
    )]
    RunnerGone,
    #[error("This step is not a gate that waits for an answer")]
    NotGate,
    #[error("This gate needs one of its options: {0} (--option NAME)")]
    NeedsOption(String),
    #[error("This gate has no options; approve it without --option")]
    NoOptions,
    #[error("This gate has no option \"{0}\"")]
    UnknownOption(String),
    #[error("Unknown step")]
    UnknownStep,
    #[error("This step has no open question")]
    NoQuestion,
    #[error("Only a failed or cancelled step can start again")]
    NotRetryable,
    #[error("The text is empty or longer than 16 KiB")]
    Text,
    #[error(
        "This run has not ended, so it stays in the list. Cancel it and wait until its runner ends it"
    )]
    NotEnded,
    #[error("This folder is not a run folder")]
    NotRun,
    #[error(transparent)]
    Write(#[from] crate::AtomicWriteError),
}

/// Trimmed text that is not empty and not too long.
pub fn bounded(text: &str) -> Result<&str, RunError> {
    let text = text.trim();
    if text.is_empty() || text.len() > MAX_TEXT {
        return Err(RunError::Text);
    }
    Ok(text)
}

pub fn read_state(dir: &Path) -> Result<RunState, RunError> {
    serde_json::from_slice(&std::fs::read(dir.join(STATE_FILE))?).map_err(|_| RunError::State)
}

/// The name of a run: its folder name, which `terminator-hook flow`
/// commands take.
pub fn run_name(dir: &Path) -> String {
    dir.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Where a run is, from its state file and its runner's lock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunStatus {
    /// Not ended, and its runner lives.
    Running,
    /// Ended with every step done.
    Done,
    /// Ended with a step that is not done.
    Stopped,
    /// Not ended, but its runner is gone (its terminal closed, the process
    /// was killed, or the computer restarted): nothing changes it again.
    RunnerGone,
}

impl RunStatus {
    /// The status of `state` whose runner is `alive` or not.
    pub fn of(state: &RunState, alive: bool) -> Self {
        let done = state.steps.values().all(|step| step.status.passed());
        match (state.ended, alive, done) {
            (true, _, true) => Self::Done,
            (true, _, false) => Self::Stopped,
            (false, true, _) => Self::Running,
            (false, false, _) => Self::RunnerGone,
        }
    }

    /// "running", "done", "stopped" or "stopped (runner gone)".
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Done => "done",
            Self::Stopped => "stopped",
            Self::RunnerGone => "stopped (runner gone)",
        }
    }

    /// Ended, or its runner is gone: nothing changes the run again.
    pub fn ended(self) -> bool {
        self != Self::Running
    }
}

/// A run as its readers show it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunView {
    /// For a run whose runner is gone, the steps that did not end show as
    /// cancelled (in memory only): nothing answers them now.
    pub state: RunState,
    pub status: RunStatus,
}

/// The view of a run whose state file `state` was read from `dir`.
/// Blocking (the runner's lock): call it off the GUI thread.
pub fn run_view(dir: &Path, mut state: RunState) -> RunView {
    let status = RunStatus::of(&state, state.ended || alive(dir));
    if status == RunStatus::RunnerGone {
        state.cancel();
    }
    RunView { state, status }
}

/// Read the run in `dir`. Blocking: call it off the GUI thread.
pub fn read_run(dir: &Path) -> Result<RunView, RunError> {
    Ok(run_view(dir, read_state(dir)?))
}

/// Every readable run in the data folder, newest first, with its folder.
#[cfg(test)]
pub fn runs(data: &Path) -> Vec<(PathBuf, RunState)> {
    recent_runs(data, usize::MAX, &mut EndedRuns::default())
}

/// The older run folders that [`recent_runs`] found ended, with the time
/// their state file changed. A later read skips them without reading the
/// file again while that time stays the same.
pub type EndedRuns = std::collections::HashMap<PathBuf, std::time::SystemTime>;

/// The readable runs among the `limit` run folders whose state file
/// changed last, newest first, and after them every older run that did
/// not end: a gate there can still wait, or its runner is gone and only
/// Remove clears it. `ended` keeps the older ended runs between reads, so
/// their files are read once. Blocking: call it off the GUI thread.
pub fn recent_runs(data: &Path, limit: usize, ended: &mut EndedRuns) -> Vec<(PathBuf, RunState)> {
    let mut found: Vec<_> = std::fs::read_dir(data.join(RUN_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let dir = entry.path();
            let changed = dir.join(STATE_FILE).metadata().ok()?.modified().ok()?;
            Some((changed, dir))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    let mut seen = EndedRuns::new();
    let runs = found
        .into_iter()
        .enumerate()
        .filter_map(|(index, (changed, dir))| {
            let older = index >= limit;
            if older && ended.get(&dir) == Some(&changed) {
                seen.insert(dir, changed);
                return None;
            }
            let state = read_state(&dir).ok()?;
            if older && state.ended {
                seen.insert(dir, changed);
                return None;
            }
            Some((dir, state))
        })
        .collect();
    // Removed folders leave the cache.
    *ended = seen;
    runs
}

/// Write `bytes` as the file `step` in the folder `folder` of the run in
/// `dir`, in one step. `step` must be a step ID of the run.
pub fn write(dir: &Path, folder: &str, step: &str, bytes: &[u8]) -> Result<(), RunError> {
    std::fs::create_dir_all(dir.join(folder))?;
    Ok(crate::atomic_write(&dir.join(folder).join(step), bytes)?)
}

/// The runner's exclusive lock on [`RUNNER_LOCK`] in its run folder, held
/// while this value lives. The OS releases it when the runner ends in any
/// way (also SIGKILL or a reboot), so a reader that can take the lock knows
/// that the runner is gone.
pub struct RunnerLock {
    _file: std::fs::File,
}

/// Take the runner's lock. Call it before the first state file write.
pub fn hold_runner(dir: &Path) -> Result<RunnerLock, RunError> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(RUNNER_LOCK))?;
    file.lock()?;
    Ok(RunnerLock { _file: file })
}

/// True while a runner holds the lock of the run in `dir`. A run with no
/// lock file has no runner that can be seen, so it counts as gone. A lock
/// that cannot be checked counts as held. Blocking: call it off the GUI
/// thread.
pub fn alive(dir: &Path) -> bool {
    let file = match std::fs::File::open(dir.join(RUNNER_LOCK)) {
        Ok(file) => file,
        Err(error) => return error.kind() != std::io::ErrorKind::NotFound,
    };
    // A shared lock taken here goes away with `file`.
    !matches!(file.try_lock_shared(), Ok(()))
}

/// A run that did not end, with its runner.
fn open(dir: &Path) -> Result<RunState, RunError> {
    let view = read_run(dir)?;
    match view.status {
        RunStatus::Running => Ok(view.state),
        RunStatus::RunnerGone => Err(RunError::RunnerGone),
        RunStatus::Done | RunStatus::Stopped => Err(RunError::Ended),
    }
}

/// The user's answer to a gate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateAnswer<'a> {
    /// Approve a gate with no options.
    Approve,
    /// Approve a gate with options, with this option.
    Choose(&'a str),
    Reject,
}

impl<'a> GateAnswer<'a> {
    /// The text of the answer file.
    pub fn text(self) -> String {
        match self {
            Self::Approve => APPROVE.to_owned(),
            Self::Choose(option) => format!("{CHOOSE}{option}"),
            Self::Reject => REJECT.to_owned(),
        }
    }

    /// The answer in an answer file's text.
    pub fn parse(text: &'a str) -> Option<Self> {
        match text.trim() {
            APPROVE => Some(Self::Approve),
            REJECT => Some(Self::Reject),
            text => text.strip_prefix(CHOOSE).map(Self::Choose),
        }
    }
}

/// Approve or reject a gate step that waits now. A gate with options takes
/// only one of its options; a gate without takes only Approve. A step ID
/// of the run is a safe file name, so only such an ID is written.
pub fn answer_gate(dir: &Path, step: &str, answer: GateAnswer<'_>) -> Result<(), RunError> {
    let state = open(dir)?;
    let Some(run) = state
        .steps
        .get(step)
        .filter(|run| run.gate.is_some() && run.status == StepStatus::Running)
    else {
        return Err(RunError::NotGate);
    };
    match answer {
        GateAnswer::Approve if !run.options.is_empty() => {
            return Err(RunError::NeedsOption(run.options.join(", ")));
        }
        GateAnswer::Choose(_) if run.options.is_empty() => return Err(RunError::NoOptions),
        GateAnswer::Choose(option) if !run.options.iter().any(|name| name == option) => {
            return Err(RunError::UnknownOption(option.to_owned()));
        }
        _ => {}
    }
    write(dir, ANSWERS, step, answer.text().as_bytes())
}

/// The open question of a step, if any.
pub fn question(dir: &Path, step: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(QUESTIONS).join(step)).ok()
}

/// The user's answer to a step's open question.
pub fn answer_question(dir: &Path, step: &str, text: &str) -> Result<(), RunError> {
    if !open(dir)?.steps.contains_key(step) {
        return Err(RunError::UnknownStep);
    }
    if !dir.join(QUESTIONS).join(step).is_file() {
        return Err(RunError::NoQuestion);
    }
    write(dir, REPLIES, step, bounded(text)?.as_bytes())
}

/// Ask the runner to start a failed or cancelled step again, as its next
/// attempt, in a new terminal. Refused for a run that ended or whose
/// runner is gone. The old terminal stays as it is.
pub fn retry(dir: &Path, step: &str) -> Result<(), RunError> {
    let state = open(dir)?;
    let run = state.steps.get(step).ok_or(RunError::UnknownStep)?;
    if !run.retryable() {
        return Err(RunError::NotRetryable);
    }
    let next = run.attempt().saturating_add(1);
    write(dir, RETRIES, step, next.to_string().as_bytes())
}

/// Stop a run: the runner starts no new step and leaves sessions alone.
pub fn cancel(dir: &Path) -> Result<(), RunError> {
    open(dir)?;
    Ok(std::fs::write(dir.join(CANCEL), b"")?)
}

/// Delete the folder of an ended run, or of a run whose runner is gone.
/// `dir` must be a real folder (not a link) directly in the run folder of
/// `data`, with a plain name and a readable state file. Its sessions are
/// not touched. Blocking: call it off the GUI thread.
pub fn remove(data: &Path, dir: &Path) -> Result<(), RunError> {
    let plain = dir
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(super::files::plain_name);
    if !plain || !std::fs::symlink_metadata(dir)?.is_dir() {
        return Err(RunError::NotRun);
    }
    let runs = std::fs::canonicalize(data.join(RUN_DIR))?;
    let parent = std::fs::canonicalize(dir)?.parent().map(Path::to_path_buf);
    if parent.as_deref() != Some(runs.as_path()) {
        return Err(RunError::NotRun);
    }
    if !read_run(dir)?.status.ended() {
        return Err(RunError::NotEnded);
    }
    // `remove_dir_all` deletes links inside the folder, never their targets.
    Ok(std::fs::remove_dir_all(dir)?)
}
