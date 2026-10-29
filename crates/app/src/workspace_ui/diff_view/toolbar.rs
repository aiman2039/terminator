use super::super::super::*;
use super::{DiffFrame, DiffPass};

impl App {
    /// The diff toolbar: edit, change, and file navigation on the left;
    /// view options, layout, undo, and the change counter on the right.
    pub(super) fn diff_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        frame: &DiffFrame<'_>,
        pass: &mut DiffPass,
    ) {
        ui.spacing_mut().item_spacing.x = 4.0;
        self.diff_file_tree_toggle(ui, frame);
        self.diff_edit_buttons(ui, frame, pass);
        Self::diff_change_buttons(ui, frame, pass);
        ui.separator();
        Self::diff_source_buttons(ui, frame, pass);
        ui.separator();
        Self::diff_file_buttons(ui, frame, pass);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            self.diff_toolbar_trailing(ui, frame, pass);
        });
    }

    /// Show or hide the changed-files panel (not for remote snapshots).
    fn diff_file_tree_toggle(&mut self, ui: &mut egui::Ui, frame: &DiffFrame<'_>) {
        if frame.remote {
            return;
        }
        let open = self.preferences.diff_file_tree;
        let toggle = appearance::selectable_icon(appearance::SelectableIconInput {
            ui,
            icon: "ListTree",
            tip: if open {
                "Hide changed files"
            } else {
                "Show changed files"
            },
            selected: open,
        });
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "diff-file-tree", toggle.rect);
        if toggle.clicked() {
            self.preferences.diff_file_tree = !open;
        }
    }

    /// Open the editable working tree from a staged diff, edit the
    /// read-only unified working tree side by side, and save or discard
    /// the working-tree buffer while it has unsaved edits.
    fn diff_edit_buttons(&mut self, ui: &mut egui::Ui, frame: &DiffFrame<'_>, pass: &mut DiffPass) {
        if frame.staged && !frame.read_only {
            let open = appearance::sidebar_action(ui, "Pencil", "Open working tree (editable)");
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "diff-open-working", open.rect);
            pass.open_working = open.clicked();
        }
        if !frame.staged && !frame.split && pass.review.buffer.is_some() {
            let edit = appearance::sidebar_action(ui, "Pencil", "Edit (side by side)");
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "diff-edit-split", edit.rect);
            if edit.clicked() {
                self.diff_split.insert(frame.key.clone());
                pass.review.jump = true;
            }
        }
        let buffer_dirty = !frame.staged && pass.review.unsaved_edits();
        if !buffer_dirty {
            return;
        }
        let save = ui
            .add_enabled_ui(!pass.review.busy && !frame.read_only, |ui| {
                appearance::sidebar_action(ui, "Save", "Save edits to the working tree")
            })
            .inner;
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "diff-edit-save", save.rect);
        pass.save_edits = save.clicked();
        let discard = ui
            .add_enabled_ui(!pass.review.busy, |ui| {
                appearance::sidebar_action(ui, "X", "Discard edits")
            })
            .inner;
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "diff-edit-cancel", discard.rect);
        if discard.clicked() {
            pass.review.buffer = None;
        }
    }

    fn diff_change_buttons(ui: &mut egui::Ui, frame: &DiffFrame<'_>, pass: &mut DiffPass) {
        let count = frame.count;
        let previous = ui
            .add_enabled_ui(count > 0 && pass.review.selected != Some(0), |ui| {
                appearance::sidebar_action(
                    ui,
                    "ArrowUp",
                    "Previous change (Shift+F7; continues into the previous file)",
                )
            })
            .inner;
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "diff-previous", previous.rect);
        if previous.clicked() {
            pass.review.advance(false, count);
        }
        let next = ui
            .add_enabled_ui(
                count > 0 && pass.review.selected != count.checked_sub(1),
                |ui| {
                    appearance::sidebar_action(
                        ui,
                        "ArrowDown",
                        "Next change (F7; continues into the next file)",
                    )
                },
            )
            .inner;
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "diff-next", next.rect);
        if next.clicked() {
            pass.review.advance(true, count);
        }
    }

    fn diff_source_buttons(ui: &mut egui::Ui, frame: &DiffFrame<'_>, pass: &mut DiffPass) {
        let source = appearance::sidebar_action(ui, "FileText", "Open at selected change (F4)");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "diff-open-source", source.rect);
        if source.clicked() {
            pass.open_line = pass.selected_change_line(frame.doc.as_ref());
        }
        pass.refresh = ui
            .add_enabled_ui(!pass.review.busy, |ui| {
                appearance::sidebar_action(ui, "RefreshCw", "Refresh diff")
            })
            .inner
            .clicked();
    }

    fn diff_file_buttons(ui: &mut egui::Ui, frame: &DiffFrame<'_>, pass: &mut DiffPass) {
        let prev_file_btn = ui
            .add_enabled_ui(frame.prev_file.is_some(), |ui| {
                appearance::sidebar_action(ui, "ArrowLeft", "Previous file")
            })
            .inner;
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "diff-prev-file", prev_file_btn.rect);
        if prev_file_btn.clicked() {
            pass.navigate.clone_from(&frame.prev_file);
        }
        if frame.file_total > 0 {
            ui.weak(format!("{}/{} files", frame.file_pos, frame.file_total));
        }
        let next_file_btn = ui
            .add_enabled_ui(frame.next_file.is_some(), |ui| {
                appearance::sidebar_action(ui, "ArrowRight", "Next file")
            })
            .inner;
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "diff-next-file", next_file_btn.rect);
        if next_file_btn.clicked() {
            pass.navigate.clone_from(&frame.next_file);
        }
    }

    fn diff_toolbar_trailing(
        &mut self,
        ui: &mut egui::Ui,
        frame: &DiffFrame<'_>,
        pass: &mut DiffPass,
    ) {
        let menu = appearance::icon_menu_button(ui, "Settings2", |ui| {
            self.diff_view_menu(ui, frame, pass);
        });
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "diff-menu", menu.response.rect);
        let _ = menu;
        self.diff_layout_toggles(ui, frame, pass);
        ui.separator();
        let undo_button = ui
            .add_enabled_ui(frame.enabled && !pass.review.undo.is_empty(), |ui| {
                appearance::sidebar_action(ui, "History", "Undo last hunk action")
            })
            .inner;
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "diff-undo", undo_button.rect);
        pass.undo = undo_button.clicked();
        ui.weak(format!(
            "Change {} of {}",
            pass.review
                .selected
                .map_or(0, |index| index.saturating_add(1)),
            frame.count
        ));
    }

    /// Unchanged-line context, whitespace, and the markdown preview.
    fn diff_view_menu(&mut self, ui: &mut egui::Ui, frame: &DiffFrame<'_>, pass: &mut DiffPass) {
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Show more unchanged lines",
            icon: "Plus",
            shortcut: "",
        })
        .clicked()
        {
            pass.context = Some(pass.review.context.max(3).saturating_add(20));
            ui.close();
        }
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Show all unchanged lines",
            icon: "FileText",
            shortcut: "",
        })
        .clicked()
        {
            pass.context = Some(usize::MAX / 4);
            ui.close();
        }
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Collapse unchanged lines",
            icon: "ChevronsDownUp",
            shortcut: "",
        })
        .clicked()
        {
            pass.context = Some(3);
            pass.collapse = true;
            ui.close();
        }
        ui.separator();
        self.diff_options_menu(ui, pass);
        let key = &frame.key;
        if frame.is_md
            && appearance::menu_item(appearance::MenuItemInput {
                ui,
                label: if self.diff_preview.contains(key) {
                    "Hide preview"
                } else {
                    "Preview"
                },
                icon: "FileText",
                shortcut: "",
            })
            .clicked()
        {
            if !self.diff_preview.remove(key) {
                self.diff_preview.insert(key.clone());
            }
            ui.close();
        }
    }

    fn diff_layout_toggles(
        &mut self,
        ui: &mut egui::Ui,
        frame: &DiffFrame<'_>,
        pass: &mut DiffPass,
    ) {
        let side = appearance::selectable_icon(appearance::SelectableIconInput {
            ui,
            icon: "Columns2",
            tip: "Side by side",
            selected: frame.split,
        });
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "diff-side-by-side", side.rect);
        if side.clicked() {
            self.diff_split.insert(frame.key.clone());
            pass.review.jump = true;
        }
        let unified = appearance::selectable_icon(appearance::SelectableIconInput {
            ui,
            icon: "Rows2",
            tip: "Unified",
            selected: !frame.split,
        });
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "diff-unified", unified.rect);
        if unified.clicked() {
            self.diff_split.remove(&frame.key);
            pass.review.jump = true;
        }
    }
}
