use super::explorer_rows::skip_clipped_row;
#[cfg(feature = "test-support")]
use crate::diagnostics;
use crate::{
    App, appearance, file_actions::FileAction, icons, services::ContextData, workspace_ops,
};
use eframe::egui::{self, Color32, RichText};
use std::path::{Path, PathBuf};
use terminator_core::ReviewMode;
use terminator_core::appearance::AppearanceConfig;

pub(super) fn skip_clipped_git_row(ui: &mut egui::Ui) -> bool {
    skip_clipped_row(ui, GIT_TREE_ROW_HEIGHT)
}

impl App {
    pub(super) fn ensure_git_lists(&mut self, root: Option<std::path::PathBuf>) {
        let Some(root) = root else {
            return;
        };
        if self.git_list_root.as_ref() == Some(&root) {
            return;
        }
        self.git_list_root = Some(root);
        self.git_branches.clear();
        self.git_log.clear();
        self.git_compare = None;
        self.git_compare_root = None;
        self.git_base_ref = None;
        self.queue_workspace(workspace_ops::Op::Branches);
        self.queue_workspace(workspace_ops::Op::Log);
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
    pub branches: &'a [String],
    pub commit_draft: &'a mut String,
    pub collapse_generation: u64,
    pub open_shortcut: &'a str,
    /// Branch comparison for the current root, when available.
    pub compare: Option<&'a workspace_ops::CompareData>,
    /// User-picked base ref; `None` means the upstream branch is used.
    pub base_ref: Option<&'a str>,
}

/// A concrete file action with its target, ready for `App` to perform.
pub struct GitFileAction {
    pub action: FileAction,
    pub path: PathBuf,
}

/// What the Git panel asks `App` to do after painting.
#[derive(Default)]
pub struct GitPanelOutcome {
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
}

/// Map a Git row click to the concrete action `App` performs. Conflicts open
/// as files; a deleted row with no staged side is a no-op; otherwise the diff
/// viewer preference picks native vs Neovim (Neovim only when the daemon
/// advertises it).
pub fn git_click_action(
    deleted: bool,
    staged: Option<bool>,
    clicked: bool,
    review_mode: ReviewMode,
    neovim_review: bool,
) -> Option<FileAction> {
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
        ui.weak("Not a Git repository");
    } else {
        git_toolbar(ui, input, &branch_name, &mut outcome);
        if !input.history
            && let Some(compare) = input.compare
        {
            git_compare_row(ui, input.theme, &branch_name, compare, input.base_ref);
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
                    .desired_rows(3),
            );
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
                let commit = ui.add_enabled(
                    staged && !input.commit_draft.trim().is_empty(),
                    egui::Button::new("Commit"),
                );
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "git-commit", commit.rect);
                if commit.clicked() {
                    outcome.commit = true;
                }
            });
        }
        if input.history {
            appearance::sidebar_scroll("git-log").show(ui, |ui| {
                if input.commits.is_empty() {
                    ui.weak("No commits");
                }
                for (hash, subject) in input.commits {
                    let row = appearance::row(
                        ui,
                        subject,
                        "GitBranch",
                        false,
                        22.0,
                        hash,
                        appearance::color(&input.theme.secondary),
                    );
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
                        git_change_row(ui, &row_ctx, &mut outcome, change, prepared.group, label);
                    }
                }
                if !any {
                    ui.weak("Working tree clean");
                }
            } else {
                for group in &prepared.groups {
                    git_group_section(
                        ui,
                        &row_ctx,
                        &mut outcome,
                        root.as_deref(),
                        group,
                        input.collapse_generation,
                    );
                }
                if let Some(compare) = input.compare
                    && !compare.files.is_empty()
                {
                    git_committed_section(
                        ui,
                        &row_ctx,
                        &mut outcome,
                        root.as_deref(),
                        &compare.files,
                        input.collapse_generation,
                    );
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

/// Inline width of the trailing compare/view actions (list toggle, base-ref
/// picker, compare refresh) when they sit beside the log button: three icon
/// slots plus their gaps.
const GIT_MORE_INLINE_WIDTH: f32 = 3.0 * appearance::TOOLBAR_BUTTON + 2.0 * 4.0;

/// Base-ref picker shared by the inline toolbar button and the overflow menu:
/// automatic upstream plus every known branch.
fn git_base_ref_contents(
    ui: &mut egui::Ui,
    input: &GitPanelInput<'_>,
    outcome: &mut GitPanelOutcome,
) {
    let auto = if input.base_ref.is_none() { "✓" } else { "" };
    if appearance::menu_item(ui, "Automatic (upstream)", "GitCompareArrows", auto).clicked() {
        outcome.clear_base = true;
        ui.close();
    }
    ui.separator();
    if input.branches.is_empty() {
        ui.weak("No branches");
    }
    for name in input.branches {
        let mark = if input.base_ref == Some(name.as_str()) {
            "✓"
        } else {
            ""
        };
        if appearance::menu_item(ui, name, "GitBranch", mark).clicked() {
            outcome.set_base = Some(name.clone());
            ui.close();
        }
    }
}

/// Trailing compare/view actions as inline toolbar icons, used when the
/// sidebar has room. Falls back to [`git_overflow_more`] when narrow.
fn git_inline_more(ui: &mut egui::Ui, input: &GitPanelInput<'_>, outcome: &mut GitPanelOutcome) {
    let list = appearance::selectable_icon(ui, "List", "View as list", input.view_list);
    #[cfg(feature = "test-support")]
    diagnostics::record(ui.ctx(), "git-view-list", list.rect);
    if list.clicked() {
        outcome.toggle_list = true;
    }
    let base = appearance::icon_menu_button(ui, "GitCompareArrows", |ui| {
        git_base_ref_contents(ui, input, outcome);
    })
    .response
    .on_hover_text("Change base ref");
    #[cfg(feature = "test-support")]
    diagnostics::record(ui.ctx(), "git-base-ref", base.rect);
    let _ = base;
    let compare = appearance::sidebar_action(ui, "RefreshCw", "Refresh branch compare");
    #[cfg(feature = "test-support")]
    diagnostics::record(ui.ctx(), "git-compare-refresh", compare.rect);
    if compare.clicked() {
        outcome.refresh_compare = true;
    }
}

/// Trailing compare/view actions behind the `…` menu, used when the sidebar
/// is too narrow for the inline icons.
fn git_overflow_more(ui: &mut egui::Ui, input: &GitPanelInput<'_>, outcome: &mut GitPanelOutcome) {
    let more = appearance::compact_menu_button(ui, "…", |ui| {
        let mark = if input.view_list { "✓" } else { "" };
        if appearance::menu_item(ui, "View as list", "List", mark).clicked() {
            outcome.toggle_list = true;
            ui.close();
        }
        let base = appearance::menu_button(ui, "Change Base Ref…", |ui| {
            git_base_ref_contents(ui, input, outcome);
        });
        let _ = base;
        ui.separator();
        if appearance::menu_item(ui, "Refresh branch compare", "RefreshCw", "").clicked() {
            outcome.refresh_compare = true;
            ui.close();
        }
    })
    .response
    .on_hover_text("More Git actions");
    #[cfg(feature = "test-support")]
    diagnostics::record(ui.ctx(), "git-more", more.rect);
    let _ = more;
}

/// Icon toolbar: collapse, Changes/History, branch switcher, refresh, log,
/// then the compare/view actions inline when the sidebar has room, or behind
/// the overflow menu when narrow.
fn git_toolbar(
    ui: &mut egui::Ui,
    input: &GitPanelInput<'_>,
    branch_name: &str,
    outcome: &mut GitPanelOutcome,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.spacing_mut().interact_size.y = appearance::TOOLBAR_BUTTON;
        let collapse = appearance::sidebar_action(ui, "ChevronsDownUp", "Collapse all");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "git-collapse", collapse.rect);
        if collapse.clicked() {
            outcome.collapse = true;
        }
        let changes = appearance::selectable_icon(ui, "FileDiff", "Changes", !input.history);
        let history = appearance::selectable_icon(ui, "History", "History", input.history);
        #[cfg(feature = "test-support")]
        {
            diagnostics::record(ui.ctx(), "git-changes", changes.rect);
            diagnostics::record(ui.ctx(), "git-history", history.rect);
        }
        if changes.clicked() {
            outcome.history = Some(false);
        }
        if history.clicked() {
            outcome.history = Some(true);
        }
        let branch = appearance::text_menu_button(ui, branch_name, |ui| {
            if input.branches.is_empty() {
                ui.weak("No branches");
            }
            for name in input.branches {
                let mark = if name == branch_name { "✓" } else { "" };
                if appearance::menu_item(ui, name, "GitBranch", mark).clicked() {
                    outcome.switch = Some(name.clone());
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("Switch branch");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "git-branch", branch.rect);
        let _ = branch;
        let refresh = appearance::sidebar_action(ui, "RefreshCw", "Refresh");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "git-refresh", refresh.rect);
        if refresh.clicked() {
            outcome.refresh = true;
        }
        let log = appearance::sidebar_action(ui, "ListTree", "View commit log");
        if log.clicked() {
            outcome.view_log = true;
        }
        if ui.available_width() >= GIT_MORE_INLINE_WIDTH {
            git_inline_more(ui, input, outcome);
        } else {
            git_overflow_more(ui, input, outcome);
        }
    });
}

/// `branch → base` with ahead/behind counts, the Orca branch compare line.
fn git_compare_row(
    ui: &mut egui::Ui,
    theme: &AppearanceConfig,
    branch: &str,
    compare: &workspace_ops::CompareData,
    base_ref: Option<&str>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.add(egui::Label::new(RichText::new(branch).monospace().strong()).truncate());
        let target = base_ref
            .or(compare.base.as_deref())
            .or(compare.upstream.as_deref());
        if let Some(target) = target {
            ui.add(
                egui::Label::new(RichText::new(format!("→ {target}")).monospace().weak())
                    .truncate(),
            )
            .on_hover_text("Base ref for the branch comparison");
        }
        if compare.ahead > 0 {
            ui.label(
                RichText::new(format!("↑{}", compare.ahead))
                    .color(appearance::color(&theme.git_added)),
            )
            .on_hover_text("Commits ahead of the base ref");
        }
        if compare.behind > 0 {
            ui.label(RichText::new(format!("↓{}", compare.behind)).weak())
                .on_hover_text("Commits behind the base ref");
        }
    });
}

