//! The Git sidebar panel.
mod branch_menu;
mod commit;
mod commit_progress;
mod committed;
#[cfg(test)]
mod footer_tests;
mod group_header;
mod name_row;
mod rows;
mod sync_progress;
#[cfg(test)]
mod tests;
mod toolbar;
mod tree;

use super::explorer_rows::skip_clipped_row;
#[cfg(feature = "test-support")]
use crate::diagnostics;
use crate::{
    App, appearance, file_actions::FileAction, icons, services::ContextData, workspace_ops,
};
pub(crate) use branch_menu::{CommitPick, commit_actions};
use commit::git_error_row;
pub(crate) use commit_progress::{CommitLock, CommitPhase, CommitProgress};
use committed::*;
use eframe::egui::{self, Color32, RichText};
use group_header::*;
use name_row::*;
#[cfg(test)]
pub(super) use rows::GIT_TREE_ROW_HEIGHT;
use rows::*;
use std::path::{Path, PathBuf};
pub(crate) use sync_progress::publish as publish_sync_run;
use terminator_core::ReviewMode;
use terminator_core::appearance::AppearanceConfig;
use toolbar::*;
pub(crate) use tree::PreparedGit;
#[cfg(test)]
pub(super) use tree::build_change_tree;
use tree::*;

pub(super) fn skip_clipped_git_row(ui: &mut egui::Ui) -> bool {
    skip_clipped_row(ui, GIT_TREE_ROW_HEIGHT)
}

impl App {
    pub(super) fn ensure_git_lists(&mut self, root: Option<std::path::PathBuf>) {
        let Some(root) = root else {
            return;
        };
        if self.git.list_root.as_ref() == Some(&root) {
            return;
        }
        self.git.list_root = Some(root);
        self.git.list_error = None;
        self.git.branches.clear();
        self.git.log.clear();
        self.git.shelves.clear();
        self.git.compare = None;
        self.git.compare_root = None;
        self.git.base_ref = None;
        self.queue_workspace(workspace_ops::Op::Branches);
        self.queue_workspace(workspace_ops::Op::Log);
        self.queue_workspace(workspace_ops::Op::Shelves);
        self.queue_workspace(workspace_ops::Op::Compare { base: None });
    }
}

pub fn git_color(theme: &AppearanceConfig, status: char) -> Color32 {
    appearance::color(match status {
        'A' => &theme.git_added,
        'M' => &theme.git_modified,
        'D' | '!' => &theme.git_deleted,
        'R' | 'C' | 'U' => &theme.git_untracked,
        _ => &theme.secondary,
    })
}

/// View data for the Git panel. `App` passes a snapshot of its state; the
/// panel paints it and returns actions without touching `App` or `Services`.
pub struct GitPanelInput<'a> {
    pub context: &'a ContextData,
    pub(crate) prepared: Option<&'a PreparedGit>,
    pub review_mode: ReviewMode,
    pub neovim_review: bool,
    pub theme: &'a AppearanceConfig,
    /// The open header tab.
    pub tab: GitTab,
    pub view_list: bool,
    pub commits: &'a [(String, String)],
    pub shelves: &'a [(String, String)],
    pub branches: &'a [String],
    pub commit_draft: &'a mut String,
    /// Recent commit messages, newest first.
    pub recent_messages: &'a [String],
    pub amend: &'a mut bool,
    pub signoff: &'a mut bool,
    /// Bumped by Expand all / Collapse all: a new generation resets every
    /// section and folder to `tree_open`.
    pub collapse_generation: u64,
    pub tree_open: bool,
    pub open_shortcut: &'a str,
    /// Branch comparison for the current root, when available.
    pub compare: Option<&'a workspace_ops::CompareData>,
    /// User-picked base ref; `None` means the upstream branch is used.
    pub base_ref: Option<&'a str>,
    /// Focus the commit message this paint (Git: Commit… in Find Action).
    pub focus_message: bool,
    /// The commit or push in flight; commit controls wait for its reply.
    pub(crate) commit_run: Option<CommitProgress<'a>>,
    /// Controls that a commit running in this repository disables.
    pub(crate) commit_lock: CommitLock,
    /// Summary line and output tail of the last failed commit or push.
    pub commit_failure: Option<&'a str>,
    /// The last failure of a list the panel loaded by itself.
    pub list_error: Option<&'a str>,
}

/// The Git panel's header tabs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GitTab {
    /// Changes and the commit area.
    #[default]
    Commit,
    /// Shelved changes (`git stash`).
    Stash,
    /// The commit log.
    History,
    /// The current branch's pull request (read only); `App` paints it.
    PullRequest,
}

/// A concrete file action with its target, ready for `App` to perform.
pub struct GitFileAction {
    pub action: FileAction,
    pub path: PathBuf,
}

