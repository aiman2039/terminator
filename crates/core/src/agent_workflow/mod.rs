//! Agent workflow requests shared by the GUI and the daemon: inbox triage
//! verbs and restoring interrupted shells. Each is gated on its own
//! capability so a new GUI keeps working with an older daemon.
use crate::Notification;

/// `Request::Notice` also takes `unread`, `restore` and `snooze-for:<secs>`.
pub const TRIAGE_CAPABILITY: &str = "notice-triage-v1";
/// `Request::CreateRestored` opens a fresh shell with saved output above it.
pub const RESTORE_CAPABILITY: &str = "restore-shell-v1";

/// Longest snooze a client may ask for.
pub const MAX_SNOOZE_SECS: u64 = 7 * 24 * 60 * 60;
/// The fixed snooze of the plain `snooze` action (older clients).
pub const DEFAULT_SNOOZE_SECS: u64 = 600;
/// Most saved output a restored shell shows above its prompt.
pub const MAX_RESTORE_SEED: usize = 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub struct UnknownAction;

impl std::fmt::Display for UnknownAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Unknown notification action")
    }
}

impl std::error::Error for UnknownAction {}

/// The wire form of a snooze for `secs` seconds.
#[must_use]
pub fn snooze_for(secs: u64) -> String {
    format!("snooze-for:{secs}")
}

/// Applies one notice action. Read state, dismissal and snooze only:
/// nothing here resolves a notice or touches agent lifecycle.
///
/// # Errors
/// Returns [`UnknownAction`] for an action this daemon does not know.
pub fn apply(notice: &mut Notification, action: &str, now: u64) -> Result<(), UnknownAction> {
    match action {
        "read" => notice.read = true,
        "unread" => notice.read = false,
        "dismiss" => notice.dismissed = true,
        "snooze" => notice.snoozed_until = now.saturating_add(DEFAULT_SNOOZE_SECS),
        // Undo of a dismiss or snooze: the notice is pending again.
        "restore" => {
            notice.dismissed = false;
            notice.snoozed_until = 0;
        }
        other => {
            let secs = other
                .strip_prefix("snooze-for:")
                .and_then(|secs| secs.parse::<u64>().ok())
                .filter(|secs| (1..=MAX_SNOOZE_SECS).contains(secs))
                .ok_or(UnknownAction)?;
            notice.snoozed_until = now.saturating_add(secs);
        }
    }
    Ok(())
}

/// The last `MAX_RESTORE_SEED` bytes of saved output, cut on a line start,
/// with bare `\n` turned into `\r\n` so each line starts at column 0.
#[must_use]
pub fn restore_seed(text: &str) -> String {
    let mut start = text.len().saturating_sub(MAX_RESTORE_SEED);
    while !text.is_char_boundary(start) {
        start = start.saturating_add(1);
    }
    let mut tail = text.get(start..).unwrap_or_default();
    if start > 0
        && let Some((_, rest)) = tail.split_once('\n')
    {
        tail = rest;
    }
    let tail = tail.trim_end();
    if tail.is_empty() {
        return String::new();
    }
    let mut seed = String::with_capacity(tail.len().saturating_add(128));
    for line in tail.split('\n') {
        seed.push_str(line.trim_end_matches('\r'));
        seed.push_str("\r\n");
    }
    seed.push_str("\x1b[2m-- Saved output from before the restart --\x1b[0m\r\n");
    seed
}

#[cfg(test)]
mod tests;