/// One changed file: diffstat, status letter, click-to-diff, and context menu.
fn git_change_row(
    ui: &mut egui::Ui,
    ctx: &GitRowCtx<'_>,
    outcome: &mut GitPanelOutcome,
    change: &terminator_git::Change,
    group: terminator_git::GitGroup,
    label: &str,
) {
    // Keep layout height without constructing thousands of off-screen widgets.
    if skip_clipped_git_row(ui) {
        return;
    }
    let letter = change.letter(group);
    let trailing = match ctx.stats.get(&change.path).copied() {
        Some((added, deleted)) if deleted > 0 => format!("+{added} -{deleted} {letter}"),
        Some((added, _)) => format!("+{added} {letter}"),
        None => letter.to_string(),
    };
    let response = appearance::file_row(
        ui,
        label,
        icons::file_icon(&change.path),
        false,
        GIT_TREE_ROW_HEIGHT,
        &trailing,
        git_color(ctx.theme, letter),
    )
    .on_hover_text(format!(
        "{}\n{} ({})",
        change.path.display(),
        terminator_git::status_description(letter),
        group.label()
    ));
    #[cfg(feature = "test-support")]
    diagnostics::record(
        ui.ctx(),
        &format!(
            "git-file-{}",
            change
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
        ),
        response.rect,
    );
    let staged = (!change.conflict()).then_some(group == terminator_git::GitGroup::Staged);
    if let Some(action) = git_click_action(
        letter == 'D',
        staged,
        response.clicked() || response.double_clicked(),
        ctx.review_mode,
        ctx.neovim_review,
    ) {
        outcome.clicked.push(GitFileAction {
            action,
            path: change.path.clone(),
        });
    }
    appearance::context_menu(&response, |ui| {
        let open_changes = git_click_action(
            letter == 'D',
            staged,
            true,
            ctx.review_mode,
            ctx.neovim_review,
        );
        if let Some(action) = open_changes
            && appearance::menu_item(ui, "Open changes", "FileDiff", "").clicked()
        {
            outcome.menu.push(GitFileAction {
                action,
                path: change.path.clone(),
            });
            ui.close();
        }
        if appearance::menu_item(ui, "Open file", "FileCode", ctx.open_shortcut).clicked() {
            outcome.menu.push(GitFileAction {
                action: FileAction::Open,
                path: change.path.clone(),
            });
            ui.close();
        }
        ui.separator();
        if group == terminator_git::GitGroup::Staged {
            if appearance::menu_item(ui, "Unstage", "ArrowLeft", "").clicked() {
                outcome.unstage.push(change.path.clone());
                ui.close();
            }
        } else if appearance::menu_item(ui, "Stage", "Plus", "").clicked() {
            outcome.stage.push(change.path.clone());
            ui.close();
        }
        if appearance::menu_item(ui, "Discard changes", "Eraser", "").clicked() {
            outcome.discard.push((
                change.path.clone(),
                group == terminator_git::GitGroup::Untracked,
            ));
            ui.close();
        }
        ui.separator();
        if appearance::menu_item(ui, "Copy path", "Copy", "").clicked() {
            outcome.copy.push(change.path.display().to_string());
            ui.close();
        }
        if let Some(root) = ctx.root
            && appearance::menu_item(ui, "Copy relative path", "Copy", "").clicked()
        {
            outcome
                .copy
                .push(workspace_ops::relative_display(root, &change.path));
            ui.close();
        }
        ui.separator();
        if appearance::menu_item(ui, "Delete file", "X", "").clicked() {
            outcome.delete.push(change.path.clone());
            ui.close();
        }
    });
}

