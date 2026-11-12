//! The `JetBrains` Commit tool window header (Commit | Stash | History
//! tabs, branch switcher, ⋮ menu) and the icon row above the changes.
use super::*;

/// Base-ref picker in the ⋮ menu: automatic upstream plus every branch.
fn git_base_ref_contents(
    ui: &mut egui::Ui,
    input: &GitPanelInput<'_>,
    outcome: &mut GitPanelOutcome,
) {
    let auto = if input.base_ref.is_none() { "✓" } else { "" };
    if appearance::menu_item(appearance::MenuItemInput {
        ui,
        label: "Automatic (upstream)",
        icon: "GitCompareArrows",
        shortcut: auto,
    })
    .clicked()
    {
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
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: name,
            icon: "GitBranch",
            shortcut: mark,
        })
        .clicked()
        {
            outcome.set_base = Some(name.clone());
            ui.close();
        }
    }
}

/// The ⋮ menu: Fetch, Pull, Push, commit log, base ref, and branch
/// compare refresh.
fn git_more_menu(ui: &mut egui::Ui, input: &GitPanelInput<'_>, outcome: &mut GitPanelOutcome) {
    let more = appearance::icon_menu_button(ui, "EllipsisVertical", |ui| {
        super::branch_menu::sync_items(ui, input.commit_lock.head, outcome);
        ui.separator();
        let log = appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "View commit log",
            icon: "ListTree",
            shortcut: "",
        });
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "git-view-log", log.rect);
        if log.clicked() {
            outcome.view_log = true;
            ui.close();
        }
        let base = appearance::menu_button(ui, "Change Base Ref…", |ui| {
            git_base_ref_contents(ui, input, outcome);
        });
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "git-base-ref", base.response.rect);
        let _ = base;
        ui.separator();
        let compare = appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Refresh branch compare",
            icon: "RefreshCw",
            shortcut: "",
        });
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "git-compare-refresh", compare.rect);
        if compare.clicked() {
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

pub(super) struct GitHeaderInput<'a> {
    pub(super) ui: &'a mut egui::Ui,
    pub(super) input: &'a GitPanelInput<'a>,
    pub(super) branch_name: &'a str,
    pub(super) outcome: &'a mut GitPanelOutcome,
}

/// Narrowest the branch switcher may truncate to beside the tabs; below
/// this the branch side wraps to its own row.
const MIN_BRANCH_WIDTH: f32 = 64.0;

/// Tabs on the left; `↑N ↓N`, the branch switcher and the ⋮ menu on the
/// right. The tabs are laid out first and the branch side gets only the
/// width left: the branch name truncates, and when even a short name does
/// not fit, the branch side moves to its own row under the tabs. A commit
/// or push running in this worktree disables the branch switch.
pub(super) fn git_header(header: GitHeaderInput<'_>) {
    let GitHeaderInput {
        ui,
        input,
        branch_name,
        outcome,
    } = header;
    let wrapped = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            git_tabs(ui, input, outcome);
            let fits = branch_beside_tabs(BranchBudget {
                free: ui.available_width(),
                trailing: trailing_width(ui, input),
                branch: branch_width(ui, branch_name),
            });
            if fits {
                git_branch_side(ui, input, branch_name, outcome);
            }
            !fits
        })
        .inner;
    if wrapped {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            git_branch_side(ui, input, branch_name, outcome);
        });
    }
}

/// Commit | Stash | History.
fn git_tabs(ui: &mut egui::Ui, input: &GitPanelInput<'_>, outcome: &mut GitPanelOutcome) {
    let current = input.tab;
    for (tab, label, target) in [
        (GitTab::Commit, "Commit", "git-changes"),
        (GitTab::Stash, "Stash", "git-stash"),
        (GitTab::History, "History", "git-history"),
    ] {
        let response = appearance::text_tab(ui, label, current == tab);
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), target, response.rect);
        let _ = target;
        if response.clicked() && current != tab {
            outcome.tab = Some(tab);
        }
    }
}

/// Widths that decide whether the branch side shares the tab row.
#[derive(Clone, Copy)]
pub(super) struct BranchBudget {
    /// Width left on the row after the tabs.
    pub(super) free: f32,
    /// The ⋮ menu, `↑N ↓N`, and the item spacing around them.
    pub(super) trailing: f32,
    /// The branch switcher at its full width.
    pub(super) branch: f32,
}

