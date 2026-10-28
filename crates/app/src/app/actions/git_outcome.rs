//! Applying a Git panel frame's outcome: file actions, commits, shelves,
//! staging, and compare-base changes, in the order the panel reported them.

use std::path::PathBuf;

use super::FileActionInput;
use crate::*;

/// Commit messages kept for the history dropdown.
const COMMIT_HISTORY_LIMIT: usize = 20;

impl App {
    pub(crate) fn perform_git_outcome(
        &mut self,
        ui: &egui::Ui,
        mut outcome: sidebar_ui::GitPanelOutcome,
    ) {
        self.git_outcome_views(ui, &mut outcome);
        self.git_outcome_commits(&mut outcome);
        self.git_outcome_index(&mut outcome);
        for path in outcome.delete {
            self.pending_delete = Some(path);
        }
        for text in outcome.copy {
            ui.ctx().copy_text(text);
        }
    }

    /// Perforce ops, refresh, and the tabs and files the panel opens.
    fn git_outcome_views(&mut self, ui: &egui::Ui, outcome: &mut sidebar_ui::GitPanelOutcome) {
        for op in std::mem::take(&mut outcome.perforce) {
            self.queue_workspace(workspace_ops::Op::Perforce(op));
        }
        if outcome.refresh {
            self.refresh_request = None;
            self.git.list_root = None;
        }
        if outcome.view_log {
            self.open_commit_log();
        }
        for clicked in std::mem::take(&mut outcome.clicked) {
            self.activate_file_action(ui, &clicked.path, clicked.action);
        }
        for picked in std::mem::take(&mut outcome.menu) {
            self.file_action(FileActionInput {
                ui,
                action: picked.action,
                path: &picked.path,
                line: None,
            });
        }
        if let Some(path) = outcome.resolve.take() {
            self.open_merge(path);
        }
        if let Some(history) = outcome.history {
            self.git.history = history;
        }
        if outcome.collapse {
            self.git.collapse = self.git.collapse.wrapping_add(1);
        }
    }

    /// Branch switch, commit, and shelf operations.
    fn git_outcome_commits(&mut self, outcome: &mut sidebar_ui::GitPanelOutcome) {
        if let Some(name) = outcome.switch.take() {
            self.queue_workspace(workspace_ops::Op::Switch(name));
        }
        if outcome.load_head_message {
            self.queue_workspace(workspace_ops::Op::HeadMessage);
        }
        if outcome.commit || outcome.commit_push {
            self.commit_draft(outcome.commit_push);
        }
        if outcome.shelve {
            let paths =
                self.changed_paths(|change| change.in_group(terminator_git::GitGroup::Staged));
            self.queue_workspace(workspace_ops::Op::Shelve(paths));
        }
        if let Some(reference) = outcome.unshelve.take() {
            self.queue_workspace(workspace_ops::Op::Unshelve(reference));
        }
        if let Some(reference) = outcome.drop_shelf.take() {
            self.git.confirm = Some(crate::sidebar_ui::GitConfirm::DropShelf(reference));
        }
    }

    /// Commits the draft message and records it in the message history.
    fn commit_draft(&mut self, push: bool) {
        let message = std::mem::take(&mut self.git.commit);
        let kept = message.trim();
        if !kept.is_empty() {
            let history = &mut self.preferences.commit_messages;
            history.retain(|old| old.trim() != kept);
            history.insert(0, kept.to_owned());
            history.truncate(COMMIT_HISTORY_LIMIT);
        }
        self.git.commit_inflight = Some(message.clone());
        self.queue_workspace(workspace_ops::Op::Commit {
            message,
            amend: self.git.amend,
            signoff: self.git.signoff,
            push,
        });
        self.git.amend = false;
    }

    /// Paths of the current changes that `keep` selects.
    fn changed_paths(&self, keep: impl Fn(&terminator_git::Change) -> bool) -> Vec<PathBuf> {
        self.context
            .as_ref()
            .map(|context| {
                context
                    .changes
                    .iter()
                    .filter(|change| keep(change))
                    .map(|change| change.path.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Staging, list mode, compare base, unstaging, and discard prompts.
    fn git_outcome_index(&mut self, outcome: &mut sidebar_ui::GitPanelOutcome) {
        for path in std::mem::take(&mut outcome.stage) {
            self.queue_workspace(workspace_ops::Op::Stage(path));
        }
        if outcome.stage_all {
            for path in self.changed_paths(|change| !change.conflict()) {
                self.queue_workspace(workspace_ops::Op::Stage(path));
            }
        }
        if outcome.toggle_list {
            self.preferences.git_view_list = !self.preferences.git_view_list;
        }
        if outcome.clear_base {
            self.git.base_ref = None;
            self.queue_workspace(workspace_ops::Op::Compare { base: None });
        }
        if let Some(base) = outcome.set_base.take() {
            self.git.base_ref = Some(base.clone());
            self.queue_workspace(workspace_ops::Op::Compare { base: Some(base) });
        }
        if outcome.refresh_compare {
            self.queue_workspace(workspace_ops::Op::Compare {
                base: self.git.base_ref.clone(),
            });
        }
        for path in std::mem::take(&mut outcome.unstage) {
            self.queue_workspace(workspace_ops::Op::Unstage(path));
        }
        if let Some((path, untracked)) = std::mem::take(&mut outcome.discard).into_iter().last() {
            self.git.confirm = Some(crate::sidebar_ui::GitConfirm::Discard { path, untracked });
        }
    }

    /// Open (or focus) a Git view tab in the selected project.
    pub(crate) fn open_git_tab(&mut self, tab: Tab) {
        let Some(project) = self.selected.clone() else {
            return;
        };
        let workspace = self
            .layouts
            .entry(project.clone())
            .or_insert_with(Workspace::empty);
        if !workspace.contains(&tab) {
            workspace.add(terminator_core::id(), tab.clone());
        }
        self.insert(&project, tab, None);
    }

    pub(crate) fn open_commit_log(&mut self) {
        let Some(project) = self.selected.clone() else {
            return;
        };
        let Some(context) = &self.context else {
            return;
        };
        let Some(root) = &context.root else {
            return;
        };
        let workspace = self
            .layouts
            .entry(project.clone())
            .or_insert_with(Workspace::empty);
        let tab = Tab::CommitLog { cwd: root.clone() };
        if !workspace.contains(&tab) {
            workspace.add(terminator_core::id(), tab.clone());
        }
        self.insert(&project, tab, None);
    }
}
