//! Changed-file rows and their tri-state checkboxes.
use super::*;

/// Inclusion state of a file, folder, or group checkbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckState {
    Unchecked,
    Checked,
    Partial,
}

/// Aggregate checkbox state: all, none, or some children included.
pub struct CheckStateInput {
    pub checked: usize,
    pub total: usize,
}

pub fn check_state(input: CheckStateInput) -> CheckState {
    let CheckStateInput { checked, total } = input;
    if total == 0 || checked == 0 {
        CheckState::Unchecked
    } else if checked >= total {
        CheckState::Checked
    } else {
        CheckState::Partial
    }
}

/// Small tri-state checkbox painted to fit the 20px Git rows. Returns true
/// when clicked; the caller decides whether that means include or exclude.
pub(super) fn tri_checkbox(ui: &mut egui::Ui, state: CheckState) -> bool {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(14.0, GIT_TREE_ROW_HEIGHT), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let box_rect = egui::Rect::from_center_size(rect.center(), egui::vec2(13.0, 13.0));
        let visuals = ui.visuals();
        let (fill, stroke) = match state {
            CheckState::Unchecked => (
                visuals.widgets.inactive.bg_fill,
                visuals.widgets.inactive.bg_stroke,
            ),
            _ => (visuals.selection.bg_fill, egui::Stroke::NONE),
        };
        ui.painter()
            .rect(box_rect, 3, fill, stroke, egui::StrokeKind::Inside);
        let mark = visuals.selection.stroke.color;
        match state {
            CheckState::Checked => {
                let c = box_rect.center();
                ui.painter().line_segment(
                    [egui::pos2(c.x - 3.5, c.y), egui::pos2(c.x - 1.0, c.y + 2.5)],
                    egui::Stroke::new(1.6, mark),
                );
                ui.painter().line_segment(
                    [
                        egui::pos2(c.x - 1.0, c.y + 2.5),
                        egui::pos2(c.x + 3.5, c.y - 2.5),
                    ],
                    egui::Stroke::new(1.6, mark),
                );
            }
            CheckState::Partial => {
                let c = box_rect.center();
                ui.painter().line_segment(
                    [egui::pos2(c.x - 3.0, c.y), egui::pos2(c.x + 3.0, c.y)],
                    egui::Stroke::new(1.8, mark),
                );
            }
            CheckState::Unchecked => {}
        }
    }
    response.clicked()
}

/// One changed file: diffstat, status letter, click-to-diff, and context menu.
pub(super) struct GitChangeRowInput<'a> {
    pub(super) ui: &'a mut egui::Ui,
    pub(super) ctx: &'a GitRowCtx<'a>,
    pub(super) outcome: &'a mut GitPanelOutcome,
    pub(super) change: &'a terminator_git::Change,
    pub(super) group: terminator_git::GitGroup,
    pub(super) label: &'a str,
}

pub(super) fn git_change_row(input: GitChangeRowInput<'_>) {
    let GitChangeRowInput {
        ui,
        ctx,
        outcome,
        change,
        group,
        label,
    } = input;
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
    // Checked means staged (included in the next commit). Conflicts must be
    // resolved first, so they get no checkbox.
    let mut check_clicked = false;
    let response = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            if group != terminator_git::GitGroup::Conflicts {
                let checked = usize::from(group == terminator_git::GitGroup::Staged);
                check_clicked =
                    tri_checkbox(ui, check_state(CheckStateInput { checked, total: 1 }));
            }
            appearance::file_row(appearance::FileRowInput {
                ui,
                label,
                icon: icons::file_icon(&change.path),
                selected: false,
                height: GIT_TREE_ROW_HEIGHT,
                trailing: &trailing,
                tint: git_color(ctx.theme, letter),
            })
        })
        .inner
        .on_hover_text(format!(
            "{}\n{} ({})",
            change.path.display(),
            terminator_git::status_description(letter),
            group.label()
        ));
    if check_clicked {
        if group == terminator_git::GitGroup::Staged {
            outcome.unstage.push(change.path.clone());
        } else {
            outcome.stage.push(change.path.clone());
        }
    }
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
    if let Some(action) = git_click_action(GitClickActionInput {
        deleted: letter == 'D',
        staged,
        clicked: response.clicked() || response.double_clicked(),
        review_mode: ctx.review_mode,
        neovim_review: ctx.neovim_review,
    }) {
        outcome.clicked.push(GitFileAction {
            action,
            path: change.path.clone(),
        });
    }
    appearance::context_menu(&response, |ui| {
        let open_changes = git_click_action(GitClickActionInput {
            deleted: letter == 'D',
            staged,
            clicked: true,
            review_mode: ctx.review_mode,
            neovim_review: ctx.neovim_review,
        });
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
        if group == terminator_git::GitGroup::Conflicts
            && appearance::menu_item(ui, "Resolve conflicts…", "GitCompareArrows", "").clicked()
        {
            outcome.resolve = Some(change.path.clone());
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

pub(super) const GIT_SECTION_LIMIT: usize = 8;

/// Per-level indent and row height for the changed-file tree. Kept tight so a
/// deep path stays legible in the narrow sidebar.
pub(super) const GIT_TREE_INDENT: f32 = 12.0;

pub(in crate::sidebar_ui) const GIT_TREE_ROW_HEIGHT: f32 = 20.0;