/// True when the branch side fits beside the tabs with the branch name at
/// its full width or truncated to no less than [`MIN_BRANCH_WIDTH`].
pub(super) fn branch_beside_tabs(budget: BranchBudget) -> bool {
    budget.free >= budget.trailing + budget.branch.min(MIN_BRANCH_WIDTH)
}

/// The branch switcher's full width: its text plus button padding.
fn branch_width(ui: &egui::Ui, branch_name: &str) -> f32 {
    text_width(ui, branch_name, egui::TextStyle::Button) + 2.0 * ui.spacing().button_padding.x
}

/// `↑N ↓N` with the item spacing after each label.
fn ahead_behind_width(ui: &egui::Ui, input: &GitPanelInput<'_>) -> f32 {
    let spacing = ui.spacing().item_spacing.x;
    input.compare.map_or(0.0, |compare| {
        ahead_behind_labels(input.theme, compare)
            .iter()
            .map(|(text, ..)| text_width(ui, text, egui::TextStyle::Body) + spacing)
            .sum()
    })
}

/// Everything on the branch side except the branch switcher itself.
fn trailing_width(ui: &egui::Ui, input: &GitPanelInput<'_>) -> f32 {
    let spacing = ui.spacing().item_spacing.x;
    appearance::TOOLBAR_BUTTON + 2.0 * spacing + ahead_behind_width(ui, input)
}

fn text_width(ui: &egui::Ui, text: &str, style: egui::TextStyle) -> f32 {
    egui::WidgetText::from(text)
        .into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, style)
        .size()
        .x
}

/// The ⋮ menu, the branch switcher and `↑N ↓N`, right to left. The branch
/// name truncates to the width that `↑N ↓N` leaves; its hover text keeps
/// the full name.
fn git_branch_side(
    ui: &mut egui::Ui,
    input: &GitPanelInput<'_>,
    branch_name: &str,
    outcome: &mut GitPanelOutcome,
) {
    let reserved = ahead_behind_width(ui, input);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        git_more_menu(ui, input, outcome);
        let room = egui::vec2(
            (ui.available_width() - reserved).max(0.0),
            ui.available_height(),
        );
        ui.allocate_ui(room, |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
            git_branch_button(ui, input, branch_name, outcome);
        });
        if let Some(compare) = input.compare {
            ahead_behind(ui, input.theme, compare);
        }
    });
}

fn git_branch_button(
    ui: &mut egui::Ui,
    input: &GitPanelInput<'_>,
    branch_name: &str,
    outcome: &mut GitPanelOutcome,
) {
    let tip = branch_tip(input, branch_name);
    let branch = appearance::enabled_unless(ui, input.commit_lock.head, |ui| {
        appearance::text_menu_button(ui, branch_name, |ui| {
            git_branch_items(ui, input, branch_name, &mut *outcome);
        })
        .response
        .on_hover_text(tip)
    });
    #[cfg(feature = "test-support")]
    diagnostics::record(ui.ctx(), "git-branch", branch.rect);
    let _ = branch;
}

/// `branch → base` for the branch switcher's hover text. It always names
/// the branch in full, since the switcher may truncate it.
fn branch_tip(input: &GitPanelInput<'_>, branch: &str) -> String {
    let target = input.compare.and_then(|compare| {
        input
            .base_ref
            .or(compare.base.as_deref())
            .or(compare.upstream.as_deref())
    });
    match target {
        Some(target) => format!("{branch} → {target}\nSwitch branch"),
        None => format!("{branch}\nSwitch branch"),
    }
}

/// `↓N` then `↑N` as (text, color, hover text): the parent lays out right
/// to left, so `↓` comes first.
fn ahead_behind_labels(
    theme: &AppearanceConfig,
    compare: &workspace_ops::CompareData,
) -> Vec<(String, Option<Color32>, &'static str)> {
    let mut labels = Vec::new();
    if compare.behind > 0 {
        labels.push((
            format!("↓{}", compare.behind),
            None,
            "Commits behind the base ref",
        ));
    }
    if compare.ahead > 0 {
        labels.push((
            format!("↑{}", compare.ahead),
            Some(appearance::color(&theme.git_added)),
            "Commits ahead of the base ref",
        ));
    }
    labels
}

