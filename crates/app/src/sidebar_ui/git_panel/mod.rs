//! The Git sidebar panel.
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
    let branch_name = input.context.branch.clone();
    let root = input.context.root.clone();
    let context = input.context;
    let changes = &context.changes;
    let error = input.context.error.clone();
    let owned_prepared;
    let prepared = if let Some(prepared) = input.prepared {
        prepared
    } else {
        owned_prepared = PreparedGit::new(context);
        &owned_prepared
    };
    let stats = &context.stats;
    if root.is_none() {
        ui.weak("Not a Git repository or Perforce workspace");
        if let Some(error) = &input.context.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
    } else {
        git_toolbar(ui, input, &branch_name, &mut outcome);
        if !input.history
            && let Some(compare) = input.compare
        {
            git_compare_row(GitCompareRowInput {
                ui,
                theme: input.theme,
                branch: &branch_name,
                compare,
                base_ref: input.base_ref,
            });
        }
        let staged = prepared
            .groups
            .iter()
            .any(|group| group.group == terminator_git::GitGroup::Staged);
        if !input.history {
            ui.add(
                egui::TextEdit::multiline(input.commit_draft)
                    .hint_text("Message")
                    .desired_width(ui.available_width())
                    .desired_rows(4),
            );
            // Subject length guide: Git tools truncate long first lines.
            let subject = input
                .commit_draft
                .lines()
                .next()
                .map_or(0, |line| line.chars().count());
            if subject > 0 {
                let text = RichText::new(format!("Subject {subject}/72")).small();
                if subject > 72 {
                    ui.colored_label(ui.visuals().warn_fg_color, text);
                } else {
                    ui.weak(text);
                }
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let amend = ui
                    .checkbox(input.amend, "Amend")
                    .on_hover_text("Amend the last commit (loads its message)");
                if amend.changed() && *input.amend && input.commit_draft.trim().is_empty() {
                    outcome.load_head_message = true;
                }
                if !input.recent_messages.is_empty() {
                    appearance::icon_menu_button(ui, "History", |ui| {
                        for message in input.recent_messages {
                            let subject = message.lines().next().unwrap_or_default();
                            if ui
                                .add(egui::Button::new(subject).truncate())
                                .on_hover_text(message.as_str())
                                .clicked()
                            {
                                message.clone_into(input.commit_draft);
                                ui.close();
                            }
                        }
                    })
                    .response
                    .on_hover_text("Recent commit messages");
                }
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "git-amend", amend.rect);
                let signoff = ui
                    .checkbox(input.signoff, "Sign-off")
                    .on_hover_text("Add a Signed-off-by trailer");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "git-signoff", signoff.rect);
                #[cfg(not(feature = "test-support"))]
                let _ = (&amend, &signoff);
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let stage_all = ui
                    .add_enabled(!changes.is_empty(), egui::Button::new("Stage All"))
                    .on_hover_text("Stage every change");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "git-stage-all", stage_all.rect);
                #[cfg(not(feature = "test-support"))]
                let _ = &stage_all;
                if stage_all.clicked() {
                    outcome.stage_all = true;
                }
                // Amending with an empty message keeps the previous message.
                let can_commit = !input.committing
                    && staged
                    && (*input.amend || !input.commit_draft.trim().is_empty());
                let commit = ui.add_enabled(can_commit, egui::Button::new("Commit"));
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "git-commit", commit.rect);
                if commit.clicked() {
                    outcome.commit = true;
                }
                let commit_push = ui.add_enabled(can_commit, egui::Button::new("Commit and Push"));
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "git-commit-push", commit_push.rect);
                if commit_push.clicked() {
                    outcome.commit_push = true;
                }
                if input.committing {
                    ui.spinner().on_hover_text("Committing…");
                }
                let shelve = ui
                    .add_enabled(staged, egui::Button::new("Shelve"))
                    .on_hover_text("Shelve staged changes (git stash)");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "git-shelve", shelve.rect);
                if shelve.clicked() {
                    outcome.shelve = true;
                }
            });
            if !input.shelves.is_empty() {
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("SHELVED {}", input.shelves.len()))
                            .small()
                            .strong(),
                    )
                    .selectable(false),
                );
                for (reference, message) in input.shelves {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.add(egui::Label::new(message).truncate())
                            .on_hover_text(format!("{reference} {message}"));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let drop = ui.small_button("Drop");
                            #[cfg(feature = "test-support")]
                            diagnostics::record(
                                ui.ctx(),
                                &format!("git-shelf-drop-{reference}"),
                                drop.rect,
                            );
                            if drop.clicked() {
                                outcome.drop_shelf = Some(reference.clone());
                            }
                            let unshelve = ui.small_button("Unshelve");
                            #[cfg(feature = "test-support")]
                            diagnostics::record(
                                ui.ctx(),
                                &format!("git-shelf-unshelve-{reference}"),
                                unshelve.rect,
                            );
                            if unshelve.clicked() {
                                outcome.unshelve = Some(reference.clone());
                            }
                        });
                    });
                }
            }
        }
        if input.history {
            appearance::sidebar_scroll("git-log").show(ui, |ui| {
                if input.commits.is_empty() {
                    ui.weak("No commits");
                }
                for (hash, subject) in input.commits {
                    let row = appearance::row(appearance::RowInput {
                        ui,
                        label: subject,
                        icon: "GitBranch",
                        selected: false,
                        height: 22.0,
                        trailing: hash,
                        tint: appearance::color(&input.theme.secondary),
                    });
                    if row.clicked() {
                        outcome.view_log = true;
                    }
                }
            });
            return outcome;
        }
        if changes.is_empty() {
            ui.weak("Working tree clean");
        }
        appearance::sidebar_scroll("git").show(ui, |ui| {
            let row_ctx = GitRowCtx {
                theme: input.theme,
                root: root.as_deref(),
                review_mode: input.review_mode,
                neovim_review: input.neovim_review,
                open_shortcut: input.open_shortcut,
                stats,
            };
            if input.view_list {
                let mut any = false;
                for prepared in &prepared.groups {
                    for (change, label) in prepared.entries.iter().zip(&prepared.labels) {
                        any = true;
                        git_change_row(GitChangeRowInput {
                            ui,
                            ctx: &row_ctx,
                            outcome: &mut outcome,
                            change,
                            group: prepared.group,
                            label,
                        });
                    }
                }
                if !any {
                    ui.weak("Working tree clean");
                }
            } else {
                for group in &prepared.groups {
                    git_group_section(GitGroupSectionInput {
                        ui,
                        ctx: &row_ctx,
                        outcome: &mut outcome,
                        root: root.as_deref(),
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
                        outcome: &mut outcome,
                        root: root.as_deref(),
                        files: &compare.files,
                        collapse_generation: input.collapse_generation,
                    });
                }
            }
        });
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
