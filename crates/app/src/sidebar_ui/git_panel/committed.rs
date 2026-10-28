//! Files committed on the branch since the base ref.
use super::*;

/// Files changed since the base ref, grouped under "COMMITTED ON BRANCH".
pub(super) struct GitCommittedSectionInput<'a, 'b> {
    pub(super) ui: &'a mut egui::Ui,
    pub(super) ctx: &'a GitRowCtx<'b>,
    pub(super) outcome: &'a mut GitPanelOutcome,
    pub(super) root: Option<&'a Path>,
    pub(super) files: &'a [workspace_ops::CommittedFile],
    pub(super) collapse_generation: u64,
}

pub(super) fn git_committed_section(input: GitCommittedSectionInput<'_, '_>) {
    let GitCommittedSectionInput {
        ui,
        ctx,
        outcome,
        root,
        files,
        collapse_generation,
    } = input;
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
                git_committed_row(GitCommittedRowInput {
                    ui,
                    ctx,
                    outcome,
                    file,
                });
            }
        });
}

pub(super) struct GitCommittedRowInput<'a> {
    pub(super) ui: &'a mut egui::Ui,
    pub(super) ctx: &'a GitRowCtx<'a>,
    pub(super) outcome: &'a mut GitPanelOutcome,
    pub(super) file: &'a workspace_ops::CommittedFile,
}

/// One file changed since the base ref. It has no index/worktree side, so the
/// only useful actions are open and copy.
pub(super) fn git_committed_row(input: GitCommittedRowInput<'_>) {
    let GitCommittedRowInput {
        ui,
        ctx,
        outcome,
        file,
    } = input;
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
    let response = appearance::file_row(appearance::FileRowInput {
        ui,
        label: &name,
        icon: icons::file_icon(&file.path),
        selected: false,
        height: 24.0,
        trailing: &trailing,
        tint: git_color(ctx.theme, file.letter),
    })
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
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Open file",
            icon: "FileCode",
            shortcut: ctx.open_shortcut,
        })
        .clicked()
        {
            outcome.menu.push(GitFileAction {
                action: FileAction::Open,
                path: file.path.clone(),
            });
            ui.close();
        }
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Copy path",
            icon: "Copy",
            shortcut: "",
        })
        .clicked()
        {
            outcome.copy.push(file.path.display().to_string());
            ui.close();
        }
    });
}
