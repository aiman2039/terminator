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
/// Folder of open questions (`flow ask`): one file per step.
pub const QUESTIONS: &str = "questions";
/// Folder of the user's answers to questions: one file per step.
pub const REPLIES: &str = "replies";
/// Longest note, question or answer, in bytes.
pub const MAX_TEXT: usize = 16 * 1024;
/// File in a run folder that the runner keeps locked while it lives.
pub const RUNNER_LOCK: &str = "runner.lock";

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
    #[error("Unknown step")]
    UnknownStep,
    #[error("This step has no open question")]
    NoQuestion,
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

/// Every readable run in the data folder, newest first, with its folder.
/// Blocking: call it off the GUI thread.
pub fn runs(data: &Path) -> Vec<(PathBuf, RunState)> {
    recent_runs(data, usize::MAX)
}

/// The readable runs among the `limit` run folders whose state file
/// changed last, newest first. Only those state files are read.
/// Blocking: call it off the GUI thread.
pub fn recent_runs(data: &Path, limit: usize) -> Vec<(PathBuf, RunState)> {
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
    found
        .into_iter()
        .take(limit)
        .filter_map(|(_, dir)| Some((dir.clone(), read_state(&dir).ok()?)))
        .collect()
}

fn write(dir: &Path, folder: &str, step: &str, bytes: &[u8]) -> Result<(), RunError> {
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
/// lock file (from an older build) has no runner that can be seen, so it
/// counts as gone. A lock that cannot be checked counts as held. Blocking:
/// call it off the GUI thread.
pub fn alive(dir: &Path) -> bool {
    let file = match std::fs::File::open(dir.join(RUNNER_LOCK)) {
        Ok(file) => file,
        Err(error) => return error.kind() != std::io::ErrorKind::NotFound,
    };
    // A shared lock taken here goes away with `file`.
    !matches!(file.try_lock_shared(), Ok(()))
}

/// A run that did not end and whose runner is gone: nothing changes it
/// again. Blocking: call it off the GUI thread.
pub fn runner_gone(dir: &Path, state: &RunState) -> bool {
    !state.ended && !alive(dir)
}

/// A run that did not end, with its runner.
fn open(dir: &Path) -> Result<RunState, RunError> {
    let state = read_state(dir)?;
    if state.ended {
        return Err(RunError::Ended);
    }
    if !alive(dir) {
        return Err(RunError::RunnerGone);
    }
    Ok(state)
}

/// Approve or reject a gate step that waits now. A step ID of the run is
/// a safe file name, so only such an ID is written.
pub fn answer_gate(dir: &Path, step: &str, approve: bool) -> Result<(), RunError> {
    let waits = open(dir)?
        .steps
        .get(step)
        .is_some_and(|run| run.gate.is_some() && run.status == StepStatus::Running);
    if !waits {
        return Err(RunError::NotGate);
    }
    let answer = if approve { APPROVE } else { REJECT };
    write(dir, ANSWERS, step, answer.as_bytes())
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
    if !read_state(dir)?.ended && alive(dir) {
        return Err(RunError::NotEnded);
    }
    // `remove_dir_all` deletes links inside the folder, never their targets.
    Ok(std::fs::remove_dir_all(dir)?)
}