/// What the Git panel asks `App` to do after painting.
#[derive(Default)]
pub struct GitPanelOutcome {
    pub perforce: Vec<crate::perforce::Op>,
    /// Row clicks mapped to concrete actions. `App` dedups rapid repeats.
    pub clicked: Vec<GitFileAction>,
    /// Context-menu picks, performed directly.
    pub menu: Vec<GitFileAction>,
    /// The error-state Retry button was pressed: re-queue a context refresh.
    pub refresh: bool,
    /// User pressed the View log button.
    pub view_log: bool,
    /// Show all changes of a side in one view (`true` = staged).
    pub show_changes: Option<bool>,
    /// A header tab was picked.
    pub tab: Option<GitTab>,
    /// Expand all (`true`) or Collapse all (`false`) was picked.
    pub expand_all: Option<bool>,
    pub commit: bool,
    pub commit_push: bool,
    /// Create PR was clicked.
    pub create_pr: bool,
    /// Stop the commit or push in flight.
    pub cancel_commit: bool,
    /// Hide the last failure's output.
    pub dismiss_commit_failure: bool,
    /// Amend was ticked with an empty draft: load the `HEAD` message.
    pub load_head_message: bool,
    /// Shelve every staged (checked) path.
    pub shelve: bool,
    pub unshelve: Option<String>,
    pub drop_shelf: Option<String>,
    pub switch: Option<String>,
    pub stage: Vec<PathBuf>,
    pub unstage: Vec<PathBuf>,
    pub discard: Vec<(PathBuf, bool)>,
    pub delete: Vec<PathBuf>,
    pub copy: Vec<String>,
    /// Stage every working-tree and untracked change.
    pub stage_all: bool,
    /// Switch between grouped sections and one flat list.
    pub toggle_list: bool,
    /// Store a user-picked base ref; `clear_base` restores the upstream.
    pub set_base: Option<String>,
    pub clear_base: bool,
    pub refresh_compare: bool,
    /// Open the conflict resolver for this file.
    pub resolve: Option<PathBuf>,
    /// Open the Compare popup for this file.
    pub compare: Option<(PathBuf, crate::revision_compare::PickerMode)>,
    /// Open the Commit Log at this commit (a History row click).
    pub log_at: Option<String>,
    /// Branch and sync actions, in click order.
    pub(crate) branch: Vec<crate::git::branch_ui::BranchRequest>,
    /// Ask a live agent to draft the commit message (Ask agent, no Enter).
    pub draft_message: bool,
}

/// Map a Git row click to the concrete action `App` performs. Conflicts open
/// as files; a deleted row with no staged side is a no-op; otherwise the diff
/// viewer preference picks native vs Neovim (Neovim only when the daemon
/// advertises it).
pub(crate) struct GitClickActionInput {
    pub deleted: bool,
    pub staged: Option<bool>,
    pub clicked: bool,
    pub review_mode: ReviewMode,
    pub neovim_review: bool,
}

pub fn git_click_action(input: GitClickActionInput) -> Option<FileAction> {
    let GitClickActionInput {
        deleted,
        staged,
        clicked,
        review_mode,
        neovim_review,
    } = input;
    if !clicked {
        return None;
    }
    let Some(staged) = staged else {
        return if deleted {
            None
        } else {
            Some(FileAction::Open)
        };
    };
    if review_mode != ReviewMode::Neovim || !neovim_review {
        return Some(if staged {
            FileAction::NativeStagedDiff
        } else {
            FileAction::NativeWorkingDiff
        });
    }
    Some(if staged {
        FileAction::StagedDiff
    } else {
        FileAction::WorkingDiff
    })
}

pub fn git_panel(ui: &mut egui::Ui, input: &mut GitPanelInput) -> GitPanelOutcome {
    let mut outcome = GitPanelOutcome::default();
    let error = input.context.error.clone();
    let owned_prepared;
    let prepared = if let Some(prepared) = input.prepared {
        prepared
    } else {
        owned_prepared = PreparedGit::new(input.context);
        &owned_prepared
    };
    if input.context.root.is_none() {
        git_not_a_repository(ui, input);
        if let Some(error) = &error {
            git_error_row(ui, input.theme, error, &mut outcome);
        }
    } else {
        // The Commit tab shows a refresh error above its changes.
        GitPanelPass {
            input,
            prepared,
            outcome: &mut outcome,
        }
        .repository(ui);
    }
    outcome
}