const GIT_SECTION_LIMIT: usize = 8;

/// Per-level indent and row height for the changed-file tree. Kept tight so a
/// deep path stays legible in the narrow sidebar.
const GIT_TREE_INDENT: f32 = 12.0;

pub(super) const GIT_TREE_ROW_HEIGHT: f32 = 20.0;

/// Orca-style section header with per-section stage/unstage all. The body is a
/// directory tree (JetBrains-style), collapsible per folder.
#[allow(clippy::too_many_arguments)]
fn git_group_section(
    ui: &mut egui::Ui,
    ctx: &GitRowCtx<'_>,
    outcome: &mut GitPanelOutcome,
    root: Option<&Path>,
    prepared: &PreparedGroup,
    collapse_generation: u64,
) {
    let group = prepared.group;
    let entries = &prepared.entries;
    let label = git_group_label(group);
    let total = entries.len();
    let id = ui.make_persistent_id(("git-section", root, label, collapse_generation));
    let state = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        id,
        collapse_generation == 0,
    );
    let _ = state
        .show_header(ui, |ui| {
            ui.add(
                egui::Label::new(RichText::new(format!("{label} {total}")).small().strong())
                    .selectable(false),
            );
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| match group {
                    terminator_git::GitGroup::Staged => {
                        let action = appearance::sidebar_action(ui, "ArrowLeft", "Unstage all");
                        if action.clicked() {
                            for change in entries {
                                outcome.unstage.push(change.path.clone());
                            }
                        }
                    }
                    terminator_git::GitGroup::Changes | terminator_git::GitGroup::Untracked => {
                        let action = appearance::sidebar_action(ui, "Plus", "Stage all");
                        if action.clicked() {
                            for change in entries {
                                outcome.stage.push(change.path.clone());
                            }
                        }
                    }
                    terminator_git::GitGroup::Conflicts => {}
                },
            );
        })
        .body(|ui| {
            let tree = &prepared.tree;
            ui.spacing_mut().indent = GIT_TREE_INDENT;
            ui.spacing_mut().item_spacing.y = 0.0;
            git_change_tree(ui, ctx, outcome, tree, "", group);
        });
}

