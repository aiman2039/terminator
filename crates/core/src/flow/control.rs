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

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("Cannot read the run: {0}")]
    Io(#[from] std::io::Error),
    #[error("The run state file is damaged")]
    State,
    #[error("This run ended")]
    Ended,
    #[error("This step is not a gate that waits for an answer")]
    NotGate,
    #[error("Unknown step")]
    UnknownStep,
    #[error("This step has no open question")]
    NoQuestion,
    #[error("The text is empty or longer than 16 KiB")]
    Text,
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
    let mut found: Vec<_> = std::fs::read_dir(data.join(RUN_DIR))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let dir = entry.path();
            let changed = dir.join(STATE_FILE).metadata().ok()?.modified().ok()?;
            Some((changed, dir.clone(), read_state(&dir).ok()?))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    found
        .into_iter()
        .map(|(_, dir, state)| (dir, state))
        .collect()
}

fn write(dir: &Path, folder: &str, step: &str, bytes: &[u8]) -> Result<(), RunError> {
    std::fs::create_dir_all(dir.join(folder))?;
    Ok(crate::atomic_write(&dir.join(folder).join(step), bytes)?)
}

/// A run that did not end.
fn open(dir: &Path) -> Result<RunState, RunError> {
    let state = read_state(dir)?;
    if state.ended {
        return Err(RunError::Ended);
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