/// `↑N ↓N` commits ahead of and behind the base ref, beside the branch.
fn ahead_behind(ui: &mut egui::Ui, theme: &AppearanceConfig, compare: &workspace_ops::CompareData) {
    for (text, color, tip) in ahead_behind_labels(theme, compare) {
        let text = match color {
            Some(color) => RichText::new(text).color(color),
            None => RichText::new(text).weak(),
        };
        ui.label(text).on_hover_text(tip);
    }
}

/// The branch switcher's menu: every branch, the current one checked.
fn git_branch_items(
    ui: &mut egui::Ui,
    input: &GitPanelInput<'_>,
    branch_name: &str,
    outcome: &mut GitPanelOutcome,
) {
    if input.branches.is_empty() {
        ui.weak("No branches");
    }
    for name in input.branches {
        let mark = if name == branch_name { "✓" } else { "" };
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: name,
            icon: "GitBranch",
            shortcut: mark,
        })
        .clicked()
        {
            outcome.switch = Some(name.clone());
            ui.close();
        }
    }
    super::branch_menu::branch_actions(ui, input, branch_name, outcome);
}

/// One toolbar icon, recorded for fixtures under `target`.
fn action(ui: &mut egui::Ui, icon: &str, tip: &str, target: &str) -> egui::Response {
    let response = appearance::sidebar_action(ui, icon, tip);
    #[cfg(feature = "test-support")]
    diagnostics::record(ui.ctx(), target, response.rect);
    let _ = target;
    response
}

pub(super) struct GitActionsInput<'a> {
    pub(super) ui: &'a mut egui::Ui,
    pub(super) input: &'a GitPanelInput<'a>,
    pub(super) prepared: &'a PreparedGit,
    pub(super) outcome: &'a mut GitPanelOutcome,
}

/// Refresh, Stage all, Shelve, Show diff | View as list, Expand all,
/// Collapse all. A commit running in this repository disables Stage all
/// (index) and Shelve (stash).
pub(super) fn git_actions(row: GitActionsInput<'_>) {
    let GitActionsInput {
        ui,
        input,
        prepared,
        outcome,
    } = row;
    let has = |group| {
        prepared
            .groups
            .iter()
            .any(|prepared| prepared.group == group)
    };
    let staged = has(terminator_git::GitGroup::Staged);
    let unstaged =
        has(terminator_git::GitGroup::Changes) || has(terminator_git::GitGroup::Untracked);
    let lock = input.commit_lock;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.spacing_mut().interact_size.y = appearance::TOOLBAR_BUTTON;
        if action(ui, "RefreshCw", "Refresh", "git-refresh").clicked() {
            outcome.refresh = true;
        }
        let stage_all = appearance::enabled_unless(ui, lock.index, |ui| {
            ui.add_enabled_ui(!input.context.changes.is_empty(), |ui| {
                action(ui, "Plus", "Stage all", "git-stage-all")
            })
            .inner
        });
        if stage_all.clicked() {
            outcome.stage_all = true;
        }
        let shelve = appearance::enabled_unless(ui, lock.stash, |ui| {
            ui.add_enabled_ui(staged, |ui| {
                action(
                    ui,
                    "Archive",
                    "Shelve staged changes (git stash)",
                    "git-shelve",
                )
            })
            .inner
        });
        if shelve.clicked() {
            outcome.shelve = true;
        }
        let diff = ui
            .add_enabled_ui(staged || unstaged, |ui| {
                action(ui, "Eye", "Show all changes", "git-show-changes")
            })
            .inner;
        if diff.clicked() {
            outcome.show_changes = Some(!unstaged);
        }
        ui.separator();
        view_actions(ui, input, outcome);
    });
}

/// View as list, Expand all, and Collapse all.
fn view_actions(ui: &mut egui::Ui, input: &GitPanelInput<'_>, outcome: &mut GitPanelOutcome) {
    let list = appearance::selectable_icon(appearance::SelectableIconInput {
        ui,
        icon: "List",
        tip: "View as list",
        selected: input.view_list,
    });
    #[cfg(feature = "test-support")]
    diagnostics::record(ui.ctx(), "git-view-list", list.rect);
    if list.clicked() {
        outcome.toggle_list = true;
    }
    if action(ui, "ChevronsUpDown", "Expand all", "git-expand").clicked() {
        outcome.expand_all = Some(true);
    }
    if action(ui, "ChevronsDownUp", "Collapse all", "git-collapse").clicked() {
        outcome.expand_all = Some(false);
    }
}
