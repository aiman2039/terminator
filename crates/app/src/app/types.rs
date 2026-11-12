use anyhow::Result;
use eframe::egui::{self};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::sync::mpsc::{self};
use std::{
    collections::HashMap,
    path::PathBuf,
    time::{Duration, Instant},
};
use terminator_core::*;

use super::super::*;
#[derive(Clone, Debug)]
pub(crate) struct HoverPopup {
    pub(crate) session: String,
    pub(crate) key: String,
    pub(crate) target: services::Target,
    pub(crate) rect: egui::Rect,
}

pub(crate) use super::drop_zone::PaneDropZone;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub(crate) enum Tab {
    Image {
        path: PathBuf,
    },
    /// A read-only native viewer with no PTY, such as the CSV/TSV table
    /// (layout v13). Only the path and viewer persist.
    Document {
        path: PathBuf,
        kind: crate::document_view::DocumentKind,
    },
    Browser {
        #[serde(default)]
        id: String,
        target: BrowserTarget,
    },
    Player,
    Terminal(String),
    NativeEditor {
        path: PathBuf,
    },
    Diff {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        remote: Option<String>,
        cwd: PathBuf,
        path: PathBuf,
        staged: bool,
    },
    CommitLog {
        cwd: PathBuf,
    },
    Blame {
        cwd: PathBuf,
        path: PathBuf,
    },
    /// Read-only diff of one file between two revisions; an empty `right`
    /// is the working tree (compare with branch, file history, commits).
    RevisionDiff {
        cwd: PathBuf,
        path: PathBuf,
        left: String,
        right: String,
    },
    /// Commits that touched one file (`IntelliJ` Show History).
    FileHistory {
        cwd: PathBuf,
        path: PathBuf,
    },
    /// Three-way merge of one conflicted file (layout v10).
    Merge {
        cwd: PathBuf,
        path: PathBuf,
    },
    /// Every changed file of a repository in one scrolling view (layout v11):
    /// the staged side, or the working tree with untracked files.
    ChangesDiff {
        cwd: PathBuf,
        staged: bool,
    },
    Vnc {
        #[serde(default)]
        id: String,
        #[serde(default)]
        session: String,
        #[serde(default)]
        profile_id: Option<String>,
        #[serde(default)]
        host: String,
        #[serde(default)]
        port: u16,
        #[serde(default)]
        label: String,
    },
    /// One window of a remote app on an SSH project's host (layout v12,
    /// `remote-apps-v1`). `window` is the host's stable window ID; `title`
    /// is the last title seen, shown until the session reconnects.
    RemoteApp {
        project: String,
        window: u64,
        #[serde(default)]
        title: String,
    },
}

/// Exact source of a view action. File paths identify buffers, not editor views.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PaneSource {
    pub project: String,
    pub group: String,
    pub path: egui_dock::TabPath,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum RenameSurface {
    Workspace,
    Pane,
    Sidebar,
}
pub(crate) struct FileActivation {
    pub(crate) project: Option<String>,
    pub(crate) path: PathBuf,
    pub(crate) staged: Option<bool>,
    pub(crate) action: FileAction,
    pub(crate) at: Instant,
}
pub(crate) struct SpawnDiff {
    pub(crate) cwd: PathBuf,
    pub(crate) path: PathBuf,
    pub(crate) staged: bool,
    pub(crate) native: bool,
}

