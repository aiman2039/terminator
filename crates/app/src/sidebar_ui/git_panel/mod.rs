//! The Git sidebar panel.
mod commit;
mod committed;
mod rows;
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
use committed::*;
use eframe::egui::{self, Color32, RichText};
#[cfg(test)]
pub(super) use rows::GIT_TREE_ROW_HEIGHT;
use rows::*;
use std::path::{Path, PathBuf};
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
    pub history: bool,
    pub view_list: bool,
    pub commits: &'a [(String, String)],
    pub shelves: &'a [(String, String)],
    pub branches: &'a [String],
    pub commit_draft: &'a mut String,
    /// Recent commit messages, newest first.
    pub recent_messages: &'a [String],
    pub amend: &'a mut bool,
    pub signoff: &'a mut bool,
    pub collapse_generation: u64,
    pub open_shortcut: &'a str,
    /// Branch comparison for the current root, when available.
    pub compare: Option<&'a workspace_ops::CompareData>,
    /// User-picked base ref; `None` means the upstream branch is used.
    pub base_ref: Option<&'a str>,
    /// A commit is in flight; commit controls are disabled until it reports.
    pub committing: bool,
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
    pub history: Option<bool>,
    pub collapse: bool,
    pub commit: bool,
    pub commit_push: bool,
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
    } else {
        let mut pass = GitPanelPass {
            input,
            prepared,
            outcome: &mut outcome,
        };
        // History mode skips the error row below.
        if !pass.repository(ui) {
            return outcome;
        }
    }
    if let Some(e) = &error {
        ui.horizontal(|ui| {
            ui.colored_label(appearance::color(&input.theme.status_failed), e);
            if ui.small_button("Retry").clicked() {
                outcome.refresh = true;
            }
        });
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
    /// Toolbar, compare line, then the log (history mode) or the commit
    /// box and changes. Returns false in history mode.
    fn repository(&mut self, ui: &mut egui::Ui) -> bool {
        let branch_name = self.input.context.branch.clone();
        git_toolbar(GitToolbarInput {
            ui,
            input: self.input,
            branch_name: &branch_name,
            outcome: self.outcome,
        });
        if !self.input.history
            && let Some(compare) = self.input.compare
        {
            git_compare_row(GitCompareRowInput {
                ui,
                theme: self.input.theme,
                branch: &branch_name,
                compare,
                base_ref: self.input.base_ref,
            });
        }
        if self.input.history {
            self.history(ui);
            return false;
        }
        self.commit_box(ui);
        if self.input.context.changes.is_empty() {
            ui.weak("Working tree clean");
        }
        appearance::sidebar_scroll("git").show(ui, |ui| self.changes(ui));
        true
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
                if row.clicked() {
                    self.outcome.view_log = true;
                }
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
                collapse_generation: input.collapse_generation,
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
                collapse_generation: input.collapse_generation,
            });
        }
    }

    fn flat_list(&mut self, ui: &mut egui::Ui, row_ctx: &GitRowCtx<'_>) {
        let mut any = false;
        for prepared in &self.prepared.groups {
            for (change, label) in prepared.entries.iter().zip(&prepared.labels) {
                any = true;
                git_change_row(GitChangeRowInput {
                    ui,
                    ctx: row_ctx,
                    outcome: self.outcome,
                    change,
                    group: prepared.group,
                    label,
                });
            }
        }
        if !any {
            ui.weak("Working tree clean");
        }
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
}

fn git_group_label(group: terminator_git::GitGroup) -> &'static str {
    use terminator_git::GitGroup;
    match group {
        GitGroup::Conflicts => "CONFLICTS",
        GitGroup::Staged => "STAGED",
        GitGroup::Changes => "CHANGES",
        GitGroup::Untracked => "UNTRACKED FILES",
    }
}
