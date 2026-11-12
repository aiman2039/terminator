use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    io::{Read, Write},
    path::PathBuf,
    time::Duration,
};

use super::{HookEvent, MAX_FRAME, PROTOCOL_VERSION, Paths, Session, Settings, State};
use crate::{generations, idle_close, remote_apps, snapshot, transport, vnc, worktrees};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Request {
    VncProfileSave {
        profile: vnc::VncProfile,
        #[serde(default)]
        forget_password: bool,
    },
    VncProfileRemove {
        id: String,
    },
    VncForgetCredential {
        id: String,
    },
    VncTransfer {
        session: String,
        transfer: vnc::VncTransfer,
    },
    VncTransferStatus {
        session: String,
        id: String,
    },
    VncConnect {
        project: String,
        #[serde(default)]
        options: vnc::VncOptions,
        #[serde(default)]
        profile_id: Option<String>,
        host: String,
        port: u16,
        #[serde(default)]
        username: String,
        #[serde(default)]
        password: String,
        #[serde(default)]
        shared: bool,
        #[serde(default)]
        view_only: bool,
        #[serde(default)]
        label: Option<String>,
    },
    VncDisconnect {
        session: String,
    },
    VncInput {
        session: String,
        events: Vec<vnc::VncInputEvent>,
    },
    VncFrame {
        session: String,
        since: u64,
        /// Ask for incremental updates. Old daemons ignore the field, and
        /// the reply's `region` decides whether the pixels are partial.
        #[serde(default)]
        regions: bool,
        /// Shared-memory frame and long poll; only sent to daemons that
        /// advertise `vnc-shm-v1`. Older daemons ignore it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        shm: Option<vnc::VncShmRequest>,
    },
    /// Start (or reuse) the remote-apps session of a remote project
    /// (`remote-apps-v1`). Answers [`Response::RemoteApps`].
    RemoteAppsConnect {
        project: String,
    },
    RemoteAppsDisconnect {
        session: String,
    },
    /// At most [`remote_apps::MAX_INPUT`] messages.
    RemoteAppsInput {
        session: String,
        messages: Vec<remote_apps::ClientMessage>,
    },
    /// Offers `text` (at most [`remote_apps::MAX_CLIPBOARD`] bytes) to the
    /// remote apps as their clipboard, then sends `then` (the paste key),
    /// in that order.
    RemoteAppsClipboard {
        session: String,
        text: String,
        #[serde(default)]
        then: Vec<remote_apps::ClientMessage>,
    },
    /// The session's windows; waits up to `wait_ms` (capped at
    /// [`remote_apps::MAX_WAIT_MS`]) for a revision newer than `since`
    /// before answering [`Response::Unchanged`].
    RemoteAppsState {
        session: String,
        since: u64,
        #[serde(default)]
        wait_ms: u32,
    },
    PruneHistory {
        budget: u64,
    },
    Archived {
        generation: String,
        request: Box<Request>,
    },
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
    /// Shell integration blocks with `id > since`, oldest first. Replies
    /// `Response::CommandBlocks`. Requires `command-blocks-v1`.
    CommandBlocks {
        session: String,
        #[serde(default)]
        since: u64,
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
    /// The one live session whose shell PID is in `pids`: `Text(id)`, or `Ok`
    /// when none or several match. Sent only by hooks; older daemons reject
    /// it and the hook falls back to `Snapshot` (`session-for-pids-v1`).
    SessionForPids {
        pids: Vec<u32>,
    },
    CreateReview {
        project: String,
        cwd: PathBuf,
        path: PathBuf,
        staged: bool,
    },
    AddRemoteProject {
        project: crate::remote::RemoteProject,
    },
    RemoteProject {
        project: String,
        operation: crate::remote::Operation,
    },
    /// With root [`HOME_ROOT`](crate::remote::HOME_ROOT) the reply is the
    /// login directory (`ssh-hosts-v1`).
    ProbeRemoteProject {
        project: crate::remote::RemoteProject,
    },
    /// Forgets an SSH project. Refused while its sessions are live; stops
    /// its OpenSSH master unless another project or session shares it.
    /// Requires `ssh-hosts-v1`.
    RemoveRemoteProject {
        project: String,
    },
    /// Replaces the saved project with the same id: name, connection, root
    /// and group. Its layout, forwards and `last_connected` stay. Changing
    /// the connection or root is refused while the project has live
    /// sessions, transfers or active forwards. Requires `ssh-edit-v1`.
    UpdateRemoteProject {
        project: crate::remote::RemoteProject,
    },
    /// `None` or blank moves the project out of any group. Requires
    /// `ssh-hosts-v1`.
    SetRemoteProjectGroup {
        project: String,
        group: Option<String>,
    },
    /// Replaces the project's saved port forwards. Refused while it would
    /// drop or change the endpoints of an active forward. Requires
    /// `ssh-forwarding-v1`.
    SetRemoteProjectForwards {
        project: String,
        forwards: Vec<crate::remote::Forward>,
    },
    /// Starts the saved forward on `local_port` through the project's
    /// OpenSSH master, starting a batch master when none is alive. Replies
    /// `Response::Forwards`. Requires `ssh-forwarding-v1`.
    StartForward {
        project: String,
        local_port: u16,
    },
    /// Cancels a forward. Replies `Response::Forwards`. Requires
    /// `ssh-forwarding-v1`.
    StopForward {
        project: String,
        local_port: u16,
    },
    /// Active forwards whose master still answers, plus background start
    /// failures. Replies `Response::Forwards`. Requires `ssh-forwarding-v1`.
    ForwardStatus,
    /// Reports the keys `ssh.host` presents. Requires `ssh-hostkeys-v1`.
    ScanHostKey {
        ssh: crate::ssh::SshOptions,
    },
    /// Appends `keys` to `known_hosts` after a fresh scan still presents
    /// exactly them. Requires `ssh-hostkeys-v1`.
    TrustHostKey {
        ssh: crate::ssh::SshOptions,
        keys: Vec<crate::ssh::HostKey>,
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
    /// Set or clear (`None` or blank) a local project's init command
    /// (`project-init-v1`).
    SetInitCommand {
        project: String,
        command: Option<String>,
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
    RetireIfDraining,
    /// Snapshots and labels of one local file, newest first. Replies
    /// `Response::LocalHistory`. Requires `local-history-v1`.
    LocalHistoryList {
        path: PathBuf,
    },
    /// The text of one snapshot of `path`. Replies `Response::Text`.
    /// Requires `local-history-v1`.
    LocalHistoryRead {
        path: PathBuf,
        id: String,
    },
    /// Adds a timeline marker for every file of the project. Requires
    /// `local-history-v1`.
    LocalHistoryLabel {
        project: String,
        label: String,
    },
    /// Records the current content of `path`, then writes snapshot `id`
    /// back to it. Sent only on an explicit, confirmed user action.
    /// Requires `local-history-v1`.
    LocalHistoryRevert {
        path: PathBuf,
        id: String,
    },
    /// One page of `LocalHistoryList`, newest first. Replies
    /// `Response::LocalHistoryPage`. Requires `local-history-limits-v1`.
    LocalHistoryPage {
        path: PathBuf,
        offset: u64,
        limit: u64,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Envelope {
    pub version: u32,
    pub auth: String,
    pub request: Request,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_hint: Option<SnapshotHint>,
    /// Client support for chunked `Snapshot` and `VncFrame` responses; old daemons ignore it.
    #[serde(default)]
    pub snapshot_chunks: bool,
    /// Client understands `Response::SshError`; old daemons ignore it and
    /// old clients receive `Response::Error`.
    #[serde(default)]
    pub ssh_errors: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Response {
    Redirect {
        generation: String,
    },
    IdleSessionsClosed(Vec<idle_close::Outcome>),
    SnapshotChunk {
        data: String,
        last: bool,
    },
    Worktrees(Vec<worktrees::GitWorktree>),
    CommandBlocks(Vec<crate::command_blocks::CommandBlock>),
    VncCreated(vnc::VncSession),
    VncTransferStatus(vnc::VncTransferStatus),
    VncFrame(Box<vnc::VncFrame>),
    RemoteApps(Box<remote_apps::RemoteAppsState>),
    Unchanged,
    Ok,
    State(Box<State>),
    RemoteProject(crate::remote::Reply),
    /// Sent only to clients whose envelope sets `ssh_errors`.
    SshError(crate::ssh::SshError),
    HostKeys(crate::ssh::HostKeyScan),
    Forwards(crate::remote::ForwardStatus),
    Created(Session),
    Text(String),
    Data(String),
    End,
    Error(String),
    LocalHistory(Vec<crate::local_history::Entry>),
    LocalHistoryPage {
        entries: Vec<crate::local_history::Entry>,
        total: u64,
    },
}
impl Response {
    pub fn checked(self) -> Result<Self> {
        match self {
            Self::Redirect { generation } => Err(IpcError::Redirected { generation }),
            Self::Error(message) => Err(IpcError::Daemon { message }),
            Self::SshError(error) => Err(IpcError::Ssh(error)),
            response => Ok(response),
        }
    }
}
mod client;
mod error;
mod request_meta;
pub use error::{FrameError, IpcError};
type Result<T, E = IpcError> = std::result::Result<T, E>;
pub use client::{
    SnapshotHint, conditional_snapshot, connect, read_frame, rpc, sanitize_layout, write_frame,
};
#[cfg(feature = "async-client")]
pub(crate) use client::{archived_owner, redirect_allowed};

#[cfg(test)]
mod tests;