/// Prepared once per accepted filesystem/Git result, reused while scrolling.
pub(crate) struct PreparedGit {
    groups: Vec<PreparedGroup>,
}

struct PreparedGroup {
    group: terminator_git::GitGroup,
    entries: Vec<terminator_git::Change>,
    labels: Vec<String>,
    tree: ChangeTree,
}

impl PreparedGit {
    pub fn new(context: &ContextData) -> Self {
        let groups = terminator_git::GitGroup::ALL
            .into_iter()
            .filter_map(|group| {
                let entries: Vec<_> = context
                    .changes
                    .iter()
                    .filter(|c| c.in_group(group))
                    .cloned()
                    .collect();
                if entries.is_empty() {
                    return None;
                }
                let labels = entries
                    .iter()
                    .map(|c| {
                        workspace_ops::relative_display(
                            context.root.as_deref().unwrap_or(&c.path),
                            &c.path,
                        )
                    })
                    .collect();
                let tree =
                    build_change_tree(&entries.iter().collect::<Vec<_>>(), context.root.as_deref());
                Some(PreparedGroup {
                    group,
                    entries,
                    labels,
                    tree,
                })
            })
            .collect();
        Self { groups }
    }
}

#[derive(Default)]
pub(super) struct ChangeTree {
    pub(super) dirs: std::collections::BTreeMap<String, ChangeTree>,
    pub(super) files: Vec<terminator_git::Change>,
    labels: Vec<String>,
    count: usize,
}

