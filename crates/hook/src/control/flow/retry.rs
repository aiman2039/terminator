//! Retry requests (`flow retry`, the GUI's Retry): one file per step in the
//! run folder, with the attempt to start. The runner takes them; a request
//! for a step that is not failed or cancelled, or for another attempt, is
//! dropped. Nothing here starts a terminal: the runner starts the retried
//! step like any other step that can start.
use super::{ANSWERS, agent::REPORTS};
use std::path::Path;
use terminator_core::flow::{
    RunState,
    control::{QUESTIONS, REPLIES, RETRIES},
};

/// Takes the Retry requests in the run folder `dir`. Each good one makes
/// its step wait again as its next attempt, with the steps after it (see
/// [`RunState::retry`]). Gives the retried step IDs.
pub(super) fn take_retries(dir: &Path, state: &mut RunState) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir.join(RETRIES)) else {
        return Vec::new();
    };
    let mut retried = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let asked = std::fs::read_to_string(&path).ok();
        // A request is used one time.
        let _ = std::fs::remove_file(&path);
        let Some(id) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let next = state
            .steps
            .get(id)
            .filter(|step| step.retryable())
            .map(|step| step.attempt().saturating_add(1));
        let wanted = asked.and_then(|text| text.trim().parse::<u32>().ok());
        if next.is_none() || next != wanted || !state.retry(id) {
            continue;
        }
        clear_answers(dir, id);
        retried.push(id.to_owned());
    }
    retried.sort_unstable();
    retried
}

/// Removes the report, gate answer and question of step `id`: an earlier
/// attempt's or round's are not the next one's.
pub(super) fn clear_answers(dir: &Path, id: &str) {
    for folder in [REPORTS, ANSWERS, QUESTIONS, REPLIES] {
        let _ = std::fs::remove_file(dir.join(folder).join(id));
    }
}
