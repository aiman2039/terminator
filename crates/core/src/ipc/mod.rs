use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    io::{Read, Write},
    path::PathBuf,
    time::Duration,
};

use super::{HookEvent, MAX_FRAME, PROTOCOL_VERSION, Paths, Session, Settings, State};
use crate::{generations, idle_close, snapshot, transport, vnc, worktrees};

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
}
impl Request {
    /// How long a client waits for this request's response. Requests whose
    /// daemon work runs Git or SSH get longer than the 3 s default: a client
    /// that gives up early reports failure for work that then succeeds.
    #[must_use]
    pub fn response_deadline(&self) -> Duration {
        Duration::from_secs(match self {
            Self::RemoteProject { .. }
            | Self::ProbeRemoteProject { .. }
            | Self::ScanHostKey { .. }
            | Self::TrustHostKey { .. }
            | Self::StartForward { .. }
            | Self::StopForward { .. }
            | Self::ForwardStatus
            | Self::WorktreeAdd { .. }
            | Self::WorktreeRemove { .. } => 40,
            Self::VncFrame { .. } | Self::CloseIdleSessions { .. } => 30,
            // Writing the OS keychain can wait on a macOS permission prompt.
            Self::VncProfileSave { .. } | Self::VncForgetCredential { .. } => 120,
            _ => 3,
        })
    }
    /// Short stable name for timeouts and diagnostics. Never includes IDs,
    /// paths, or labels.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::VncProfileSave { .. } => "VncProfileSave",
            Self::VncProfileRemove { .. } => "VncProfileRemove",
            Self::VncConnect { .. } => "VncConnect",
            Self::VncForgetCredential { .. } => "VncForgetCredential",
            Self::VncTransfer { .. } => "VncTransfer",
            Self::VncTransferStatus { .. } => "VncTransferStatus",
            Self::VncDisconnect { .. } => "VncDisconnect",
            Self::VncInput { .. } => "VncInput",
            Self::VncFrame { .. } => "VncFrame",
            Self::PruneHistory { .. } => "PruneHistory",
            Self::Archived { request, .. } => request.name(),
            Self::CloseIdleSessions { .. } => "CloseIdleSessions",
            Self::ShellCommand { .. } => "ShellCommand",
            Self::ShellPrompt { .. } => "ShellPrompt",
            Self::WorktreeList { .. } => "WorktreeList",
            Self::WorktreeAdd { .. } => "WorktreeAdd",
            Self::WorktreeRemove { .. } => "WorktreeRemove",
            Self::Screen { .. } => "Screen",
            Self::TerminalNotify { .. } => "TerminalNotify",
            Self::DismissTerminalNotice { .. } => "DismissTerminalNotice",
            Self::Snapshot => "Snapshot",
            Self::CreateReview { .. } => "CreateReview",
            Self::AddProject { .. } => "AddProject",
            Self::AddRemoteProject { .. } => "AddRemoteProject",
            Self::RemoteProject { .. } => "RemoteProject",
            Self::ProbeRemoteProject { .. } => "ProbeRemoteProject",
            Self::RemoveRemoteProject { .. } => "RemoveRemoteProject",
            Self::SetRemoteProjectGroup { .. } => "SetRemoteProjectGroup",
            Self::SetRemoteProjectForwards { .. } => "SetRemoteProjectForwards",
            Self::StartForward { .. } => "StartForward",
            Self::StopForward { .. } => "StopForward",
            Self::ForwardStatus => "ForwardStatus",
            Self::ScanHostKey { .. } => "ScanHostKey",
            Self::TrustHostKey { .. } => "TrustHostKey",
            Self::SaveLayout { .. } => "SaveLayout",
            Self::SelectProject { .. } => "SelectProject",
            Self::Create { .. } => "Create",
            Self::Stop { .. } => "Stop",
            Self::Rename { .. } => "Rename",
            Self::Remove { .. } => "Remove",
            Self::Focus { .. } => "Focus",
            Self::Notice { .. } => "Notice",
            Self::Settings(_) => "Settings",
            Self::Heartbeat { .. } => "Heartbeat",
            Self::Hook(_) => "Hook",
            Self::Cwd { .. } => "Cwd",
            Self::Attach { .. } => "Attach",
            Self::Input { .. } => "Input",
            Self::Resize { .. } => "Resize",
            Self::History { .. } => "History",
            Self::ClearHistory { .. } => "ClearHistory",
            Self::EditorCompare { .. } => "EditorCompare",
            Self::EditorSave { .. } => "EditorSave",
            Self::EditorStatus { .. } => "EditorStatus",
            Self::Shutdown => "Shutdown",
            Self::ShutdownIfIdle => "ShutdownIfIdle",
            Self::RetireIfDraining => "RetireIfDraining",
        }
    }
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
    VncCreated(vnc::VncSession),
    VncTransferStatus(vnc::VncTransferStatus),
    VncFrame(Box<vnc::VncFrame>),
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
pub use error::{FrameError, IpcError};
type Result<T, E = IpcError> = std::result::Result<T, E>;
pub use client::{
    SnapshotHint, conditional_snapshot, connect, read_frame, rpc, sanitize_layout, write_frame,
};
#[cfg(feature = "async-client")]
pub(crate) use client::{archived_generation, redirect_allowed};

#[cfg(test)]
mod tests;
