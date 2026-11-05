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
        self.diff_change_buttons(ui, frame, pass);
        Self::diff_stage_file_button(ui, frame, pass);
        ui.separator();
        Self::diff_source_buttons(ui, frame, pass);
        ui.separator();
        self.diff_file_buttons(ui, frame, pass);
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
        diagnostics::record_response(ui.ctx(), "diff-edit-save", &save);
        pass.save_edits = save.clicked();
        let discard = ui
            .add_enabled_ui(!pass.review.busy, |ui| {
                appearance::sidebar_action(ui, "X", "Discard edits")
            })
            .inner;
        #[cfg(feature = "test-support")]
        diagnostics::record_response(ui.ctx(), "diff-edit-cancel", &discard);
        if discard.clicked() {
            pass.review.buffer = None;
        }
    }

    fn diff_change_buttons(&self, ui: &mut egui::Ui, frame: &DiffFrame<'_>, pass: &mut DiffPass) {
        let count = frame.count;
        let previous = ui
            .add_enabled_ui(count > 0 && pass.review.selected != Some(0), |ui| {
                appearance::sidebar_action(
                    ui,
                    "ArrowUp",
                    &self.nav_tip(
                        "Previous change; continues into the previous file",
                        Action::DiffPrevChange,
                    ),
                )
            })
            .inner;
        #[cfg(feature = "test-support")]
        diagnostics::record_response(ui.ctx(), "diff-previous", &previous);
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
                        &self.nav_tip(
                            "Next change; continues into the next file",
                            Action::DiffNextChange,
                        ),
                    )
                },
            )
            .inner;
        #[cfg(feature = "test-support")]
        diagnostics::record_response(ui.ctx(), "diff-next", &next);
        if next.clicked() {
            pass.review.advance(true, count);
        }
    }

    /// Stage (working tree) or unstage (staged) the whole file, mode
    /// changes included; the only action for a mode-only change.
    fn diff_stage_file_button(ui: &mut egui::Ui, frame: &DiffFrame<'_>, pass: &mut DiffPass) {
        if frame.read_only || frame.remote {
            return;
        }
        let (icon, tip) = if frame.staged {
            ("Download", "Unstage whole file")
        } else {
            ("Upload", "Stage whole file")
        };
        let lock = frame.commit_lock.then_some(workspace_ops::COMMIT_WAIT);
        let button = appearance::enabled_unless(ui, lock, |ui| {
            ui.add_enabled_ui(frame.enabled && !pass.review.unsaved_edits(), |ui| {
                appearance::sidebar_action(ui, icon, tip)
            })
            .inner
        });
        #[cfg(feature = "test-support")]
        diagnostics::record_response(ui.ctx(), "diff-stage-file", &button);
        pass.stage_file = button.clicked();
    }

    /// Why Undo waits: a running commit locks undoing a stage or unstage.
    fn diff_undo_lock(frame: &DiffFrame<'_>, pass: &DiffPass) -> Option<&'static str> {
        let index = pass.review.undo.last().is_some_and(|undo| !undo.worktree());
        (frame.commit_lock && index).then_some(workspace_ops::COMMIT_WAIT)
    }

    /// The labeled undo history, newest first. Only the newest entry can
    /// be undone; older ones wait their turn.
    fn diff_undo_history(ui: &mut egui::Ui, frame: &DiffFrame<'_>, pass: &mut DiffPass) {
        if pass.review.undo.is_empty() {
            return;
        }
        let lock = Self::diff_undo_lock(frame, pass);
        let menu = appearance::icon_menu_button(ui, "List", |ui| {
            ui.weak("Undo history");
            for (at, undo) in pass.review.undo.iter().rev().enumerate() {
                let label = undo.label.as_str();
                if at == 0 {
                    let item = appearance::enabled_unless(ui, lock, |ui| {
                        ui.add_enabled(
                            frame.enabled,
                            egui::Button::new(format!("Undo {label}")).frame(false),
                        )
                    });
                    if item.clicked() {
                        pass.undo = true;
                        ui.close();
                    }
                } else {
                    ui.weak(label);
                }
            }
        });
        let _ = menu.response.on_hover_text("Undo history");
    }

    /// `text` with the action's current keybinding, if it has one.
    fn nav_tip(&self, text: &str, action: Action) -> String {
        let key = self.shortcut_label(action);
        if key.is_empty() {
            text.to_owned()
        } else {
            format!("{text} ({key})")
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

    fn diff_file_buttons(&self, ui: &mut egui::Ui, frame: &DiffFrame<'_>, pass: &mut DiffPass) {
        let prev_file_btn = ui
            .add_enabled_ui(frame.prev_file.is_some(), |ui| {
                appearance::sidebar_action(
                    ui,
                    "ArrowLeft",
                    &self.nav_tip("Previous file", Action::DiffPrevFile),
                )
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
                appearance::sidebar_action(
                    ui,
                    "ArrowRight",
                    &self.nav_tip("Next file", Action::DiffNextFile),
                )
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
        let lock = Self::diff_undo_lock(frame, pass);
        let undo_button = appearance::enabled_unless(ui, lock, |ui| {
            ui.add_enabled_ui(frame.enabled && !pass.review.undo.is_empty(), |ui| {
                appearance::sidebar_action(ui, "History", "Undo last hunk action")
            })
            .inner
        });
        #[cfg(feature = "test-support")]
        diagnostics::record_response(ui.ctx(), "diff-undo", &undo_button);
        pass.undo = undo_button.clicked();
        Self::diff_undo_history(ui, frame, pass);
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
