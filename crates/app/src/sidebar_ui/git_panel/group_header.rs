//! A change group's header row (`JetBrains` `Changes  2 files`).
use super::*;

/// Group checkbox, the bold name with a dim `N files`, and the
/// stage/unstage-all action, which `lock` disables. The name's menu shows
/// all of the group's changes. Returns whether the checkbox was clicked.
pub(super) fn git_group_header(
    ui: &mut egui::Ui,
    prepared: &PreparedGroup,
    lock: Option<&str>,
    outcome: &mut GitPanelOutcome,
) -> bool {
    let group = prepared.group;
    let total = prepared.entries.len();
    let label = git_group_label(group);
    ui.spacing_mut().item_spacing.x = 2.0;
    let mut check_clicked = false;
    if group != terminator_git::GitGroup::Conflicts {
        let checked = if group == terminator_git::GitGroup::Staged {
            total
        } else {
            0
        };
        let state = check_state(CheckStateInput { checked, total });
        check_clicked = tri_checkbox(ui, state, lock);
    }
    let name = ui.add(
        egui::Label::new(RichText::new(label).strong())
            .selectable(false)
            .sense(egui::Sense::click()),
    );
    ui.add_space(4.0);
    let hint = prepared.tree.files_hint();
    ui.add(egui::Label::new(RichText::new(hint).weak()).selectable(false));
    if group != terminator_git::GitGroup::Conflicts {
        appearance::context_menu(&name, |ui| {
            if appearance::menu_action(ui, true, "Show all changes", "FileDiff").clicked() {
                outcome.show_changes = Some(group == terminator_git::GitGroup::Staged);
            }
        });
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        git_group_bulk_action(ui, prepared, lock, outcome);
    });
    check_clicked
}

fn git_group_bulk_action(
    ui: &mut egui::Ui,
    prepared: &PreparedGroup,
    lock: Option<&str>,
    outcome: &mut GitPanelOutcome,
) {
    let (icon, tip) = match prepared.group {
        terminator_git::GitGroup::Staged => (UNSTAGE_ICON, "Unstage all"),
        terminator_git::GitGroup::Changes | terminator_git::GitGroup::Untracked => {
            (STAGE_ICON, "Stage all")
        }
        terminator_git::GitGroup::Conflicts => return,
    };
    let action =
        appearance::enabled_unless(ui, lock, |ui| appearance::sidebar_action(ui, icon, tip));
    #[cfg(feature = "test-support")]
    diagnostics::record(ui.ctx(), &format!("git-group-{tip}"), action.rect);
    if action.clicked() {
        let paths = prepared.entries.iter().map(|change| change.path.clone());
        if prepared.group == terminator_git::GitGroup::Staged {
            outcome.unstage.extend(paths);
        } else {
            outcome.stage.extend(paths);
        }
    }
}
