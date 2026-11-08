//! Explorer, session rows, and sidebar tool hosting.
mod dnd;
mod entry;
mod session_row;
mod status;
mod tool;
mod tree;

use super::attention::state_color;
use super::explorer_prompts::PromptSlot;
use super::explorer_rows::{ExplorerFileRowInput, explorer_file_row, skip_clipped_row};
use super::explorer_select::{ExplorerDrag, tree_focus_id};
use super::git_panel::{PreparedGit, git_color, git_panel};
use crate::app::FileActionInput;
#[cfg(feature = "test-support")]
use crate::diagnostics;
use crate::sidebar_ui::ExplorerKeysInput;
use crate::sidebar_ui::git_panel::GitPanelInput;
use crate::{
    Action, App, RenameSurface, Tab, appearance,
    preferences::{ExplorerSearchMode, SidebarTool},
    workspace_ops,
};

struct ExplorerPromptRowInput<'a> {
    ui: &'a mut egui::Ui,
    depth: usize,
    tree_focus: egui::Id,
}

struct ExplorerTreeRowInput<'a, 'b> {
    ui: &'a mut egui::Ui,
    row: &'a crate::sidebar_cache::ExplorerRow,
    context: &'a RowContext<'b>,
    drop_dir: &'a mut Option<PathBuf>,
}

struct ExplorerRowDndInput<'a> {
    ui: &'a egui::Ui,
    response: &'a egui::Response,
    path: &'a Path,
    directory: bool,
    drop_dir: &'a mut Option<PathBuf>,
}

struct ExplorerEntryInput<'a, 'b> {
    ui: &'a mut egui::Ui,
    entry: &'a terminator_git::Entry,
    row: &'a crate::sidebar_cache::ExplorerRow,
    context: &'a RowContext<'b>,
    drop_dir: &'a mut Option<PathBuf>,
}

pub(crate) struct SessionRowInTabInput<'a> {
    pub(crate) ui: &'a mut egui::Ui,
    pub(crate) session: &'a Session,
    pub(crate) project_id: &'a str,
    pub(crate) tab_id: &'a str,
}

use eframe::egui::{self, RichText};
use std::path::{Path, PathBuf};
use terminator_core::{
    AgentState, NVIM_REVIEW_CAPABILITY, Project, Request, Session, SessionKind, now,
};

/// Explorer rows are one fixed height, which lets the tree paint only the
/// visible slice.
pub(super) const ROW_HEIGHT: f32 = 24.0;

/// Per-paint inputs every tree row shares.
struct RowContext<'a> {
    rows: &'a [crate::sidebar_cache::ExplorerRow],
    focus: egui::Id,
    focused_file: Option<PathBuf>,
}

pub(crate) struct ExplorerReveal {
    pub project: String,
    pub host: egui::Id,
    pub root: PathBuf,
    pub file: PathBuf,
    pub pending_scroll: bool,
}

impl App {
    /// The local file in the selected main pane. A focused strip shell
    /// means the visible editor is in the background, so it is not the
    /// current file for Reveal.
    pub(crate) fn focused_local_file(&self) -> Option<PathBuf> {
        let project = self.selected.as_ref()?;
        if self.active_session.as_ref().is_some_and(|sid| {
            self.state
                .sessions
                .iter()
                .any(|session| &session.id == sid && session.kind == SessionKind::Shell)
        }) {
            return None;
        }
        let pane = self.layouts.get(project)?.active_pane()?;
        match pane {
            Tab::NativeEditor { path }
            | Tab::Image { path }
            | Tab::Diff { path, .. }
            | Tab::Blame { path, .. }
            | Tab::RevisionDiff { path, .. }
            | Tab::FileHistory { path, .. }
            | Tab::Merge { path, .. } => Some(path.clone()),
            Tab::Browser { target, .. } => target.file().map(Path::to_path_buf),
            Tab::Terminal(sid) => self
                .state
                .sessions
                .iter()
                .find(|session| {
                    &session.id == sid
                        && &session.project_id == project
                        && session.kind == SessionKind::Editor
                        && !session.review
                })
                .and_then(|session| session.file.clone()),
            Tab::CommitLog { .. }
            | Tab::ChangesDiff { .. }
            | Tab::Player
            | Tab::Vnc { .. }
            | Tab::RemoteApp { .. } => None,
        }
    }

    /// Prefer the Explorer's current root. If a file is elsewhere in its
    /// project, reveal it from the project root without changing the
    /// shell's working directory.
    pub(crate) fn explorer_reveal_candidate(&self) -> Option<(PathBuf, PathBuf)> {
        let file = self.focused_local_file()?;
        let project = self.selected_project()?;
        let cwd = self.cwd().unwrap_or_else(|| project.path.clone());
        let root = if file.starts_with(&cwd) {
            cwd
        } else if file.starts_with(&project.path) {
            project.path.clone()
        } else {
            return None;
        };
        let relative = file.strip_prefix(&root).ok()?;
        if relative.as_os_str().is_empty() || relative.components().count() > 20 {
            return None;
        }
        Some((file, root))
    }

