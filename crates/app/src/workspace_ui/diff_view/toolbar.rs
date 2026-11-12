use super::super::super::*;
use super::{DiffFrame, DiffPass};

impl App {
    /// The diff toolbar: changed files, change navigation with its
    /// counter between the arrows, file navigation likewise, the labeled
    /// whole-file stage action, edit and source on the left; undo, layout,
    /// and the view menu (with Refresh) on the right.
    pub(super) fn diff_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        frame: &DiffFrame<'_>,
        pass: &mut DiffPass,
    ) {
        ui.spacing_mut().item_spacing.x = 4.0;
        pass.compact = ui.available_width() < COMPACT_WIDTH;
        self.diff_file_tree_toggle(ui, frame);
        ui.separator();
        self.diff_change_buttons(ui, frame, pass);
        ui.separator();
        self.diff_file_buttons(ui, frame, pass);
        ui.separator();
        Self::diff_stage_file_button(ui, frame, pass);
        self.diff_edit_buttons(ui, frame, pass);
        Self::diff_source_button(ui, frame, pass);
        self.diff_note_buttons(ui, frame, pass);
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
        let position = pass
            .review
            .selected
            .map_or(0, |index| index.saturating_add(1));
        ui.weak(change_counter(position, count, pass.compact));
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
    /// changes included; the only action for a mode-only change. The one
    /// labeled control on the row, with the same glyphs as the gutter.
    fn diff_stage_file_button(ui: &mut egui::Ui, frame: &DiffFrame<'_>, pass: &mut DiffPass) {
        if frame.read_only || frame.remote {
            return;
        }
        let (icon, label, tip) = stage_file_control(frame.staged);
        let lock = frame.commit_lock.then_some(workspace_ops::COMMIT_WAIT);
        let button = appearance::enabled_unless(ui, lock, |ui| {
            ui.add_enabled_ui(frame.enabled && !pass.review.unsaved_edits(), |ui| {
                appearance::icon_label_button(appearance::IconLabelButtonInput {
                    ui,
                    icon,
                    label,
                    height: appearance::TOOLBAR_BUTTON,
                })
                .on_hover_text(tip)
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
        let menu = appearance::icon_menu_button(ui, "ChevronDown", |ui| {
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

    fn diff_source_button(ui: &mut egui::Ui, frame: &DiffFrame<'_>, pass: &mut DiffPass) {
        let source = appearance::sidebar_action(ui, "FileText", "Open at selected change (F4)");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "diff-open-source", source.rect);
        if source.clicked() {
            pass.open_line = pass.selected_change_line(frame.doc.as_ref());
        }
    }

    /// Add a review note on the selected change; when the repository has
    /// notes, Send opens Ask agent with all of them. GUI-only state.
    fn diff_note_buttons(&self, ui: &mut egui::Ui, frame: &DiffFrame<'_>, pass: &mut DiffPass) {
        let add = ui
            .add_enabled_ui(frame.count > 0 && pass.review.note.is_none(), |ui| {
                appearance::sidebar_action(
                    ui,
                    "MessageSquarePlus",
                    "Add a review note on the selected change (Alt+click lines to note \
                     only those; right-click a line to note it)",
                )
            })
            .inner;
        #[cfg(feature = "test-support")]
        diagnostics::record_response(ui.ctx(), "diff-add-note", &add);
        if add.clicked() {
            pass.review.note = pass.note_draft(frame.doc.as_ref());
        }
        let count = self.diff_notes.list(&frame.notes_repo()).len();
        if count == 0 {
            return;
        }
        let tip = format!("Send {count} review note(s) to an agent…");
        let send = appearance::sidebar_action(ui, "Send", &tip);
        #[cfg(feature = "test-support")]
        diagnostics::record_response(ui.ctx(), "diff-send-notes", &send);
        ui.weak(count.to_string());
        pass.send_notes = send.clicked();
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
            ui.weak(file_counter(frame.file_pos, frame.file_total, pass.compact));
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
        // Right to left: the history chevron paints right of Undo.
        Self::diff_undo_history(ui, frame, pass);
        let lock = Self::diff_undo_lock(frame, pass);
        let undo_tip = pass
            .review
            .undo
            .last()
            .map_or_else(|| "Undo".to_owned(), |undo| format!("Undo {}", undo.label));
        let undo_button = appearance::enabled_unless(ui, lock, |ui| {
            ui.add_enabled_ui(frame.enabled && !pass.review.undo.is_empty(), |ui| {
                appearance::sidebar_action(ui, "Undo2", &undo_tip)
            })
            .inner
        });
        #[cfg(feature = "test-support")]
        diagnostics::record_response(ui.ctx(), "diff-undo", &undo_button);
        pass.undo = undo_button.clicked();
    }

    /// Refresh, unchanged-line context, whitespace, and the markdown preview.
    fn diff_view_menu(&mut self, ui: &mut egui::Ui, frame: &DiffFrame<'_>, pass: &mut DiffPass) {
        let refresh = ui.add_enabled_ui(!pass.review.busy, |ui| {
            appearance::menu_item(appearance::MenuItemInput {
                ui,
                label: "Refresh diff",
                icon: "RefreshCw",
                shortcut: "",
            })
        });
        if refresh.inner.clicked() {
            pass.refresh = true;
            ui.close();
        }
        ui.separator();
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

/// Below this toolbar width the counters use their short form.
const COMPACT_WIDTH: f32 = 600.0;

/// The change counter between the change arrows.
pub(super) fn change_counter(position: usize, count: usize, compact: bool) -> String {
    if compact {
        format!("{position}/{count}")
    } else {
        format!("Change {position} of {count}")
    }
}

/// The file counter between the file arrows.
pub(super) fn file_counter(position: usize, total: usize, compact: bool) -> String {
    if compact {
        format!("{position}/{total}")
    } else {
        format!("{position}/{total} files")
    }
}

/// Glyph, label, and tip of the whole-file control: the gutter's Stage
/// (Plus) and Unstage (Minus) glyphs, so one glyph has one verb.
pub(super) fn stage_file_control(staged: bool) -> (&'static str, &'static str, &'static str) {
    if staged {
        ("Minus", "Unstage file", "Unstage the whole file")
    } else {
        ("Plus", "Stage file", "Stage the whole file")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counters_shorten_on_a_narrow_toolbar() {
        assert_eq!(change_counter(2, 5, false), "Change 2 of 5");
        assert_eq!(change_counter(2, 5, true), "2/5");
        assert_eq!(file_counter(1, 3, false), "1/3 files");
        assert_eq!(file_counter(1, 3, true), "1/3");
    }

    #[test]
    fn whole_file_control_uses_the_gutter_glyphs_with_a_label() {
        assert_eq!(stage_file_control(false).0, "Plus");
        assert_eq!(stage_file_control(false).1, "Stage file");
        assert_eq!(stage_file_control(true).0, "Minus");
        assert_eq!(stage_file_control(true).1, "Unstage file");
        assert!(crate::icons::is_ui_icon("Undo2"));
    }
}