#[derive(Clone)]
pub(crate) enum After {
    None,
    Create(Option<String>),
    /// Split creation: originating top-level tab ID, leaf anchors and split.
    /// The tab ID keeps the result in its tab when the anchors are gone.
    CreateAt(Option<String>, Vec<Tab>, Option<String>),
    Workspace(String, Vec<Tab>),
    Strip,
    StripAt(Vec<Tab>, Option<String>),
    /// Creation issued from a floating window: the originating viewport
    /// plus leaf anchors and split. Completion lands in that window's
    /// dock even after focus or project changes.
    Float(egui::ViewportId, Vec<Tab>, Option<String>),
    Text(String),
    /// Shell integration blocks for this session.
    CommandBlocks(String),
}
pub(crate) enum Job {
    PrepareLayouts(u64, Vec<(String, Workspace)>),
    SaveLayout(String, serde_json::Value, String),
    CloseIdle(editor_close::Target, String, Vec<String>),
    RepairInstallation(String, exit::Checkpoint),
    RestartSessionService(recovery::RestartInventory),
    StartSessionService,
    CreateWorktree(worktree_ui::WorktreeDraft),
    WorktreeProbe(PathBuf, worktree_ui::WorktreeDraft),
    /// Read-only branch and diff stats of a worktree row (local Git).
    WorktreeStats(PathBuf),
    Control(Box<Request>, After),
    OpenProject(PathBuf, u64),
    Preferences(UiPreferences),
    MigrateTypography,
    MigrateAttention,
    ExitDrain(u64, u64),
    ExitSave(u64, exit::Checkpoint),
    SaveAppearance(Box<AppearanceConfig>, String),
    HookStatus,
    CloseEditors(
        editor_close::Target,
        Vec<String>,
        editor_close::Mode,
        Duration,
    ),
    ResolveTarget(String, String, PathBuf),
    Browser(String),
    /// Download a browser tab icon; answers with `Update::Favicon`.
    Favicon(String),
    PasteClipboard(String),
    Diff(Tab),
    DiffContext(Tab, diff::DiffView),
    DiffExpand(
        Tab,
        std::sync::Arc<diff::DiffDocument>,
        usize,
        diff::GapPart,
    ),
    DiffAction(Tab, Box<diff::actions::Request>),
    Install(String, bool),
    TestNtfy {
        channel: String,
        machine: String,
    },
    External(PathBuf),
    /// Read usage preview lines `(file, lines)` for usages generation `u64`.
    LinePreviews(u64, Vec<(PathBuf, Vec<u32>)>),
    /// Files a commit changed, for the commit log's details pane.
    CommitFiles(PathBuf, String),
    /// Read a file's `HEAD` text and repository root for gutter markers.
    GitBase(PathBuf, u64),
    /// Blame a native buffer's text `(path, buffer revision, text)`.
    EditorBlame(PathBuf, u64, String),
    /// Stage one gutter block's changes; `cwd` is the repository root if known.
    StageGutterBlock {
        cwd: PathBuf,
        path: PathBuf,
        lines: std::ops::Range<usize>,
        text: String,
    },
    TestExternal(PathBuf, String, Vec<String>),
    Workspace(PathBuf, workspace_ops::Op),
    Search {
        id: u64,
        root: PathBuf,
        query: search::Query,
        show_ignored: bool,
    },
    RemoteAdd {
        project: terminator_core::remote::RemoteProject,
        probe: bool,
        generation: u64,
    },
    RemoteRead {
        project: String,
        operation: terminator_core::remote::Operation,
    },
    /// Scans `ssh`'s host keys, or trusts `keys` after a matching rescan.
    HostKey {
        ssh: terminator_core::ssh::SshOptions,
        keys: Option<Vec<terminator_core::ssh::HostKey>>,
    },
    /// Lists `~/.ssh/config` aliases for the SSH project dialog opened as
    /// this generation.
    SshConfigHosts(u64),
    /// `ssh -O check` per (project id, control socket) for the Hosts sidebar.
    SshControlStatus(Vec<(String, PathBuf)>),
    /// `StartForward`, `StopForward` or `ForwardStatus`.
    Forwards(Request),
    RemoteAsset {
        project: String,
        path: String,
        cache_root: PathBuf,
        after: After,
    },
    /// Picks the local file off the GUI thread, then starts an SSH project
    /// transfer in the daemon.
    RemoteTransfer {
        project: String,
        pick: crate::transfers_ui::Pick,
    },
    RemoteTransferPoll {
        project: String,
        id: String,
    },
    VncPickTransfer {
        session: String,
        direction: terminator_core::vnc::TransferDirection,
        remote: String,
        /// A file dropped on the screen; `None` opens a file picker.
        local: Option<std::path::PathBuf>,
    },
    VncTransferPoll {
        session: String,
        id: String,
    },
    VncConnectTab {
        tab_id: String,
        request: Request,
    },
    VncFrame {
        session: String,
        since: u64,
        regions: bool,
        /// Shared-memory long poll; `None` for daemons without `vnc-shm-v1`.
        shm: Option<terminator_core::vnc::VncShmRequest>,
    },
    /// Connect a project's remote apps (`session: None`), or long-poll the
    /// session's windows and read changed pixels off the GUI thread.
    RemoteApps {
        project: String,
        session: Option<String>,
        since: u64,
        readers: crate::remote_apps_ui::Readers,
    },
    /// Load a commit-log or blame tab off the GUI thread.
    GitView(Tab),
    GitLog(Tab, git_log::LogQuery),
    LocalHistory(crate::local_history::Op),
    /// List or load system fonts off the GUI thread.
    Fonts(crate::appearance::fonts::Op),
}
/// Loaded contents of a commit-log or blame tab.
pub(crate) enum GitView {
    Loading,
    Log(git_log::CommitLog),
    Blame(Result<git_log::BlameData, String>),
    History(Result<Vec<git_log::HistoryEntry>, String>),
    Revision(Result<std::sync::Arc<crate::diff::DiffDocument>, String>),
}
impl Job {
    pub(crate) fn rpc(request: Request, after: After) -> Self {
        Self::Control(Box::new(request), after)
    }
}
pub(crate) enum Update {
    #[cfg(test)]
    TestBarrier(mpsc::Sender<()>),
    ProjectDirectories(Vec<(String, PathBuf, bool)>),
    LayoutsPrepared(u64, Vec<(String, serde_json::Value, String)>),
    LayoutSaved(String, String, Result<(), String>),
    RadioCatalog(std::sync::Arc<Vec<player::radio::Station>>),
    RestartFinished(String),
    ServiceStarted(Result<(), String>),
    WorktreeCreated(Box<State>, String, worktree_ui::TaskStart),
    IdleClosed(
        editor_close::Target,
        Vec<String>,
        Result<Vec<terminator_core::idle_close::Outcome>, String>,
    ),
    InstallationRepaired(Result<Box<State>, String>),
    ExitDrained(u64, u64),
    ExitSaved(u64, Result<(), String>),
    #[cfg(test)]
    UiRequest(
        terminator_core::ui_control::Request,
        mpsc::SyncSender<Result<serde_json::Value, String>>,
        Instant,
    ),
    AsyncUiRequest(
        terminator_core::ui_control::Request,
        tokio::sync::oneshot::Sender<Result<serde_json::Value, String>>,
        Instant,
    ),
    Metadata(u64, metadata::Metadata),
    Resources(resource_sample::Sample),
    OpenImage(String, PathBuf, After),
    /// Opens a GUI-only `Tab::Document` (see `document_view`).
    OpenDocument(String, Tab, After),
    OpenNativeEditor(String, PathBuf, After),
    OpenBrowser(String, BrowserTarget, After),
    RemoteAdded(
        terminator_core::remote::RemoteProject,
        u64,
        Result<crate::remote_ui::FormResult, crate::remote_ui::Failure>,
    ),
    RemoteReply(
        String,
        terminator_core::remote::Operation,
        Result<terminator_core::remote::Reply, crate::remote_ui::Failure>,
    ),
    HostKeyScanned(
        terminator_core::ssh::SshOptions,
        Result<terminator_core::ssh::HostKeyScan, String>,
    ),
    HostKeyTrusted(terminator_core::ssh::SshOptions, Result<(), String>),
    SshConfigHosts(u64, Vec<terminator_core::ssh::ConfigHost>),
    /// Empty when the check was not run.
    SshControlStatus(Vec<(String, bool)>),
    /// The (project, local port) the request toggled, if any, and the
    /// daemon's forward status after it.
    Forwards(
        Option<(String, u16)>,
        Result<terminator_core::remote::ForwardStatus, crate::remote_ui::Failure>,
    ),
    RemoteAsset(String, PathBuf, After),
    /// Project, the polled transfer ID, and its status (`None` when the
    /// picker was cancelled).
    RemoteTransfer(
        String,
        Option<String>,
        Result<Option<terminator_core::remote::TransferStatus>, crate::remote_ui::Failure>,
    ),
    VncTransfer(
        String,
        Result<Option<terminator_core::vnc::VncTransferStatus>, String>,
    ),
    VncConnected(terminator_core::vnc::VncSession, After),
    VncTabConnected(String, Result<terminator_core::vnc::VncSession, String>),
    VncFrame(
        String,
        Result<Option<crate::vnc_ui::VncDecodedFrame>, String>,
    ),
    /// A project's remote-apps state; `Ok(None)` is unchanged.
    RemoteApps(
        String,
        Result<Option<Box<crate::remote_apps_ui::RemoteAppsFrame>>, String>,
    ),
    FocusViewport(egui::ViewportId),
    RevealFileSession(String, String),
    Image(PathBuf, u64, Result<egui::ColorImage, String>),
    Table(
        PathBuf,
        u64,
        Result<std::sync::Arc<crate::document_view::Table>, String>,
    ),
    TestPickerClosed,
    PickedProject(Option<PathBuf>, u64),
    OpenedProject(Box<State>, String, u64),
    TypographyMigrated,
    AttentionMigrated(Result<(), String>),
    Activation(String),
    Appearance(Box<AppearanceFile>),
    HookStatus(HashMap<String, bool>),
    /// Catalog agents found on PATH, probed off the GUI thread.
    InstalledAgents(Vec<crate::agent_launch::InstalledAgent>),
    EditorsClosed(editor_close::Target, Vec<String>, Result<(), String>),
    ResolvedTarget(String, Option<services::Target>),
    PreferencesSaved(Box<Result<UiPreferences, String>>),
    PickedFile {
        path: Option<PathBuf>,
        project: Option<String>,
        cwd: PathBuf,
    },
    PickedAudio(Vec<PathBuf>),
    PickedVncFiles(Vec<PathBuf>),
    PickedPath {
        path: Option<PathBuf>,
        target: BrowseTarget,
    },
    State(Box<State>),
    /// Session, split, leaf anchors and originating top-level tab ID.
    Created(Session, Option<String>, Option<Vec<Tab>>, Option<String>),
    WorkspaceCreated(Session, String, Vec<Tab>),
    StripCreated(Session, Option<String>, Vec<Tab>),
    FloatCreated(Session, egui::ViewportId, Option<String>, Vec<Tab>),
    Text(String, String),
    CommandBlocks(String, Vec<terminator_core::command_blocks::CommandBlock>),
    /// A Text or CommandBlocks request failed: clear its `loading` key.
    LoadFailed(String, String),
    Diff(String, Result<diff::DiffDocument, String>),
    DiffAction(Tab, bool, Result<diff::actions::Outcome, String>),
    DiffActionFailed(String, String),
    /// A workspace op finished in the repository root it ran in.
    Workspace(
        PathBuf,
        workspace_ops::Op,
        Result<workspace_ops::Report, String>,
    ),
    Search(u64, Result<Vec<search::Hit>, String>),
    Favicon(String, Option<Vec<u8>>),
    LinePreviews(u64, Vec<((PathBuf, u32), String)>),
    GitBase(PathBuf, u64, Option<String>, Option<PathBuf>),
    /// The `HEAD` read of `(path, generation)` ended without a reply.
    GitBaseFailed(PathBuf, u64),
    /// The create for this new top-level tab ID failed or was refused.
    CreateFailed(String, String),
    /// The worktree wizard submission of this draft ID failed.
    WorktreeFailed(u64, String),
    /// `PrepareLayouts` of this generation ended without its layouts.
    LayoutPrepareFailed(u64),
    /// A gutter block staged `(path, message or error)`.
    GutterStaged(PathBuf, Result<String, String>),
    EditorBlame(PathBuf, u64, Result<git_log::BlameData, String>),
    CommitFiles(String, Result<Vec<(char, PathBuf)>, String>),
    WorktreeProbe(worktree_ui::WorktreeDraft, bool),
    WorktreeStats(PathBuf, Option<crate::worktree_stats::WorktreeGit>),
    GitView(String, GitView),
    LocalHistory(crate::local_history::Reply),
    Fonts(crate::appearance::fonts::Reply),
    DirectoryPreview(
        u64,
        PathBuf,
        PathBuf,
        Result<Vec<terminator_git::Entry>, services::DirectoryError>,
    ),
    Refresh(
        u64,
        services::ContextData,
        Vec<(
            PathBuf,
            Result<Vec<terminator_git::Entry>, services::DirectoryError>,
        )>,
        bool,
    ),
    ClipboardPaste(String, String),
    Error(String),
    Info(String),
}
