//! Agent workflow requests shared by the GUI and the daemon: inbox triage
//! verbs and restoring interrupted shells. Each is gated on its own
//! capability so a new GUI keeps working with an older daemon.
use crate::Notification;

/// `Request::Notice` also takes `unread`, `restore` and `snooze-for:<secs>`.
pub const TRIAGE_CAPABILITY: &str = "notice-triage-v1";
/// `Request::CreateRestored` opens a fresh shell with saved output above it.
pub const RESTORE_CAPABILITY: &str = "restore-shell-v1";

/// Hook events carry `usage` and `State::agent_usage` keeps it.
pub const USAGE_CAPABILITY: &str = "agent-usage-v1";

/// What an agent itself reports in its hook input: the model and the
/// permission mode. Display only; never a lifecycle source. Fields no
/// supported agent reports (context fullness, cost) are not kept.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AgentUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,
}

/// Longest stored model or mode text.
const MAX_USAGE_TEXT: usize = 64;

fn usage_text(value: Option<&str>) -> Option<String> {
    let text: String = value?
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_USAGE_TEXT)
        .collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

impl AgentUsage {
    /// Usage from the raw hook fields, cleaned and capped.
    #[must_use]
    pub fn new(model: Option<&str>, permission_mode: Option<&str>) -> Option<Self> {
        let usage = Self {
            model: usage_text(model),
            permission_mode: usage_text(permission_mode),
        };
        (!usage.is_empty()).then_some(usage)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.model.is_none() && self.permission_mode.is_none()
    }

    /// Newer reported fields replace older ones; missing ones keep theirs.
    pub fn merge(&mut self, newer: &Self) {
        if newer.model.is_some() {
            self.model.clone_from(&newer.model);
        }
        if newer.permission_mode.is_some() {
            self.permission_mode.clone_from(&newer.permission_mode);
        }
    }

    /// Short chip text for a permission mode, by the agents' own names.
    #[must_use]
    pub fn mode_label(&self) -> Option<&str> {
        Some(match self.permission_mode.as_deref()? {
            "default" => return None,
            "plan" => "Plan",
            "acceptEdits" => "Accept edits",
            "auto" => "Auto",
            "dontAsk" => "Don't ask",
            "bypassPermissions" => "Bypass",
            other => other,
        })
    }
}

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