fn git_not_a_repository(ui: &mut egui::Ui, input: &GitPanelInput<'_>) {
    ui.weak("Not a Git repository or Perforce workspace");
    if let Some(error) = &input.context.error {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
}

/// One paint of the panel for a repository root.
struct GitPanelPass<'a, 'b> {
    input: &'a mut GitPanelInput<'b>,
    prepared: &'a PreparedGit,
    outcome: &'a mut GitPanelOutcome,
}

impl GitPanelPass<'_, '_> {
    /// The tab header, then the open tab: the Commit tab's icon row,
    /// changes and commit area, the Stash list, or the History log.
    fn repository(&mut self, ui: &mut egui::Ui) {
        let branch_name = self.input.context.branch.clone();
        git_header(GitHeaderInput {
            ui,
            input: self.input,
            branch_name: &branch_name,
            outcome: self.outcome,
        });
        ui.separator();
        sync_progress::sync_progress(ui, self.input.context.root.as_deref());
        if let Some(error) = self.input.list_error {
            ui.add(
                egui::Label::new(
                    RichText::new(error)
                        .small()
                        .color(ui.visuals().error_fg_color),
                )
                .truncate(),
            )
            .on_hover_text(error);
        }
        match self.input.tab {
            GitTab::History => self.history(ui),
            GitTab::Stash => self.stash_tab(ui),
            // Painted after the panel by `App::pull_request_tab`.
            GitTab::PullRequest => {}
            GitTab::Commit => {
                git_actions(GitActionsInput {
                    ui,
                    input: self.input,
                    prepared: self.prepared,
                    outcome: self.outcome,
                });
                self.commit_tab(ui);
            }
        }
    }

    fn history(&mut self, ui: &mut egui::Ui) {
        appearance::sidebar_scroll("git-log").show(ui, |ui| {
            if self.input.commits.is_empty() {
                ui.weak("No commits");
            }
            for (hash, subject) in self.input.commits {
                let row = appearance::row(appearance::RowInput {
                    ui,
                    label: subject,
                    icon: "GitBranch",
                    selected: false,
                    height: 22.0,
                    trailing: hash,
                    tint: appearance::color(&self.input.theme.secondary),
                });
                crate::shortcuts::tool_row_focus(ui, &row);
                if row.clicked() {
                    self.outcome.log_at = Some(hash.clone());
                }
                let lock = self.input.commit_lock.head;
                let outcome = &mut *self.outcome;
                appearance::context_menu(&row, |ui| match commit_actions(ui, hash, lock) {
                    Some(CommitPick::Copy(text)) => outcome.copy.push(text),
                    Some(CommitPick::Branch(request)) => outcome.branch.push(request),
                    None => {}
                });
            }
        });
    }

    /// Changed files as one flat list or grouped sections, then the files
    /// committed on the branch.
    fn changes(&mut self, ui: &mut egui::Ui) {
        let input = &*self.input;
        let root = input.context.root.as_deref();
        let row_ctx = GitRowCtx {
            theme: input.theme,
            root,
            review_mode: input.review_mode,
            neovim_review: input.neovim_review,
            open_shortcut: input.open_shortcut,
            stats: &input.context.stats,
            lock: input.commit_lock,
            generation: input.collapse_generation,
            open: input.tree_open,
        };
        if input.view_list {
            self.flat_list(ui, &row_ctx);
            return;
        }
        for group in &self.prepared.groups {
            git_group_section(GitGroupSectionInput {
                ui,
                ctx: &row_ctx,
                outcome: self.outcome,
                root,
                prepared: group,
            });
        }
        if let Some(compare) = input.compare
            && !compare.files.is_empty()
        {
            git_committed_section(GitCommittedSectionInput {
                ui,
                ctx: &row_ctx,
                outcome: self.outcome,
                root,
                files: &compare.files,
            });
        }
    }

    fn flat_list(&mut self, ui: &mut egui::Ui, row_ctx: &GitRowCtx<'_>) {
        let rows = self.prepared.groups.iter().flat_map(|prepared| {
            prepared
                .entries
                .iter()
                .zip(&prepared.labels)
                .map(|(change, label)| (prepared.group, change, label))
        });
        let outcome = &mut *self.outcome;
        // The name, then its folder dimmed (`JetBrains` flat view).
        tree::cull_rows(ui, rows, |ui, (group, change, label)| {
            let (folder, name) = split_label(label);
            git_change_row(GitChangeRowInput {
                ui,
                ctx: row_ctx,
                outcome,
                change,
                group,
                label: name,
                hint: folder,
            });
        });
    }
}

/// Shared view references for a Git file row.
struct GitRowCtx<'a> {
    theme: &'a AppearanceConfig,
    root: Option<&'a Path>,
    review_mode: ReviewMode,
    neovim_review: bool,
    open_shortcut: &'a str,
    stats: &'a std::collections::HashMap<PathBuf, (u32, u32)>,
    lock: CommitLock,
    /// Expand/collapse-all generation: a new one resets every section and
    /// folder to `open`.
    generation: u64,
    open: bool,
}

fn git_group_label(group: terminator_git::GitGroup) -> &'static str {
    use terminator_git::GitGroup;
    match group {
        GitGroup::Conflicts => "Conflicts",
        GitGroup::Staged => "Staged",
        GitGroup::Changes => "Changes",
        GitGroup::Untracked => "Untracked Files",
    }
}