    /// Why the current file cannot be revealed in the Explorer.
    pub(crate) fn explorer_reveal_note(&self) -> &'static str {
        if self.focused_local_file().is_some() {
            "The focused file is outside the Explorer folder"
        } else {
            "Focus a local file to reveal it in Explorer"
        }
    }

    pub(crate) fn reveal_current_file_in_explorer(&mut self, host: egui::Id) -> bool {
        let Some((file, root)) = self.explorer_reveal_candidate() else {
            return false;
        };
        self.reveal_under(host, file, root)
    }

    /// Status-bar breadcrumb: show the Explorer and select `target`, the
    /// focused file or one of its folders. Like Locate, a folder above the
    /// shell directory is shown from the project root. The first Explorer
    /// that paints takes the reveal.
    pub(crate) fn reveal_in_explorer(&mut self, target: &Path) -> bool {
        let Some((file, shell_root)) = self.explorer_reveal_candidate() else {
            return false;
        };
        let inside = |root: &Path| target.starts_with(root) && target != root;
        let root = if inside(shell_root.as_path()) {
            shell_root
        } else {
            match self.selected_project().map(|project| project.path.clone()) {
                Some(root) if inside(root.as_path()) => root,
                _ => return false,
            }
        };
        if !file.starts_with(target) || !self.reveal_under(egui::Id::NULL, file.clone(), root) {
            return false;
        }
        if target != file
            && let Some(reveal) = self.explorer_reveal.as_mut()
        {
            // Scroll to the folder, not to the file inside it.
            reveal.pending_scroll = false;
        }
        // "Always select opened file" must not take the selection back.
        self.explorer_followed = Some(file);
        self.explorer_selection = vec![target.to_path_buf()];
        self.explorer_anchor = Some(target.to_path_buf());
        self.explorer_scroll_to = Some(target.to_path_buf());
        self.show_explorer();
        true
    }

    /// Show the Explorer where it already is, else in the sidebar the mode
    /// treats as primary (left in IDE mode, right otherwise).
    fn show_explorer(&mut self) {
        let prefs = &mut self.preferences;
        let left = prefs.left_shows(SidebarTool::Explorer);
        let right = prefs.visible && !prefs.right_projects && prefs.tool == SidebarTool::Explorer;
        if left || right {
            return;
        }
        if prefs.ide_mode {
            prefs.set_left_content(crate::LeftContent::Tool(SidebarTool::Explorer));
            prefs.left_visible = true;
        } else {
            prefs.right_projects = false;
            prefs.tool = SidebarTool::Explorer;
            prefs.visible = true;
        }
    }

    /// Open the folders between `root` and `file` and point the Explorer
    /// in `host` at `root`, scrolled to `file`. `Id::NULL` lets the first
    /// Explorer that paints take it.
    fn reveal_under(&mut self, host: egui::Id, file: PathBuf, root: PathBuf) -> bool {
        let Some(project) = self.selected.clone() else {
            return false;
        };
        let mut parent = file.parent();
        while let Some(dir) = parent.filter(|dir| *dir != root) {
            if !dir.starts_with(&root) {
                return false;
            }
            self.expanded_dirs.insert(dir.to_path_buf());
            parent = dir.parent();
        }
        self.preferences.explorer_search_mode = ExplorerSearchMode::Names;
        self.explorer_query.clear();
        self.explorer_search_last = None;
        self.explorer_reveal = Some(ExplorerReveal {
            project,
            host,
            root,
            file,
            pending_scroll: true,
        });
        self.refresh_request = None;
        true
    }
    pub(crate) fn explorer_tooltip(&self) -> String {
        let Some(cwd) = self.cwd() else {
            return "Explorer — no selected directory".into();
        };
        let unconfirmed = self.folder_session().is_some_and(|s| !s.cwd_confirmed);
        format!(
            "Explorer\n{}{}",
            cwd.display(),
            if unconfirmed {
                "\nLast known directory"
            } else {
                ""
            }
        )
    }

    pub(crate) fn sidebar_project(&self, project: &Project) -> std::sync::Arc<Project> {
        let mut cached = self.sidebar_projects.borrow_mut();
        let entry = cached
            .entry(project.id.clone())
            .or_insert_with(|| std::sync::Arc::new(project.clone()));
        if entry.as_ref() != project {
            *entry = std::sync::Arc::new(project.clone());
        }
        std::sync::Arc::clone(entry)
    }

    pub(crate) fn visible_projects(&self) -> Vec<std::sync::Arc<Project>> {
        self.cached_projects().as_ref().clone()
    }

    /// Brand plus status for a project row when one of its live sessions has an agent.
    /// A running agent wins over a finished one, so a working logo is not replaced by a folder.
    pub(super) fn project_agent_face(
        &self,
        project_id: &str,
    ) -> Option<(&'static str, AgentState)> {
        self.sidebar_index().faces.get(project_id).copied()
    }

    pub(super) fn session_row(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
    ) -> Option<egui::Response> {
        self.session_row_for_tab(ui, session, None)
    }

    /// One session row that stands in for a single-terminal tab. It offers to
    /// name that tab, since the tab node is not painted.
    pub(super) fn session_row_in_tab(
        &mut self,
        input: SessionRowInTabInput<'_>,
    ) -> Option<egui::Response> {
        let SessionRowInTabInput {
            ui,
            session,
            project_id,
            tab_id,
        } = input;
        self.session_row_for_tab(ui, session, Some((project_id, tab_id)))
    }
}
