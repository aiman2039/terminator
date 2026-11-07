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

/// The ⋮ menu: commit log, base ref, and branch compare refresh.
fn git_more_menu(ui: &mut egui::Ui, input: &GitPanelInput<'_>, outcome: &mut GitPanelOutcome) {
    let more = appearance::icon_menu_button(ui, "EllipsisVertical", |ui| {
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

/// Tabs on the left; the ⋮ menu and the branch switcher on the right. A
/// commit or push running in this worktree disables the branch switch.
pub(super) fn git_header(header: GitHeaderInput<'_>) {
    let GitHeaderInput {
        ui,
        input,
        branch_name,
        outcome,
    } = header;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let current = input.tab();
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
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            git_more_menu(ui, input, outcome);
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
            if let Some(compare) = input.compare {
                ahead_behind(ui, input.theme, compare);
            }
        });
    });
}

/// `branch → base` for the branch switcher's hover text.
fn branch_tip(input: &GitPanelInput<'_>, branch: &str) -> String {
    let target = input.compare.and_then(|compare| {
        input
            .base_ref
            .or(compare.base.as_deref())
            .or(compare.upstream.as_deref())
    });
    match target {
        Some(target) => format!("{branch} → {target}\nSwitch branch"),
        None => "Switch branch".to_owned(),
    }
}

/// `↑N ↓N` commits ahead of and behind the base ref, beside the branch.
/// The parent lays out right to left, so `↓` comes first.
fn ahead_behind(ui: &mut egui::Ui, theme: &AppearanceConfig, compare: &workspace_ops::CompareData) {
    if compare.behind > 0 {
        ui.label(RichText::new(format!("↓{}", compare.behind)).weak())
            .on_hover_text("Commits behind the base ref");
    }
    if compare.ahead > 0 {
        ui.label(
            RichText::new(format!("↑{}", compare.ahead)).color(appearance::color(&theme.git_added)),
        )
        .on_hover_text("Commits ahead of the base ref");
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
        outcome.expand = true;
    }
    if action(ui, "ChevronsDownUp", "Collapse all", "git-collapse").clicked() {
        outcome.collapse = true;
    }
}
