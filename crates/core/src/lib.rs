//! Versioned local protocol and persistent, renderer-independent models.
pub mod appearance;
pub mod idle_close;
pub mod metadata;
pub mod snapshot;
pub mod ui_control;
pub mod worktrees;
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    os::unix::{
        fs::{OpenOptionsExt, PermissionsExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const PROTOCOL_VERSION: u32 = 1;
pub const NVIM_REVIEW_CAPABILITY: &str = "nvim-review-v1";
pub const MAX_FRAME: usize = 8 * 1024 * 1024;
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
/// Find an executable without invoking a shell. Include standard GUI-launch paths.
pub fn find_executable(program: &str) -> Option<PathBuf> {
    let mut paths = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    paths.extend(["/opt/homebrew/bin", "/usr/local/bin", "/bin", "/usr/bin"].map(PathBuf::from));
    if let Some(base) = directories::BaseDirs::new() {
        paths.extend([".local/bin", ".opencode/bin", ".grok/bin"].map(|p| base.home_dir().join(p)));
    }
    find_executable_in(program, &paths)
}
fn find_executable_in(program: &str, paths: &[PathBuf]) -> Option<PathBuf> {
    let candidates = if program.contains('/') {
        vec![PathBuf::from(program)]
    } else {
        paths.iter().map(|p| p.join(program)).collect()
    };
    candidates.into_iter().find(|p| {
        p.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    })
}
/// Check the current user's ability to execute a regular file, including ACLs.
pub fn executable_available(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    path.is_file()
        && std::ffi::CString::new(path.as_os_str().as_bytes())
            .is_ok_and(|p| unsafe { libc::access(p.as_ptr(), libc::X_OK) == 0 })
}
pub fn default_shell() -> Result<PathBuf> {
    ["zsh", "bash", "sh"]
        .into_iter()
        .find_map(find_executable)
        .context("No zsh, bash, or sh executable found")
}
pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[derive(Clone, Debug)]
pub struct Paths {
    pub data: PathBuf,
    pub runtime: PathBuf,
}
impl Paths {
    pub fn discover() -> Result<Self> {
        if let Some(path) = std::env::var_os("TERMINATOR_DATA_DIR") {
            let mut paths = Self::at(PathBuf::from(path));
            if let Some(runtime) = std::env::var_os("TERMINATOR_RUNTIME_DIR") {
                paths.runtime = runtime.into();
            }
            return Ok(paths);
        }
        let dirs = directories::ProjectDirs::from("dev", "terminator", "Terminator")
            .context("No user data directory")?;
        // Unix sockets have short path limits, particularly on macOS.
        let home = directories::BaseDirs::new().context("No home directory")?;
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        home.home_dir().hash(&mut h);
        Ok(Self {
            data: dirs.data_local_dir().into(),
            runtime: PathBuf::from(format!("/tmp/terminator-{:x}", h.finish())),
        })
    }
    pub fn at(data: PathBuf) -> Self {
        Self {
            runtime: data.join("run"),
            data,
        }
    }
    pub fn init(&self) -> Result<()> {
        for path in [&self.data, &self.runtime, &self.history_dir()] {
            if path.exists() {
                ensure!(
                    !fs::symlink_metadata(path)?.file_type().is_symlink(),
                    "Refusing symlink data directory"
                );
            }
            fs::create_dir_all(path)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
        ensure!(
            self.socket().as_os_str().len() < 100,
            "Runtime path too long for Unix socket; use a shorter data directory"
        );
        Ok(())
    }
    pub fn socket(&self) -> PathBuf {
        self.runtime.join("daemon.sock")
    }
    pub fn auth(&self) -> PathBuf {
        self.runtime.join("auth")
    }
    pub fn history_dir(&self) -> PathBuf {
        self.data.join("history")
    }
    pub fn editor_socket(&self, session: &str) -> PathBuf {
        self.runtime
            .join(format!("{}.nvim", &session[..8.min(session.len())]))
    }
    pub fn token(&self) -> Result<String> {
        Ok(fs::read_to_string(self.auth())?.trim().into())
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Missing parent directory")?;
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(".terminator-{}.tmp", id()));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    Running,
    Stopping,
    Ended,
    Interrupted,
}
impl Lifecycle {
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
#[derive(Clone, Debug, Serialize, Deserialize)]
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
#[derive(Clone, Debug, Serialize, Deserialize)]
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
    pub fn actionable(self) -> bool {
        matches!(
            self,
            Self::WaitingInput | Self::WaitingPermission | Self::Completed | Self::Failed
        )
    }
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
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Resume {
    pub program: String,
    pub args: Vec<String>,
}
impl Resume {
    pub fn display(&self) -> String {
        std::iter::once(&self.program)
            .chain(self.args.iter())
            .map(|s| quote(s))
            .collect::<Vec<_>>()
            .join(" ")
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Agent {
    pub invocation_id: String,
    pub session_id: String,
    pub kind: String,
    pub provider_session_id: Option<String>,
    pub state: AgentState,
    pub sequence: Option<u64>,
    pub updated: u64,
    pub resume: Option<Resume>,
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
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewMode {
    #[default]
    Native,
    Neovim,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub events: BTreeSet<AgentState>,
    pub os_events: BTreeSet<AgentState>,
    pub dismissal: Dismissal,
    pub notifications_side: bool,
    pub terminal_notifications: bool,
    pub terminal_notifications_os: bool,
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
            keybindings: [
                ("new_terminal".into(), "command+T".into()),
                ("open_file".into(), "command+O".into()),
                ("split_right".into(), "command+shift+D".into()),
                ("split_down".into(), "command+alt+D".into()),
                ("next_pane".into(), "command+]".into()),
            ]
            .into(),
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<()> {
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
        Ok(())
    }
}

pub const METADATA_SETTINGS_CAPABILITY: &str = "metadata-settings-v1";
pub const WORKTREES_CAPABILITY: &str = "worktrees-v1";
pub const SHUTDOWN_IF_IDLE_CAPABILITY: &str = "shutdown-if-idle-v1";
pub const STABLE_HELPER_CAPABILITY: &str = "stable-helper-v1";
pub const SCREEN_CAPABILITY: &str = "screen-v1";
pub const TERMINAL_NOTICES_CAPABILITY: &str = "terminal-notices-v1";
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
    pub daemon_version: Option<String>,
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
    pub settings: Settings,
    pub recent_events: Vec<String>,
    pub selected_project: Option<String>,
    pub degraded: Option<String>,
}
impl State {
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
                .drain(..self.terminal_notices.len() - 128);
        }
        self.revision += 1;
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
        }
        self.revision += 1;
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
            let a = &self.agents[i];
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
        if self.recent_events.len() > 8192 {
            self.recent_events.drain(..4096);
        }
        let repeated_state =
            existing.is_some_and(|i| self.agents[i].state == e.state) && e.request_id.is_none();
        let mut agent = existing.map(|i| self.agents[i].clone()).unwrap_or(Agent {
            invocation_id: e.agent_invocation_id.clone(),
            session_id: e.terminal_session_id.clone(),
            kind: e.agent_kind.clone(),
            provider_session_id: None,
            state: AgentState::Unknown,
            sequence: None,
            updated: now(),
            resume: None,
        });
        agent.state = e.state;
        agent.sequence = e.sequence;
        agent.updated = now();
        if e.provider_session_id.is_some() {
            agent.provider_session_id = e.provider_session_id.clone();
        }
        if e.resume.is_some() {
            agent.resume = e.resume.clone();
        }
        if let Some(i) = existing {
            self.agents[i] = agent;
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
        self.revision += 1;
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
        if self.notifications.len() > 10_000
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
            self.revision += 1;
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Request {
    CloseIdleSessions {
        generation: String,
        sessions: Vec<String>,
    },
    ShellCommand {
        session: String,
    },
    ShellPrompt {
        session: String,
        generation: u64,
        jobs_empty: bool,
    },
    WorktreeList {
        project: String,
    },
    WorktreeAdd {
        project: String,
        path: PathBuf,
        branch: Option<String>,
        start: String,
    },
    WorktreeRemove {
        project: String,
    },
    Screen {
        session: String,
    },
    TerminalNotify {
        session: String,
        title: String,
        body: String,
    },
    DismissTerminalNotice {
        id: String,
    },
    Snapshot,
    CreateReview {
        project: String,
        cwd: PathBuf,
        path: PathBuf,
        staged: bool,
    },
    AddProject {
        path: PathBuf,
    },
    SaveLayout {
        project: String,
        layout: serde_json::Value,
    },
    SelectProject {
        project: String,
    },
    Create {
        project: String,
        cwd: Option<PathBuf>,
        file: Option<PathBuf>,
        line: Option<u32>,
        #[serde(default)]
        column: Option<u32>,
        editor: bool,
    },
    Stop {
        session: String,
    },
    Rename {
        session: String,
        label: String,
    },
    Remove {
        session: String,
    },
    Focus {
        session: String,
    },
    Notice {
        id: String,
        action: String,
    },
    Settings(Settings),
    Heartbeat {
        focused: bool,
    },
    Hook(HookEvent),
    Cwd {
        session: String,
        path: PathBuf,
    },
    Attach {
        session: String,
        rows: u16,
        cols: u16,
    },
    Input {
        data: String,
    },
    Resize {
        rows: u16,
        cols: u16,
    },
    History {
        session: String,
    },
    ClearHistory {
        session: Option<String>,
    },
    EditorCompare {
        session: String,
    },
    EditorSave {
        session: String,
    },
    EditorStatus {
        session: String,
    },
    Shutdown,
    ShutdownIfIdle,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Envelope {
    pub version: u32,
    pub auth: String,
    pub request: Request,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_hint: Option<SnapshotHint>,
    /// Client support on the existing Snapshot request; old daemons ignore it.
    #[serde(default)]
    pub snapshot_chunks: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Response {
    IdleSessionsClosed(Vec<idle_close::Outcome>),
    SnapshotChunk { data: String, last: bool },
    Worktrees(Vec<worktrees::GitWorktree>),
    Unchanged,
    Ok,
    State(Box<State>),
    Created(Session),
    Text(String),
    Data(String),
    End,
    Error(String),
}
impl Response {
    pub fn checked(self) -> Result<Self> {
        if let Self::Error(e) = self {
            bail!("{e}")
        } else {
            Ok(self)
        }
    }
}
pub fn write_frame<T: Serialize>(writer: &mut impl Write, value: &T) -> Result<()> {
    let data = serde_json::to_vec(value)?;
    ensure!(data.len() <= MAX_FRAME, "IPC frame too large");
    writer.write_all(&(data.len() as u32).to_be_bytes())?;
    writer.write_all(&data)?;
    writer.flush()?;
    Ok(())
}
pub fn read_frame<T: DeserializeOwned>(reader: &mut impl Read) -> Result<T> {
    let mut header = [0; 4];
    reader.read_exact(&mut header)?;
    let n = u32::from_be_bytes(header) as usize;
    ensure!(n <= MAX_FRAME, "IPC frame too large");
    let mut bytes = vec![0; n];
    reader.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}
pub fn connect(paths: &Paths, request: Request, token: Option<String>) -> Result<UnixStream> {
    connect_hint(paths, request, token, None)
}
fn connect_hint(
    paths: &Paths,
    request: Request,
    token: Option<String>,
    snapshot_hint: Option<SnapshotHint>,
) -> Result<UnixStream> {
    let mut s = UnixStream::connect(paths.socket()).context("Session daemon unavailable")?;
    s.set_read_timeout(Some(Duration::from_secs(
        if matches!(&request, Request::CloseIdleSessions { .. }) {
            30
        } else {
            3
        },
    )))?;
    s.set_write_timeout(Some(Duration::from_secs(3)))?;
    write_frame(
        &mut s,
        &Envelope {
            version: PROTOCOL_VERSION,
            auth: match token {
                Some(t) => t,
                None => paths.token()?,
            },
            snapshot_chunks: matches!(request, Request::Snapshot),
            request,
            snapshot_hint,
        },
    )?;
    Ok(s)
}
pub fn rpc(paths: &Paths, request: Request) -> Result<Response> {
    let mut s = connect(paths, request, None)?;
    snapshot::read_response(&mut s)?.checked()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotHint {
    pub generation: String,
    pub revision: u64,
}
impl State {
    pub fn snapshot_hint(&self) -> SnapshotHint {
        SnapshotHint {
            generation: self.generation.clone(),
            revision: self.revision,
        }
    }
}
pub fn conditional_snapshot(paths: &Paths, hint: Option<SnapshotHint>) -> Result<Response> {
    let mut stream = connect_hint(paths, Request::Snapshot, None, hint)?;
    snapshot::read_response(&mut stream)?.checked()
}

/// Docking libraries use infinite rectangles before their first layout pass. JSON
/// encodes those as null; replace only coordinate placeholders, never optional IDs.
pub fn sanitize_layout(mut value: serde_json::Value) -> serde_json::Value {
    fn walk(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, value) in map {
                    if (key == "x" || key == "y") && value.is_null() {
                        *value = serde_json::json!(0.0);
                    } else {
                        walk(value);
                    }
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    walk(item)
                }
            }
            _ => {}
        }
    }
    walk(&mut value);
    value
}

pub mod process;
pub use process::{CommandOptions, run_command, spawn_session_leader};
/// Legacy callers inspect the exit status themselves.
pub fn bounded_output(
    cmd: std::process::Command,
    timeout: Duration,
) -> Result<std::process::Output> {
    run_command(
        cmd,
        CommandOptions {
            timeout,
            accepted_exit_codes: None,
            ..Default::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_preference_skips_missing_and_nonexecutable_files() {
        let dir = tempfile::tempdir().unwrap();
        let paths = vec![dir.path().to_path_buf()];
        let choose = || {
            ["zsh", "bash", "sh"]
                .into_iter()
                .find_map(|name| find_executable_in(name, &paths))
        };
        assert!(choose().is_none());
        for name in ["sh", "bash", "zsh"] {
            let p = dir.path().join(name);
            fs::write(&p, b"#!/bin/sh\n").unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o700)).unwrap();
            assert_eq!(choose(), Some(p));
        }
        fs::set_permissions(dir.path().join("zsh"), fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(choose(), Some(dir.path().join("bash")));
        fs::remove_file(dir.path().join("bash")).unwrap();
        assert_eq!(choose(), Some(dir.path().join("sh")));
    }
    fn setup() -> State {
        let mut s = State::default();
        s.sessions.push(Session {
            review: false,
            id: "s".into(),
            project_id: "p".into(),
            label: "shell".into(),
            cwd: "/tmp".into(),
            kind: SessionKind::Shell,
            file: None,
            lifecycle: Lifecycle::Running,
            created: 0,
            exit_code: None,
            rows: 24,
            cols: 80,
            generation: id(),
            pid: None,
            truncated: false,
            cwd_confirmed: true,
        });
        s
    }
    #[test]
    fn terminal_notifications_never_create_or_change_agents_and_are_bounded() {
        let mut state = setup();
        state
            .terminal_notice("s", "Terminal title", "permission needed")
            .unwrap();
        assert!(state.agents.is_empty());
        assert!(state.notifications.is_empty());
        assert!(
            state
                .terminal_notice("s", "Terminal title", "permission needed")
                .unwrap()
                .is_none()
        );
        assert!(state.terminal_notice("missing", "test", "test").is_err());
        for i in 0..200 {
            state
                .terminal_notice("s", &format!("{i}"), "hello")
                .unwrap();
        }
        assert_eq!(state.terminal_notices.len(), 128);
    }
    fn event(n: u64, state: AgentState) -> HookEvent {
        HookEvent {
            protocol_version: 1,
            event_id: format!("e{n}"),
            terminal_session_id: "s".into(),
            agent_invocation_id: "a".into(),
            agent_kind: "custom".into(),
            provider_session_id: None,
            state,
            request_id: Some("req".into()),
            sequence: Some(n),
            summary: "Need input".into(),
            details: String::new(),
            resume: None,
        }
    }
    #[test]
    fn dismissal_does_not_resume_agent() {
        let mut s = setup();
        s.apply_hook(event(1, AgentState::WaitingInput)).unwrap();
        s.focus("s");
        assert!(s.notifications[0].dismissed);
        assert_eq!(s.agents[0].state, AgentState::WaitingInput);
        s.apply_hook(event(1, AgentState::WaitingInput)).unwrap();
        assert_eq!(s.notifications.len(), 1);
    }
    #[test]
    fn stale_events_and_other_invocations() {
        let mut s = setup();
        s.apply_hook(event(3, AgentState::Running)).unwrap();
        s.apply_hook(event(2, AgentState::WaitingPermission))
            .unwrap();
        assert!(s.notifications.is_empty());
        let mut e = event(4, AgentState::Completed);
        e.agent_invocation_id = "old-agent".into();
        s.apply_hook(e).unwrap();
        assert_eq!(s.agents[0].state, AgentState::Running);
    }
    #[test]
    fn recovery_does_not_relaunch() {
        let mut s = setup();
        s.recover();
        assert_eq!(s.sessions[0].lifecycle, Lifecycle::Interrupted);
        assert!(s.sessions[0].pid.is_none());
    }
    #[test]
    fn recovery_preserves_history_loss_but_discards_previous_runtime_health() {
        let mut state = setup();
        state.sessions[0].truncated = true;
        state.degraded = Some("Output storage queue saturated; some history was not saved".into());
        state.daemon_executable = Some("/removed/daemon".into());
        state.attachment_helper_executable = Some("/removed/helper".into());
        state.attachment_helper_available = Some(false);
        state.recover();
        assert!(state.sessions[0].truncated);
        assert!(state.degraded.is_none());
        assert!(state.daemon_executable.is_none());
        assert!(state.attachment_helper_executable.is_none());
        assert!(state.attachment_helper_available.is_none());
    }
    #[test]
    fn oversized_frame_rejected_before_allocation() {
        let mut b = (MAX_FRAME as u32 + 1).to_be_bytes().as_slice().to_vec();
        assert!(read_frame::<Request>(&mut b.as_slice()).is_err());
        b.clear();
    }
    #[test]
    fn shell_quote_is_literal() {
        assert_eq!(quote("a'b$(x)"), "'a'\\''b$(x)'");
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    #[test]
    fn envelopes_remain_compatible_in_both_directions() {
        #[derive(Deserialize)]
        struct LegacyEnvelope {
            version: u32,
            auth: String,
            request: Request,
        }
        let hint = State::default().snapshot_hint();
        let new = Envelope {
            version: 1,
            auth: "fixture".into(),
            request: Request::Snapshot,
            snapshot_hint: Some(hint),
            snapshot_chunks: true,
        };
        let legacy: LegacyEnvelope =
            serde_json::from_value(serde_json::to_value(&new).unwrap()).unwrap();
        assert_eq!(legacy.version, 1);
        assert_eq!(legacy.auth, "fixture");
        assert!(matches!(legacy.request, Request::Snapshot));
        let new: Envelope = serde_json::from_value(
            serde_json::json!({"version":1,"auth":"fixture","request":"Snapshot"}),
        )
        .unwrap();
        assert!(new.snapshot_hint.is_none());
        assert!(!new.snapshot_chunks);
    }
    #[test]
    fn legacy_snapshots_do_not_advertise_new_daemon_features() {
        let legacy: State = serde_json::from_value(serde_json::json!({"revision":7})).unwrap();
        assert!(legacy.capabilities.is_empty());
        assert!(legacy.daemon_version.is_none());
        let new = State {
            capabilities: vec![NVIM_REVIEW_CAPABILITY.into()],
            ..State::default()
        };
        let encoded = serde_json::to_value(&new).unwrap();
        #[derive(Deserialize)]
        struct LegacyState {
            revision: u64,
        }
        assert_eq!(
            serde_json::from_value::<LegacyState>(encoded.clone())
                .unwrap()
                .revision,
            0
        );
        assert_eq!(
            serde_json::from_value::<State>(encoded)
                .unwrap()
                .capabilities,
            new.capabilities
        );
    }
    #[test]
    fn restarting_changes_hint_even_if_revision_is_equal() {
        let mut state = State::default();
        state.recover();
        let previous = state.snapshot_hint();
        state.recover();
        state.revision = previous.revision;
        assert_ne!(previous, state.snapshot_hint());
    }
}

#[cfg(test)]
mod executable_health_tests {
    use super::*;
    #[test]
    fn missing_non_executable_and_directory_helpers_are_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("helper");
        assert!(!executable_available(&file));
        assert!(!executable_available(dir.path()));
        fs::write(&file, "#!/bin/sh\nexit 0\n").unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(!executable_available(&file));
        fs::set_permissions(&file, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(executable_available(&file));
        fs::remove_file(&file).unwrap();
        assert!(!executable_available(&file));
    }
}
