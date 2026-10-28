//! Git panel toolbar, base-ref picker, and branch compare line.
use super::*;

/// Inline width of the trailing compare/view actions (list toggle, base-ref
/// picker, compare refresh) when they sit beside the log button: three icon
/// slots plus their gaps.
pub(super) const GIT_MORE_INLINE_WIDTH: f32 = 3.0 * appearance::TOOLBAR_BUTTON + 2.0 * 4.0;

/// Base-ref picker shared by the inline toolbar button and the overflow menu:
/// automatic upstream plus every known branch.
pub(super) fn git_base_ref_contents(
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

/// Trailing compare/view actions as inline toolbar icons, used when the
/// sidebar has room. Falls back to [`git_overflow_more`] when narrow.
pub(super) fn git_inline_more(
    ui: &mut egui::Ui,
    input: &GitPanelInput<'_>,
    outcome: &mut GitPanelOutcome,
) {
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
pub(super) fn git_overflow_more(
    ui: &mut egui::Ui,
    input: &GitPanelInput<'_>,
    outcome: &mut GitPanelOutcome,
) {
    let more = appearance::compact_menu_button(ui, "…", |ui| {
        let mark = if input.view_list { "✓" } else { "" };
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "View as list",
            icon: "List",
            shortcut: mark,
        })
        .clicked()
        {
            outcome.toggle_list = true;
            ui.close();
        }
        let base = appearance::menu_button(ui, "Change Base Ref…", |ui| {
            git_base_ref_contents(ui, input, outcome);
        });
        let _ = base;
        ui.separator();
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Refresh branch compare",
            icon: "RefreshCw",
            shortcut: "",
        })
        .clicked()
        {
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
pub(super) fn git_toolbar(
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
        let changes = appearance::selectable_icon(appearance::SelectableIconInput {
            ui,
            icon: "FileDiff",
            tip: "Changes",
            selected: !input.history,
        });
        let history = appearance::selectable_icon(appearance::SelectableIconInput {
            ui,
            icon: "History",
            tip: "History",
            selected: input.history,
        });
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
pub(super) struct GitCompareRowInput<'a> {
    pub(super) ui: &'a mut egui::Ui,
    pub(super) theme: &'a AppearanceConfig,
    pub(super) branch: &'a str,
    pub(super) compare: &'a workspace_ops::CompareData,
    pub(super) base_ref: Option<&'a str>,
}

pub(super) fn git_compare_row(input: GitCompareRowInput<'_>) {
    let GitCompareRowInput {
        ui,
        theme,
        branch,
        compare,
        base_ref,
    } = input;
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