impl ChangeTree {
    pub(super) fn count_files(&self) -> usize {
        self.count
    }
    pub(super) fn collect<'a>(&'a self, out: &mut Vec<&'a terminator_git::Change>) {
        out.extend(self.files.iter());
        for dir in self.dirs.values() {
            dir.collect(out);
        }
    }
}

pub(super) fn build_change_tree(
    entries: &[&terminator_git::Change],
    root: Option<&Path>,
) -> ChangeTree {
    fn insert(node: &mut ChangeTree, dirs: &[String], change: &terminator_git::Change) {
        node.count = node.count.saturating_add(1);
        match dirs.split_first() {
            None => {
                node.files.push(change.clone());
                node.labels.push(
                    change
                        .path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                );
            }
            Some((dir, rest)) => insert(node.dirs.entry(dir.clone()).or_default(), rest, change),
        }
    }
    let mut tree = ChangeTree::default();
    for change in entries {
        let relative = root
            .and_then(|root| change.path.strip_prefix(root).ok())
            .unwrap_or(&change.path);
        let components: Vec<String> = relative
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect();
        if !components.is_empty() {
            let parent = components.len().saturating_sub(1);
            if let Some(dirs) = components.get(..parent) {
                insert(&mut tree, dirs, change);
            }
        }
    }
    tree
}

fn git_change_tree(
    ui: &mut egui::Ui,
    ctx: &GitRowCtx<'_>,
    outcome: &mut GitPanelOutcome,
    node: &ChangeTree,
    prefix: &str,
    group: terminator_git::GitGroup,
) {
    for (name, child) in &node.dirs {
        let key = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        let id = ui.make_persistent_id(("git-tree", key.clone(), group.label()));
        let count = child.count_files();
        let mut folder_response = None;
        let mut header =
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true)
                .show_header(ui, |ui| {
                    let trailing = count.to_string();
                    folder_response = Some(appearance::file_row(
                        ui,
                        name,
                        "Folder",
                        false,
                        GIT_TREE_ROW_HEIGHT,
                        &trailing,
                        appearance::color(&ctx.theme.secondary),
                    ));
                });
        if folder_response
            .as_ref()
            .is_some_and(eframe::egui::Response::clicked)
        {
            header.toggle();
        }
        let _ = header.body(|ui| git_change_tree(ui, ctx, outcome, child, &key, group));
        if let Some(response) = folder_response {
            let response = response.on_hover_text(format!("{key} ({count} changed)"));
            appearance::context_menu(&response, |ui| match group {
                terminator_git::GitGroup::Staged => {
                    if appearance::menu_item(ui, "Unstage folder", "ArrowLeft", "").clicked() {
                        let mut files = Vec::new();
                        child.collect(&mut files);
                        for change in files {
                            outcome.unstage.push(change.path.clone());
                        }
                        ui.close();
                    }
                }
                terminator_git::GitGroup::Changes | terminator_git::GitGroup::Untracked => {
                    if appearance::menu_item(ui, "Stage folder", "Plus", "").clicked() {
                        let mut files = Vec::new();
                        child.collect(&mut files);
                        for change in files {
                            outcome.stage.push(change.path.clone());
                        }
                        ui.close();
                    }
                }
                terminator_git::GitGroup::Conflicts => {}
            });
        }
    }
    for (change, label) in node.files.iter().zip(&node.labels) {
        git_change_row(ui, ctx, outcome, change, group, label);
    }
}

