use anyhow::{Context, Result, bail, ensure};
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
    ProbeRemoteProject {
        project: crate::remote::RemoteProject,
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
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Response {
    Redirect { generation: String },
    IdleSessionsClosed(Vec<idle_close::Outcome>),
    SnapshotChunk { data: String, last: bool },
    Worktrees(Vec<worktrees::GitWorktree>),
    VncCreated(vnc::VncSession),
    VncTransferStatus(vnc::VncTransferStatus),
    VncFrame(Box<vnc::VncFrame>),
    Unchanged,
    Ok,
    State(Box<State>),
    RemoteProject(crate::remote::Reply),
    Created(Session),
    Text(String),
    Data(String),
    End,
    Error(String),
}
impl Response {
    pub fn checked(self) -> Result<Self> {
        if let Self::Redirect { generation } = self {
            bail!("Request rejected before execution; use active generation {generation}")
        } else if let Self::Error(e) = self {
            bail!("{e}")
        }
        Ok(self)
    }
}
pub fn write_frame<T: Serialize>(writer: &mut impl Write, value: &T) -> Result<()> {
    let data = serde_json::to_vec(value)?;
    ensure!(data.len() <= MAX_FRAME, "IPC frame too large");
    let len = u32::try_from(data.len()).context("IPC frame too large")?;
    writer.write_all(&len.to_be_bytes())?;
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
pub fn connect(
    paths: &Paths,
    request: Request,
    token: Option<String>,
) -> Result<transport::Stream> {
    connect_hint(paths, request, token, None)
}
fn connect_hint(
    paths: &Paths,
    request: Request,
    token: Option<String>,
    snapshot_hint: Option<SnapshotHint>,
) -> Result<transport::Stream> {
    let routed = generations::owner_for(paths, &request)?;
    let paths = &routed;
    let mut s = transport::Stream::connect_ipc(&paths.socket())?;
    s.set_timeouts(
        Duration::from_secs(
            if matches!(
                &request,
                Request::RemoteProject { .. } | Request::ProbeRemoteProject { .. }
            ) {
                40
            } else if matches!(
                &request,
                Request::VncFrame { .. } | Request::CloseIdleSessions { .. }
            ) {
                30
            } else {
                3
            },
        ),
        Duration::from_secs(3),
    )?;
    write_frame(
        &mut s,
        &Envelope {
            version: PROTOCOL_VERSION,
            auth: match token {
                Some(t) => t,
                None => paths.token()?,
            },
            snapshot_chunks: matches!(request, Request::Snapshot | Request::VncFrame { .. }),
            request,
            snapshot_hint,
        },
    )?;
    Ok(s)
}
pub fn rpc(paths: &Paths, request: Request) -> Result<Response> {
    if matches!(request, Request::Snapshot) && generations::exists(paths) {
        return Ok(Response::State(Box::new(generations::snapshot(paths)?)));
    }
    if generations::exists(paths)
        && matches!(
            &request,
            Request::Heartbeat { .. } | Request::ClearHistory { session: None }
        )
    {
        fanout_owners(paths, &request)?;
        return Ok(Response::Ok);
    }
    if generations::exists(paths)
        && let Some(owner) = archived_generation(paths, &request)?
        && !matches!(
            request,
            Request::Archived { .. }
                | Request::Shutdown
                | Request::ShutdownIfIdle
                | Request::Snapshot
        )
    {
        return rpc(
            paths,
            Request::Archived {
                generation: owner,
                request: Box::new(request),
            },
        );
    }
    for _ in 0..3 {
        let mut stream = connect(paths, request.clone(), None)?;
        let response = snapshot::read_response(&mut stream)?;
        if let Response::Redirect { generation } = &response
            && generations::exists(paths)
            && redirect_allowed(&request)
        {
            ensure!(
                generations::Catalog::open(paths)?
                    .generations()?
                    .iter()
                    .any(|g| &g.id == generation),
                "Redirect names an unregistered owner"
            );
            continue;
        }
        return response.checked();
    }
    bail!("Active service changed repeatedly before execution; retry the operation")
}

pub(crate) fn redirect_allowed(request: &Request) -> bool {
    matches!(
        request,
        Request::Create { .. }
            | Request::CreateReview { .. }
            | Request::AddProject { .. }
            | Request::AddRemoteProject { .. }
            | Request::SaveLayout { .. }
            | Request::SelectProject { .. }
            | Request::Settings(_)
            | Request::WorktreeAdd { .. }
            | Request::WorktreeRemove { .. }
            | Request::VncProfileSave { .. }
            | Request::VncProfileRemove { .. }
            | Request::VncForgetCredential { .. }
            | Request::VncConnect { .. }
    )
}

fn fanout_owners(paths: &Paths, request: &Request) -> Result<()> {
    let catalog = generations::Catalog::open(paths)?;
    let active = catalog.active()?;
    for owner in catalog.generations()? {
        if owner.status == generations::Status::Prepared {
            continue;
        }
        if generations::historical(&owner, active.as_deref()) {
            if matches!(request, Request::ClearHistory { .. }) {
                rpc(
                    paths,
                    Request::Archived {
                        generation: owner.id,
                        request: Box::new(request.clone()),
                    },
                )?;
            }
        } else {
            let result = rpc(&owner.paths(), request.clone());
            // Old owners may outlive their temporary runtime files. Their
            // health is reported by Snapshot; a best-effort focus heartbeat
            // must still reach the active owner and other healthy owners.
            // Explicit history mutations continue to report every failure.
            if !matches!(request, Request::Heartbeat { .. })
                || active.as_deref() == Some(owner.id.as_str())
            {
                result?;
            }
        }
    }
    Ok(())
}

pub(crate) fn archived_generation(paths: &Paths, request: &Request) -> Result<Option<String>> {
    let catalog = generations::Catalog::open(paths)?;
    let active = catalog.active()?;
    let target = generations::owner_for(paths, request)?;
    Ok(catalog
        .generations()?
        .into_iter()
        .find(|g| g.data == target.data && generations::historical(g, active.as_deref()))
        .map(|g| g.id))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotHint {
    pub generation: String,
    pub revision: u64,
    #[serde(default)]
    pub catalog_revision: u64,
    #[serde(default)]
    pub owner_revisions: Vec<(String, u64, Option<String>, generations::Status)>,
}
impl State {
    #[must_use]
    pub fn snapshot_hint(&self) -> SnapshotHint {
        SnapshotHint {
            generation: self.generation.clone(),
            revision: self.revision,
            catalog_revision: self.catalog_revision,
            owner_revisions: self
                .generations
                .iter()
                .map(|g| {
                    (
                        g.owner.id.clone(),
                        g.revision,
                        g.error.clone(),
                        g.owner.status.clone(),
                    )
                })
                .collect(),
        }
    }
}
pub fn conditional_snapshot(paths: &Paths, hint: Option<SnapshotHint>) -> Result<Response> {
    if generations::exists(paths) {
        let state = generations::snapshot(paths)?;
        return if hint.as_ref() == Some(&state.snapshot_hint()) {
            Ok(Response::Unchanged)
        } else {
            Ok(Response::State(Box::new(state)))
        };
    }
    let mut stream = connect_hint(paths, Request::Snapshot, None, hint)?;
    snapshot::read_response(&mut stream)?.checked()
}

/// Docking libraries use infinite rectangles before their first layout pass. JSON
/// encodes those as null; replace only coordinate placeholders, never optional IDs.
#[must_use]
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
                    walk(item);
                }
            }
            _ => {}
        }
    }
    walk(&mut value);
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_frame_rejected_before_allocation() {
        let mut b = u32::try_from(MAX_FRAME)
            .unwrap_or(u32::MAX)
            .saturating_add(1)
            .to_be_bytes()
            .as_slice()
            .to_vec();
        assert!(read_frame::<Request>(&mut b.as_slice()).is_err());
        b.clear();
    }

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
}
