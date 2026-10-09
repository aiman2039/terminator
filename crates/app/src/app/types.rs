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

/// Drop zone within a hovered split leaf while a terminal is dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaneDropZone {
    /// The middle: swap single panes or join the leaf.
    Center,
    /// An edge band: open the dragged pane in a new split beside the leaf.
    Above,
    Below,
    Left,
    Right,
}
impl PaneDropZone {
    pub(crate) fn split(self) -> Option<egui_dock::Split> {
        match self {
            Self::Center => None,
            Self::Above => Some(egui_dock::Split::Above),
            Self::Below => Some(egui_dock::Split::Below),
            Self::Left => Some(egui_dock::Split::Left),
            Self::Right => Some(egui_dock::Split::Right),
        }
    }
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Center => "Place terminal here",
            Self::Above => "Split above",
            Self::Below => "Split below",
            Self::Left => "Split left",
            Self::Right => "Split right",
        }
    }
    /// The part of the leaf the dragged pane will occupy.
    pub(crate) fn landing(self, rect: egui::Rect) -> egui::Rect {
        let center = rect.center();
        match self {
            Self::Center => rect,
            Self::Above => egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), center.y)),
            Self::Below => egui::Rect::from_min_max(egui::pos2(rect.left(), center.y), rect.max),
            Self::Left => egui::Rect::from_min_max(rect.min, egui::pos2(center.x, rect.bottom())),
            Self::Right => egui::Rect::from_min_max(egui::pos2(center.x, rect.top()), rect.max),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub(crate) enum Tab {
    Image {
        path: PathBuf,
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
}
impl Tab {
    pub(crate) fn key(&self) -> String {
        if let Self::Browser { id, .. } = self
            && !id.is_empty()
        {
            return id.clone();
        }
        serde_json::to_string(self).unwrap_or_default()
    }

    pub(crate) fn browser_file(path: PathBuf) -> Self {
        Self::Browser {
            id: String::new(),
            target: BrowserTarget::File(path),
        }
    }

    pub(crate) fn layout_version(&self) -> u32 {
        match self {
            Self::NativeEditor { .. } => 7,
            Self::Browser { .. } => 6,
            Self::Player => 5,
            Self::Image { .. } => 3,
            Self::Diff { .. } | Self::Terminal(_) | Self::CommitLog { .. } | Self::Blame { .. } => {
                2
            }
        }
    }
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
    CreateAt(Vec<Tab>, Option<String>),
    Workspace(String, Vec<Tab>),
    Strip,
    StripAt(Vec<Tab>, Option<String>),
    /// Creation issued from a floating window: the originating viewport
    /// plus leaf anchors and split. Completion lands in that window's
    /// dock even after focus or project changes.
    Float(egui::ViewportId, Vec<Tab>, Option<String>),
    Text(String),
}
pub(crate) enum Job {
    PrepareLayouts(u64, Vec<(String, Workspace)>),
    SaveLayout(String, serde_json::Value, String),
    CloseIdle(editor_close::Target, String, Vec<String>),
    RepairInstallation(String, exit::Checkpoint),
    RestartSessionService(recovery::RestartInventory),
    StartSessionService,
    CreateWorktree(worktree_ui::WorktreeDraft),
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
    PasteClipboard(String),

    Diff(Tab),
    Install(String, bool),
    TestNtfy {
        channel: String,
        machine: String,
    },
    External(PathBuf),
    TestExternal(PathBuf, String, Vec<String>),
    Workspace(PathBuf, workspace_ops::Op),
    Search {
        id: u64,
        root: PathBuf,
        query: search::Query,
        show_ignored: bool,
    },
}
impl Job {
    pub(crate) fn rpc(request: Request, after: After) -> Self {
        Self::Control(Box::new(request), after)
    }
}
pub(crate) enum Update {
    ProjectDirectories(Vec<(String, PathBuf, bool)>),
    LayoutsPrepared(u64, Vec<(String, serde_json::Value, String)>),
    LayoutSaved(String, String, Result<(), String>),
    RadioCatalog(std::sync::Arc<Vec<player::radio::Station>>),
    RestartFinished(String),
    ServiceStarted(Result<(), String>),
    WorktreeCreated(Box<State>, String, bool),
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
    OpenNativeEditor(String, PathBuf, After),
    OpenBrowser(String, BrowserTarget, After),
    Image(PathBuf, u64, Result<egui::ColorImage, String>),
    TestPickerClosed,
    PickedProject(Option<PathBuf>, u64),
    OpenedProject(Box<State>, String, u64),
    TypographyMigrated,
    AttentionMigrated(Result<(), String>),
    Activation(String),
    Appearance(Box<AppearanceFile>),
    HookStatus(HashMap<String, bool>),
    EditorsClosed(editor_close::Target, Vec<String>, Result<(), String>),
    ResolvedTarget(String, Option<services::Target>),
    PreferencesSaved(Box<Result<UiPreferences, String>>),
    PickedFile {
        path: Option<PathBuf>,
        project: Option<String>,
        cwd: PathBuf,
    },
    PickedAudio(Vec<PathBuf>),
    PickedPath {
        path: Option<PathBuf>,
        target: BrowseTarget,
    },
    State(Box<State>),
    Created(Session, Option<String>, Option<Vec<Tab>>),
    WorkspaceCreated(Session, String, Vec<Tab>),
    StripCreated(Session, Option<String>, Vec<Tab>),
    FloatCreated(Session, egui::ViewportId, Option<String>, Vec<Tab>),
    Text(String, String),
    Diff(String, Result<diff::DiffDocument, String>),
    Workspace(workspace_ops::Op, Result<workspace_ops::Report, String>),
    Search(u64, Result<Vec<search::Hit>, String>),
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