/// Files changed since the base ref, grouped under "COMMITTED ON BRANCH".
fn git_committed_section(
    ui: &mut egui::Ui,
    ctx: &GitRowCtx<'_>,
    outcome: &mut GitPanelOutcome,
    root: Option<&Path>,
    files: &[workspace_ops::CommittedFile],
    collapse_generation: u64,
) {
    let total = files.len();
    let id = ui.make_persistent_id(("git-committed", root, total, collapse_generation));
    let view_all_id =
        ui.make_persistent_id(("git-committed-all", root, total, collapse_generation));
    let mut expanded = ui
        .ctx()
        .data_mut(|data| data.get_temp::<bool>(view_all_id).unwrap_or(false));
    let state = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        id,
        collapse_generation == 0,
    );
    let _ = state
        .show_header(ui, |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(format!("COMMITTED ON BRANCH {total}"))
                        .small()
                        .strong(),
                )
                .selectable(false),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if total > GIT_SECTION_LIMIT {
                    let text = if expanded { "Show less" } else { "View all" };
                    if ui.small_button(text).clicked() {
                        expanded = !expanded;
                        ui.ctx()
                            .data_mut(|data| data.insert_temp(view_all_id, expanded));
                    }
                }
            });
        })
        .body(|ui| {
            let shown = if expanded {
                total
            } else {
                total.min(GIT_SECTION_LIMIT)
            };
            for file in files.iter().take(shown) {
                git_committed_row(ui, ctx, outcome, file);
            }
        });
}

/// One file changed since the base ref. It has no index/worktree side, so the
/// only useful actions are open and copy.
fn git_committed_row(
    ui: &mut egui::Ui,
    ctx: &GitRowCtx<'_>,
    outcome: &mut GitPanelOutcome,
    file: &workspace_ops::CommittedFile,
) {
    if skip_clipped_git_row(ui) {
        return;
    }
    let name = file
        .path
        .strip_prefix(ctx.root.unwrap_or(&file.path))
        .unwrap_or(&file.path)
        .display()
        .to_string();
    let trailing = if file.deleted > 0 {
        format!("+{} -{} {}", file.added, file.deleted, file.letter)
    } else {
        format!("+{} {}", file.added, file.letter)
    };
    let response = appearance::file_row(
        ui,
        &name,
        icons::file_icon(&file.path),
        false,
        24.0,
        &trailing,
        git_color(ctx.theme, file.letter),
    )
    .on_hover_text(file.path.display().to_string());
    #[cfg(feature = "test-support")]
    diagnostics::record(
        ui.ctx(),
        &format!(
            "git-committed-{}",
            file.path.file_name().unwrap_or_default().to_string_lossy()
        ),
        response.rect,
    );
    if response.clicked() || response.double_clicked() {
        outcome.menu.push(GitFileAction {
            action: FileAction::Open,
            path: file.path.clone(),
        });
    }
    appearance::context_menu(&response, |ui| {
        if appearance::menu_item(ui, "Open file", "FileCode", ctx.open_shortcut).clicked() {
            outcome.menu.push(GitFileAction {
                action: FileAction::Open,
                path: file.path.clone(),
            });
            ui.close();
        }
        if appearance::menu_item(ui, "Copy path", "Copy", "").clicked() {
            outcome.copy.push(file.path.display().to_string());
            ui.close();
        }
    });
}
