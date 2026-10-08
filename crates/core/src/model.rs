use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::PathBuf};

use super::{
    MAX_AGENTS, MAX_NOTIFICATIONS, MAX_RECENT_EVENTS, PROTOCOL_VERSION, Paths, id, now, quote,
};
use crate::{agents, generations, worktrees};

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
    pub shell: String,
    pub history_days: u64,
    pub session_mib: u64,
    pub total_mib: u64,
    pub scrollback_lines: usize,
    pub font_size: f32,
    pub editor_close_timeout_secs: u64,
    pub diff_split_default: bool,
    pub native_vim: bool,
    pub keybindings: std::collections::BTreeMap<String, String>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            events: [
                AgentState::WaitingInput,
                AgentState::WaitingPermission,
                AgentState::Completed,
                AgentState::Failed,
            ]
            .into(),
            os_events: [
                AgentState::WaitingInput,
                AgentState::WaitingPermission,
                AgentState::Failed,
            ]
            .into(),
            dismissal: Dismissal::OnFocus,
            notifications_side: true,
            terminal_notifications: true,
            terminal_notifications_os: false,
            notification_sound: true,
            automatic_update_checks: true,
            ntfy_enabled: false,
            ntfy_channel: String::new(),
            ntfy_machine: String::new(),
            pr_metadata: false,
            editor_mode: EditorMode::Embedded,
            review_mode: ReviewMode::Native,
            editor_program: "nvim".into(),
            external_editor: if cfg!(target_os = "macos") {
                "open"
            } else {
                "xdg-open"
            }
            .into(),
            external_args: vec![],
            shell: String::new(),
            history_days: 30,
            session_mib: 50,
            total_mib: 2048,
            scrollback_lines: 10_000,
            font_size: 13.0,
            editor_close_timeout_secs: 1,
            diff_split_default: false,
            native_vim: true,
            keybindings: [
                ("new_terminal".into(), "command+T".into()),
                ("open_file".into(), "command+O".into()),
                ("split_up".into(), "command+alt+Up".into()),
                ("split_down".into(), "command+alt+D".into()),
                ("split_left".into(), "command+shift+Left".into()),
                ("split_right".into(), "command+shift+D".into()),
                ("next_pane".into(), "command+]".into()),
                ("select_all".into(), "command+A".into()),
                ("find_in_terminal".into(), "command+F".into()),
                ("search_scrollback".into(), "command+alt+F".into()),
                ("copy_working_directory".into(), "command+alt+C".into()),
                ("rename_terminal".into(), "command+shift+R".into()),
                ("close_session".into(), "command+W".into()),
                ("clear_scrollback".into(), "command+shift+K".into()),
                ("editor_save".into(), "command+S".into()),
                ("compare_disk".into(), "command+shift+Y".into()),
                ("toggle_left_sidebar".into(), "command+B".into()),
                ("toggle_right_sidebar".into(), "command+L".into()),
                ("toggle_ide_mode".into(), "command+E".into()),
                ("move_to_main".into(), "command+alt+M".into()),
                ("move_to_strip".into(), "command+alt+Down".into()),
                ("open_settings".into(), "command+,".into()),
                ("open_palette".into(), "command+P".into()),
                ("next_attention".into(), "command+shift+J".into()),
            ]
            .into(),
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
        // Disabled ntfy accepts any channel/machine contents (including
        // leftovers) so turning the toggle off always saves. Enabling
        // requires a usable channel again, and delivery still re-validates.
        if self.ntfy_enabled {
            ensure!(
                (1..=128).contains(&self.ntfy_channel.len())
                    && self
                        .ntfy_channel
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
                "ntfy channel must contain 1–128 letters, digits, underscores or hyphens when enabled"
            );
            ensure!(
                self.ntfy_machine.len() <= 128 && !self.ntfy_machine.chars().any(char::is_control),
                "ntfy machine name must be at most 128 bytes without control characters"
            );
        }
        ensure!(
            (1..=3650).contains(&self.history_days),
            "History age must be 1–3650 days"
        );
        ensure!(
            (1..=4096).contains(&self.session_mib)
                && self.total_mib >= self.session_mib
                && self.total_mib <= 65536,
            "Invalid disk limits"
        );
        ensure!(
            (100..=20_000).contains(&self.scrollback_lines),
            "History lines must be 100–20000"
        );
        ensure!(
            (9.0..=32.0).contains(&self.font_size),
            "Font size must be 9–32"
        );
        ensure!(
            (1..=30).contains(&self.editor_close_timeout_secs),
            "Editor close timeout must be 1–30 seconds"
        );
        Ok(())
    }
}

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
    pub worktrees: Vec<worktrees::Registration>,
    pub sessions: Vec<Session>,
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
    ) -> Result<Option<String>> {
        ensure!(
            self.sessions
                .iter()
                .any(|s| s.id == session && s.lifecycle.live()),
            "Unknown live session"
        );
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
        self.revision = self.revision.saturating_add(1);
    }
    pub fn apply_hook(&mut self, e: HookEvent) -> Result<Option<String>> {
        ensure!(
            e.protocol_version == PROTOCOL_VERSION,
            "Unsupported hook version"
        );
        ensure!(
            self.sessions
                .iter()
                .any(|s| s.id == e.terminal_session_id && s.lifecycle.live()),
            "Unknown or ended session"
        );
        ensure!(
            !e.event_id.is_empty() && !e.agent_invocation_id.is_empty() && !e.agent_kind.is_empty(),
            "Missing event identity"
        );
        ensure!(
            e.summary.len() <= 4096
                && e.details.len() <= 65536
                && e.event_id.len() <= 256
                && e.agent_invocation_id.len() <= 512,
            "Hook too large"
        );
        if self.recent_events.contains(&e.event_id) {
            return Ok(None);
        }
        let existing = self.agents.iter().position(|a| {
            a.invocation_id == e.agent_invocation_id && a.session_id == e.terminal_session_id
        });
        if let Some(i) = existing {
            let Some(a) = self.agents.get(i) else {
                return Ok(None);
            };
            if a.state == AgentState::Stopped {
                return Ok(None);
            }
            if let (Some(old), Some(new)) = (a.sequence, e.sequence)
                && new <= old
            {
                return Ok(None);
            }
        }
        self.recent_events.push(e.event_id.clone());
        if self.recent_events.len() > MAX_RECENT_EVENTS {
            self.recent_events.drain(
                ..self
                    .recent_events
                    .len()
                    .saturating_sub(MAX_RECENT_EVENTS / 2),
            );
        }
        let repeated_state = existing
            .is_some_and(|i| self.agents.get(i).is_some_and(|a| a.state == e.state))
            && e.request_id.is_none();
        let mut agent = existing
            .and_then(|i| self.agents.get(i).cloned())
            .unwrap_or(Agent {
                invocation_id: e.agent_invocation_id.clone(),
                session_id: e.terminal_session_id.clone(),
                kind: e.agent_kind.clone(),
                provider_session_id: None,
                state: AgentState::Unknown,
                sequence: None,
                updated: now(),
                resume: None,
                // A new invocation never inherits an earlier invocation's process link.
                process: None,
            });
        agent.state = e.state;
        agent.sequence = e.sequence;
        agent.updated = now();
        // Link lifecycle to a detected process only after verifying the
        // helper-supplied identity against a fresh observation of the same
        // kind in the same terminal. Unlinked events keep existing delivery.
        if let Some(identity) = &e.process {
            let observed = now();
            if self.presence.iter().any(|p| {
                p.session_id == e.terminal_session_id
                    && p.outcome == agents::PresenceOutcome::Verified
                    && agents::observation_fresh(p.observed_at, observed)
                    && p.agents
                        .iter()
                        .any(|a| a.kind == e.agent_kind && a.process == *identity)
            }) {
                agent.process = Some(identity.clone());
            }
        }
        if e.provider_session_id.is_some() {
            agent.provider_session_id.clone_from(&e.provider_session_id);
        }
        if e.resume.is_some() {
            agent.resume.clone_from(&e.resume);
        }
        if let Some(i) = existing
            && let Some(slot) = self.agents.get_mut(i)
        {
            *slot = agent;
        } else {
            self.agents.push(agent);
        }
        if matches!(
            e.state,
            AgentState::Running | AgentState::Completed | AgentState::Failed | AgentState::Stopped
        ) {
            for n in &mut self.notifications {
                if n.invocation_id == e.agent_invocation_id
                    && n.session_id == e.terminal_session_id
                    && !n.resolved
                {
                    n.resolved = true;
                    if self.settings.dismissal == Dismissal::OnResolve {
                        n.dismissed = true;
                    }
                }
            }
        }
        self.revision = self.revision.saturating_add(1);
        if repeated_state || !e.state.actionable() || !self.settings.events.contains(&e.state) {
            return Ok(None);
        }
        if let Some(request) = &e.request_id
            && self.notifications.iter().any(|n| {
                n.session_id == e.terminal_session_id
                    && n.invocation_id == e.agent_invocation_id
                    && n.request_id.as_ref() == Some(request)
                    && n.state == e.state
            })
        {
            return Ok(None);
        }
        let nid = id();
        self.notifications.push(Notification {
            id: nid.clone(),
            session_id: e.terminal_session_id,
            invocation_id: e.agent_invocation_id,
            request_id: e.request_id,
            state: e.state,
            summary: e.summary,
            details: e.details,
            created: now(),
            read: false,
            dismissed: false,
            resolved: false,
            snoozed_until: 0,
        });
        // Bound completed history; never silently discard pending attention.
        if self.notifications.len() > MAX_NOTIFICATIONS
            && let Some(i) = self
                .notifications
                .iter()
                .position(|n| n.dismissed || n.resolved)
        {
            self.notifications.remove(i);
        }
        Ok(Some(nid))
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
mod tests {
    use super::*;

    #[test]
    fn new_settings_defaults_include_editor_close_timeout() {
        let s = Settings::default();
        assert_eq!(s.editor_close_timeout_secs, 1);
    }

    #[test]
    fn new_settings_defaults_include_diff_split_default() {
        let s = Settings::default();
        assert!(!s.diff_split_default);
    }

    #[test]
    fn settings_serialization_round_trips_new_fields() {
        let s = Settings {
            editor_close_timeout_secs: 5,
            diff_split_default: true,
            ..Settings::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        let restored: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.editor_close_timeout_secs, 5);
        assert!(restored.diff_split_default);
    }
    #[test]
    fn ntfy_settings_default_off_validate_and_round_trip() {
        let mut settings: Settings = serde_json::from_str("{}").unwrap();
        assert!(!settings.ntfy_enabled);
        settings.ntfy_enabled = true;
        assert!(settings.validate().is_err());
        settings.ntfy_channel = "phone_123-test".into();
        settings.ntfy_machine = "My laptop".into();
        settings.validate().unwrap();
        let restored: Settings =
            serde_json::from_slice(&serde_json::to_vec(&settings).unwrap()).unwrap();
        assert_eq!(restored, settings);
        for channel in ["https://ntfy.sh/topic", "a/b", "a?b", "a\nB", "has space"] {
            settings.ntfy_channel = channel.into();
            assert!(settings.validate().is_err());
        }
        settings.ntfy_channel = "valid".into();
        settings.ntfy_machine = "bad\nheader".into();
        assert!(settings.validate().is_err());
    }

    #[test]
    fn disabled_ntfy_accepts_any_channel_and_machine_contents() {
        // Turning the toggle off must always save, even with leftover or
        // cleared field contents. Re-enabling still requires valid values.
        let mut settings = Settings::default();
        settings.validate().unwrap();
        settings.ntfy_channel = "has space!".into();
        settings.ntfy_machine = "bad\nheader".into();
        settings.validate().unwrap();
        settings.ntfy_channel.clear();
        settings.ntfy_machine.clear();
        settings.validate().unwrap();
        settings.ntfy_enabled = true;
        assert!(settings.validate().is_err());
        settings.ntfy_channel = "ok-channel_1".into();
        settings.ntfy_machine = "My laptop".into();
        settings.validate().unwrap();
    }

    #[test]
    fn automatic_update_checks_default_on_and_round_trip() {
        assert!(Settings::default().automatic_update_checks);
        let parsed: Settings = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(parsed.automatic_update_checks);
        let off: Settings =
            serde_json::from_value(serde_json::json!({"automatic_update_checks": false})).unwrap();
        assert!(!off.automatic_update_checks);
        let encoded = serde_json::to_value(&off).unwrap();
        assert_eq!(encoded["automatic_update_checks"], false);
    }

    #[test]
    fn notification_sound_defaults_on_and_round_trips() {
        assert!(Settings::default().notification_sound);
        let parsed: Settings = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(parsed.notification_sound);
        let off: Settings =
            serde_json::from_value(serde_json::json!({"notification_sound": false})).unwrap();
        assert!(!off.notification_sound);
        let encoded = serde_json::to_value(&off).unwrap();
        assert_eq!(encoded["notification_sound"], false);
    }
}
