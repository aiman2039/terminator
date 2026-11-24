//! Step mailbox (`flow send` / `flow inbox`) and structured step output
//! path validation (`flow report --output PATH`). Nothing here writes to
//! any terminal.
use super::StepStatus;
use super::control::{RunError, bounded, open, write};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

/// Folder of step mailbox files in a run folder: one JSON file per step.
pub const MAILBOX: &str = "mailbox";
/// Most messages stored in one step's mailbox.
pub const MAX_MESSAGES: usize = 100;
/// Most output paths one step may report.
pub const MAX_OUTPUTS: usize = 32;
/// Longest output path, in bytes.
const MAX_OUTPUT_PATH: usize = 1024;

/// One message stored in a step's mailbox.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailMessage {
    pub seq: u64,
    pub at: u64,
    pub text: String,
}

/// Store a message in `step`'s mailbox without writing to any terminal.
pub fn send_message(dir: &Path, step: &str, text: &str) -> Result<MailMessage, RunError> {
    let state = open(dir)?;
    let run = state.steps.get(step).ok_or(RunError::UnknownStep)?;
    if run.gate.is_some() || !matches!(run.status, StepStatus::Waiting | StepStatus::Running) {
        return Err(RunError::CannotSend);
    }
    let text = bounded(text)?;
    let mut messages = read_inbox(dir, step);
    if messages.len() >= MAX_MESSAGES {
        return Err(RunError::MailboxFull);
    }
    let seq = messages.last().map_or(1, |msg| msg.seq.saturating_add(1));
    let message = MailMessage {
        seq,
        at: crate::now(),
        text: text.to_owned(),
    };
    messages.push(message.clone());
    let bytes = serde_json::to_vec(&messages).map_err(|_| RunError::State)?;
    write(dir, MAILBOX, step, &bytes)?;
    Ok(message)
}

/// Read all messages stored in `step`'s mailbox (empty when none exist).
pub fn read_inbox(dir: &Path, step: &str) -> Vec<MailMessage> {
    let Ok(bytes) = std::fs::read(dir.join(MAILBOX).join(step)) else {
        return Vec::new();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Validate one output path reported by `flow report --output PATH`.
pub fn output_path_ok(raw: &str) -> Result<String, RunError> {
    let path = raw.trim();
    if path.is_empty()
        || path.len() > MAX_OUTPUT_PATH
        || path.chars().any(char::is_control)
        || Path::new(path)
            .components()
            .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(RunError::BadOutput(path.to_owned()));
    }
    Ok(path.to_owned())
}
