//! Git commands in Find Action (the palette). Each one runs the same code
//! as its Git panel button, menu row, or editor menu item.
use std::path::Path;

use crate::revision_compare::PickerMode;
use crate::*;

/// The Git actions, in Find Action order.
pub(crate) const GIT_ACTIONS: &[Action] = &[
    Action::GitCommit,
    Action::GitCommitPush,
    Action::GitRefresh,
    Action::GitShowLog,
    Action::GitCompareWith,
    Action::GitCompareRevision,
    Action::GitShowHistory,
    Action::GitAnnotate,
    Action::GitStageAll,
];

impl App {
    /// Where a Git action can run: a local Git repository for the panel
    /// actions, a focused file for the file actions. Cheap; no I/O.
    pub(crate) fn git_action_applies(&self, action: Action) -> bool {
        match action {
            Action::GitCommit | Action::GitShowLog => self.local_git_context().is_some(),
            Action::GitRefresh => {
                self.selected_remote_project().is_none() && self.context.is_some()
            }
            Action::GitCommitPush => self.commit_ready(),
            Action::GitStageAll => self
                .local_git_context()
                .is_some_and(|context| context.changes.iter().any(|change| !change.conflict())),
            Action::GitCompareWith | Action::GitCompareRevision | Action::GitShowHistory => {
                self.focused_local_file().is_some()
            }
            Action::GitAnnotate => self.active_editor_path_ref().is_some(),
            _ => false,
        }
    }

    pub(crate) fn run_git_action(&mut self, action: Action) {
        match action {
            Action::GitCommit => {
                self.show_git_changes();
                self.git.focus_message = true;
            }
            Action::GitCommitPush => {
                self.show_git_changes();
                self.commit_draft(true);
            }
            Action::GitRefresh => {
                self.refresh_request = None;
                self.git.list_root = None;
            }
            Action::GitShowLog => self.open_commit_log(),
            Action::GitCompareWith => self.compare_focused_file(PickerMode::Refs),
            Action::GitCompareRevision => self.compare_focused_file(PickerMode::History),
            Action::GitShowHistory => self.show_focused_file_history(),
            Action::GitAnnotate => self.annotate_active_editor(),
            Action::GitStageAll => self.stage_all_changes(),
            _ => {}
        }
    }

    /// The context of a local Git repository (not SSH, not Perforce).
    fn local_git_context(&self) -> Option<&services::ContextData> {
        self.context.as_ref().filter(|context| {
            context.root.is_some()
                && context.perforce.is_none()
                && self.selected_remote_project().is_none()
        })
    }

    /// The Commit and Push button would be enabled: staged changes, a
    /// message (or Amend), and no commit or push running.
    fn commit_ready(&self) -> bool {
        self.git.run.is_none()
            && (self.git.amend || !self.git.commit.trim().is_empty())
            && self.local_git_context().is_some_and(|context| {
                context
                    .changes
                    .iter()
                    .any(|change| change.in_group(terminator_git::GitGroup::Staged))
            })
    }

    /// The Git panel in Changes view, where the commit message is.
    fn show_git_changes(&mut self) {
        let left =
            self.preferences.left_visible && self.preferences.left_tool == Some(SidebarTool::Git);
        if !left {
            self.show_git_sidebar();
        }
        self.git.history = false;
    }

    fn compare_focused_file(&mut self, mode: PickerMode) {
        if let Some(path) = self.focused_local_file() {
            self.open_compare_picker(&path, mode);
        }
    }

    fn show_focused_file_history(&mut self) {
        let Some(path) = self.focused_local_file() else {
            return;
        };
        if self.remote_git_file(&path) {
            return;
        }
        // The Explorer's Show history uses the repository root too.
        let cwd = self
            .context
            .as_ref()
            .and_then(|context| context.root.clone())
            .filter(|root| path.starts_with(root))
            .or_else(|| path.parent().map(Path::to_path_buf));
        if let Some(cwd) = cwd {
            self.open_git_tab(Tab::FileHistory { cwd, path });
        }
    }

    fn annotate_active_editor(&mut self) {
        let Some(path) = self.active_editor_path() else {
            return;
        };
        if !self.remote_git_file(&path) {
            self.toggle_editor_blame(&path);
        }
    }

    /// A file with no local repository: says so in the status bar.
    fn remote_git_file(&mut self, path: &Path) -> bool {
        let remote = self.selected_remote_project().is_some()
            || crate::remote_ui::is_snapshot(&self.paths.data, path);
        if remote {
            self.info = Some(crate::revision_compare::REMOTE_NOTE.into());
        }
        remote
    }
}
