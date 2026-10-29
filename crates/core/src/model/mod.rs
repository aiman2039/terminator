use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::PathBuf};

use super::{
    MAX_AGENTS, MAX_NOTIFICATIONS, MAX_RECENT_EVENTS, PROTOCOL_VERSION, Paths, id, now, quote,
};
use crate::{agents, generations, vnc, worktrees};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    Running,
    Stopping,
    Ended,
    Interrupted,
}
impl Lifecycle {
    #[must_use]
    pub fn live(&self) -> bool {
        matches!(self, Self::Running | Self::Stopping)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Shell,
    Editor,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// App-owned, read-only Neovim Git review (independent of editor settings).
    #[serde(default)]
    pub review: bool,
    #[serde(default)]
    pub remote: Option<crate::remote::RemoteSession>,
    pub id: String,
    pub project_id: String,
    pub label: String,
    pub cwd: PathBuf,
    pub kind: SessionKind,
    pub file: Option<PathBuf>,
    pub lifecycle: Lifecycle,
    pub created: u64,
    pub exit_code: Option<u32>,
    pub rows: u16,
    pub cols: u16,
    pub generation: String,
    pub pid: Option<u32>,
    pub truncated: bool,
    pub cwd_confirmed: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub layout: serde_json::Value,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Unknown,
    Running,
    WaitingInput,
    WaitingPermission,
    Completed,
    Failed,
    Stopped,
}
impl AgentState {
    #[must_use]
    pub fn actionable(self) -> bool {
        matches!(
            self,
            Self::WaitingInput | Self::WaitingPermission | Self::Completed | Self::Failed
        )
    }
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Running => "Working",
            Self::WaitingInput => "Needs input",
            Self::WaitingPermission => "Needs permission",
            Self::Completed => "Completed",
            Self::Failed => "Failed",
            Self::Stopped => "Stopped",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HookEvent {
    pub protocol_version: u32,
    pub event_id: String,
    pub terminal_session_id: String,
    pub agent_invocation_id: String,
    pub agent_kind: String,
    pub provider_session_id: Option<String>,
    pub state: AgentState,
    pub request_id: Option<String>,
    pub sequence: Option<u64>,
    pub summary: String,
    #[serde(default)]
    pub details: String,
    #[serde(default)]
    pub resume: Option<Resume>,
    /// Hook-helper ancestor process identity. The daemon verifies it against
    /// live presence observations before linking lifecycle to a process.
    #[serde(default)]
    pub process: Option<agents::ProcessIdentity>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resume {
    pub program: String,
    pub args: Vec<String>,
}
impl Resume {
    #[must_use]
    pub fn display(&self) -> String {
        std::iter::once(&self.program)
            .chain(self.args.iter())
            .map(|s| quote(s))
            .collect::<Vec<_>>()
            .join(" ")
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agent {
    pub invocation_id: String,
    pub session_id: String,
    pub kind: String,
    pub provider_session_id: Option<String>,
    pub state: AgentState,
    pub sequence: Option<u64>,
    pub updated: u64,
    pub resume: Option<Resume>,
    /// Last verified process link. Set only when the daemon confirms the
    /// hook's process identity against a live presence observation of the
    /// same kind; unlinked hook events remain as "Last reported" state.
    #[serde(default)]
    pub process: Option<agents::ProcessIdentity>,
}
impl Agent {
    /// A session is worth keeping in History only when an agent left a
    /// provider resume command behind (e.g. `codex resume <id>`).
    /// Plain shells and file editors have no such handle, so reopening
    /// them restores nothing actionable.
    #[must_use]
    pub fn resumable(&self) -> bool {
        self.resume.is_some()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Notification {
    pub id: String,
    pub session_id: String,
    pub invocation_id: String,
    pub request_id: Option<String>,
    pub state: AgentState,
    pub summary: String,
    pub details: String,
    pub created: u64,
    pub read: bool,
    pub dismissed: bool,
    pub resolved: bool,
    pub snoozed_until: u64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Dismissal {
    OnFocus,
    OnResolve,
    Manual,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum EditorMode {
    Embedded,
    Terminal,
    External,
    /// GUI-owned buffer (`terminator-native-edit`); no PTY, no daemon editor.
    Native,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewMode {
    #[default]
    Native,
    Neovim,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub events: BTreeSet<AgentState>,
    pub os_events: BTreeSet<AgentState>,
    pub dismissal: Dismissal,
    pub notifications_side: bool,
    pub terminal_notifications: bool,
    pub terminal_notifications_os: bool,
    pub notification_sound: bool,
    pub automatic_update_checks: bool,
    pub ntfy_enabled: bool,
    pub ntfy_channel: String,
    pub ntfy_machine: String,
    pub pr_metadata: bool,
    pub editor_mode: EditorMode,
    pub review_mode: ReviewMode,
    pub editor_program: String,
    pub external_editor: String,
    pub external_args: Vec<String>,
    /// Language-server executables keyed by protocol language id
    /// (`rust`, `python`). Empty or missing means automatic lookup.
    pub lsp_servers: std::collections::BTreeMap<String, String>,
    pub shell: String,
    pub history_days: u64,
    pub session_mib: u64,
    pub total_mib: u64,
    pub scrollback_lines: usize,
    pub font_size: f32,
    pub editor_close_timeout_secs: u64,
    pub diff_split_default: bool,
    pub native_vim: bool,
    /// Inline type and parameter-name hints from the language server.
    pub inlay_hints: bool,
    pub keybindings: std::collections::BTreeMap<String, String>,
    /// Disconnect a VNC session no viewer has watched for this many
    /// minutes; 0 keeps detached sessions connected.
    pub vnc_detached_timeout_minutes: u32,
}
mod error;
mod hooks;
mod settings;
pub use error::{SettingsError, StateError};

pub const METADATA_SETTINGS_CAPABILITY: &str = "metadata-settings-v1";
pub const WORKTREES_CAPABILITY: &str = "worktrees-v1";
pub const SHUTDOWN_IF_IDLE_CAPABILITY: &str = "shutdown-if-idle-v1";
pub const STABLE_HELPER_CAPABILITY: &str = "stable-helper-v1";
pub const SCREEN_CAPABILITY: &str = "screen-v1";
pub const TERMINAL_NOTICES_CAPABILITY: &str = "terminal-notices-v1";
pub const DIFF_CLOSE_SETTINGS_CAPABILITY: &str = "diff-close-settings-v1";
pub const NTFY_CAPABILITY: &str = "ntfy-v1";
pub const NOTIFICATION_SOUND_CAPABILITY: &str = "notification-sound-v1";
pub const AGENT_PRESENCE_CAPABILITY: &str = agents::CAPABILITY;
pub const VNC_CAPABILITY: &str = vnc::CAPABILITY;
pub const VNC_REGIONS_CAPABILITY: &str = vnc::REGIONS_CAPABILITY;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TerminalNotice {
    pub id: String,
    pub session_id: String,
    pub title: String,
    pub body: String,
    pub created: u64,
    pub dismissed: bool,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    /// Local async-client observation order; never sent or persisted.
    #[serde(skip)]
    pub client_observation: u64,
    pub catalog_revision: u64,
    pub generations: Vec<generations::Health>,
    pub daemon_version: Option<String>,
    pub daemon_build: Option<String>,
    pub daemon_catalog_version: Option<u32>,
    /// Runtime health; absent on older daemons. Refreshed before snapshots.
    pub daemon_executable: Option<PathBuf>,
    /// Private executable pinned for the lifetime of this daemon, when supported.
    pub attachment_helper_executable: Option<PathBuf>,
    pub attachment_helper_available: Option<bool>,
    /// Features advertised by the running daemon, not the GUI binary on disk.
    pub capabilities: Vec<String>,
    pub revision: u64,
    pub generation: String,
    pub projects: Vec<Project>,
    pub remote_projects: Vec<crate::remote::RemoteProject>,
    pub worktrees: Vec<worktrees::Registration>,
    pub sessions: Vec<Session>,
    #[serde(default)]
    pub vnc_profiles: Vec<vnc::VncProfile>,
    #[serde(default)]
    pub vnc_sessions: Vec<vnc::VncSession>,
    pub agents: Vec<Agent>,
    pub notifications: Vec<Notification>,
    pub terminal_notices: Vec<TerminalNotice>,
    /// Live agent-presence observations. Additive, never persisted, never a
    /// lifecycle source; absent on older daemons.
    #[serde(default)]
    pub presence: Vec<agents::TerminalPresence>,
    pub settings: Settings,
    pub recent_events: Vec<String>,
    pub selected_project: Option<String>,
    pub degraded: Option<String>,
}
impl State {
    /// True when at least one agent left a provider resume command for this
    /// session (e.g. `codex resume <id>`).
    #[must_use]
    pub fn session_has_resume(&self, session: &str) -> bool {
        self.agents
            .iter()
            .any(|a| a.session_id == session && a.resumable())
    }
    /// Drop ended/interrupted sessions that have no resume handle, along with
    /// their agent, notification, and terminal-notice records. Live sessions
    /// are always kept. Returns the removed session ids so callers can also
    /// delete their stored scrollback.
    pub fn prune_non_resumable_ended(&mut self) -> Vec<String> {
        let removed: Vec<String> = self
            .sessions
            .iter()
            .filter(|s| !s.lifecycle.live() && !self.session_has_resume(&s.id))
            .map(|s| s.id.clone())
            .collect();
        if removed.is_empty() {
            return removed;
        }
        self.sessions.retain(|s| !removed.contains(&s.id));
        self.agents.retain(|a| !removed.contains(&a.session_id));
        self.notifications
            .retain(|n| !removed.contains(&n.session_id));
        self.terminal_notices
            .retain(|n| !removed.contains(&n.session_id));
        self.presence.retain(|p| !removed.contains(&p.session_id));
        self.revision = self.revision.saturating_add(1);
        removed
    }
    /// Drop transient records that only matter to a live owner so a historical
    /// state stays small when it is loaded for aggregation. Pending attention
    /// is never dropped; only already dismissed/resolved notifications beyond
    /// the retention cap are removed. Sessions and resumable agents are kept.
    pub fn compact_history(&mut self) {
        self.recent_events.clear();
        self.presence.clear();
        let len = self.notifications.len();
        if len > MAX_NOTIFICATIONS {
            let cutoff = len.saturating_sub(MAX_NOTIFICATIONS);
            let mut index = 0;
            self.notifications.retain(|n| {
                let keep = index >= cutoff || !(n.dismissed || n.resolved);
                index = index.saturating_add(1);
                keep
            });
        }
        let len = self.agents.len();
        if len > MAX_AGENTS {
            let cutoff = len.saturating_sub(MAX_AGENTS);
            let mut index = 0;
            self.agents.retain(|a| {
                // Keep live/resumable agents and the most recent records; a
                // resumable handle must never be trimmed.
                let keep = index >= cutoff || a.resumable() || a.state == AgentState::Running;
                index = index.saturating_add(1);
                keep
            });
        }
    }
    /// Resolve from the already-loaded inventory; safe to use in GUI rendering.
    #[must_use]
    pub fn session_paths(&self, fallback: &Paths, session: &str) -> Paths {
        self.sessions
            .iter()
            .find(|s| s.id == session)
            .and_then(|s| self.generations.iter().find(|g| g.owner.id == s.generation))
            .map_or_else(|| fallback.clone(), |g| g.owner.paths())
    }
    /// Terminal messages are untrusted UI notices, never agent lifecycle events.
    pub fn terminal_notice(
        &mut self,
        session: &str,
        title: &str,
        body: &str,
    ) -> Result<Option<String>, StateError> {
        if !self
            .sessions
            .iter()
            .any(|s| s.id == session && s.lifecycle.live())
        {
            return Err(StateError::UnknownLiveSession {
                session: session.to_owned(),
            });
        }
        if !self.settings.terminal_notifications {
            return Ok(None);
        }
        let clean = |text: &str, limit| {
            text.chars()
                .filter(|c| !c.is_control() || *c == '\n')
                .take(limit)
                .collect::<String>()
        };
        let title = clean(title, 256);
        let body = clean(body, 1024);
        if title.trim().is_empty() && body.trim().is_empty() {
            return Ok(None);
        }
        if self.terminal_notices.iter().rev().take(32).any(|n| {
            n.session_id == session
                && n.title == title
                && n.body == body
                && now().saturating_sub(n.created) < 2
        }) {
            return Ok(None);
        }
        let id = id();
        self.terminal_notices.push(TerminalNotice {
            id: id.clone(),
            session_id: session.into(),
            title,
            body,
            created: now(),
            dismissed: false,
        });
        if self.terminal_notices.len() > 128 {
            self.terminal_notices
                .drain(..self.terminal_notices.len().saturating_sub(128));
        }
        self.revision = self.revision.saturating_add(1);
        Ok(Some(id))
    }
    pub fn recover(&mut self) {
        self.generation = id();
        // Runtime health must be re-evaluated by this daemon. Historical loss
        // remains on each session's truncated flag, not a stale queue warning.
        self.degraded = None;
        self.daemon_executable = None;
        self.attachment_helper_executable = None;
        self.attachment_helper_available = None;
        for s in &mut self.sessions {
            if s.lifecycle.live() {
                s.lifecycle = Lifecycle::Interrupted;
                s.pid = None;
            }
        }
        for a in &mut self.agents {
            if !matches!(
                a.state,
                AgentState::Completed | AgentState::Failed | AgentState::Stopped
            ) {
                a.state = AgentState::Unknown;
            }
            // Verified process links do not survive a daemon restart; the
            // inspector re-observes live processes from scratch.
            a.process = None;
        }
        // Presence is live observation only; a restart clears it.
        self.presence.clear();
        self.vnc_sessions.clear();
        self.revision = self.revision.saturating_add(1);
    }
    pub fn focus(&mut self, session: &str) {
        if self.settings.dismissal == Dismissal::OnFocus {
            for n in &mut self.notifications {
                if n.session_id == session {
                    n.dismissed = true;
                    n.read = true;
                }
            }
            self.revision = self.revision.saturating_add(1);
        }
    }
}

#[cfg(test)]
mod tests;
