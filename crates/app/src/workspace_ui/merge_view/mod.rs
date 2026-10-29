//! Three-way merge tab (`IntelliJ` Merge Revisions): Ours | Result |
//! Theirs, aligned chunk by chunk. Gutter chevrons accept a side into the
//! editable center; F7 / Shift+F7 walk the unresolved changes. Save writes
//! the file; Mark resolved also stages it, and only on that click.
use super::super::*;
use crate::merge::{MergeDoc, MergeTab};

mod rows;
mod text;

/// What the merge tab's controls asked for this frame.
#[derive(Default)]
struct MergeClicks {
    reload: bool,
    save: bool,
    resolve: bool,
}

impl App {
    pub(in super::super) fn merge_view(&mut self, ui: &mut egui::Ui, tab: &Tab) {
        let Tab::Merge { cwd, path } = tab else {
            return;
        };
        self.poll_merge(tab);
        let key = tab.key();
        let focused = ui.ui_contains_pointer() || self.merge_tab_active(tab);
        let Some(mut state) = self.merge.remove(&key) else {
            return;
        };
        if state.busy() {
            // Poll the pending read or write until it lands.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        let mut clicks = MergeClicks::default();
        merge_toolbar(ui, tab, &mut state, &mut clicks);
        if focused && !ui.ctx().egui_wants_keyboard_input() {
            merge_keys(ui, &mut state);
        }
        merge_status(ui, &mut state, &mut clicks);
        let colors = rows::MergeColors::new(&self.theme, ui);
        if state.doc.is_some() {
            rows::merge_body(rows::MergeBodyInput {
                ui,
                key: &key,
                path,
                state: &mut state,
                colors,
            });
        } else if state.error.is_none() {
            ui.spinner();
        }
        self.merge.insert(key.clone(), state);
        if clicks.reload {
            self.reload_merge(&key, cwd, path);
        } else if clicks.save || clicks.resolve {
            self.save_merge(&key, path, clicks.resolve);
        }
    }

    fn merge_tab_active(&self, tab: &Tab) -> bool {
        self.selected
            .as_ref()
            .and_then(|project| self.layouts.get(project))
            .and_then(|workspace| workspace.active_pane())
            == Some(tab)
    }
}

/// Title, previous/next change, and reload.
fn merge_toolbar(ui: &mut egui::Ui, tab: &Tab, state: &mut MergeTab, clicks: &mut MergeClicks) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let title = ui.add(egui::Label::new(RichText::new(tab.git_title()).strong()).truncate());
        if let Tab::Merge { path, .. } = tab {
            title.on_hover_text(path.display().to_string());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if appearance::sidebar_action(ui, "RefreshCw", "Reload from Git (drops edits)")
                .clicked()
            {
                clicks.reload = true;
            }
            if appearance::sidebar_action(ui, "ArrowDown", "Next change (F7)").clicked() {
                step(state, true);
            }
            if appearance::sidebar_action(ui, "ArrowUp", "Previous change (Shift+F7)").clicked() {
                step(state, false);
            }
        });
    });
}

fn merge_keys(ui: &egui::Ui, state: &mut MergeTab) {
    let (next, previous) = ui.input_mut(|input| {
        (
            input.consume_key(egui::Modifiers::NONE, egui::Key::F7),
            input.consume_key(egui::Modifiers::SHIFT, egui::Key::F7),
        )
    });
    if next {
        step(state, true);
    } else if previous {
        step(state, false);
    }
}

/// Move to the next or previous unresolved change and scroll to it.
fn step(state: &mut MergeTab, forward: bool) {
    let Some(doc) = state.doc.as_ref() else {
        return;
    };
    if let Some(index) = doc.neighbor(state.current, forward) {
        state.current = Some(index);
        state.scroll_to = Some(index);
    }
}

/// Counters, errors, and the apply/save/resolve buttons.
fn merge_status(ui: &mut egui::Ui, state: &mut MergeTab, clicks: &mut MergeClicks) {
    if let Some(error) = &state.error {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    let busy = state.busy();
    let Some(doc) = state.doc.as_mut() else {
        return;
    };
    let (conflicts, changes) = (doc.conflicts_left(), doc.changes_left());
    ui.horizontal_wrapped(|ui| {
        ui.strong(counter(conflicts, "conflict"));
        ui.weak(counter(changes, "change"));
        ui.add_enabled_ui(!busy, |ui| {
            if ui
                .add_enabled(
                    changes > 0,
                    egui::Button::new("Apply non-conflicting changes"),
                )
                .clicked()
                && doc.apply_non_conflicting() > 0
            {
                state.dirty = true;
            }
            clicks.save = ui
                .add_enabled(state.dirty, egui::Button::new("Save"))
                .on_hover_text("Write the result; unresolved conflicts keep markers")
                .clicked();
            clicks.resolve = ui
                .add_enabled(
                    conflicts == 0 && changes == 0,
                    egui::Button::new("Mark resolved"),
                )
                .on_hover_text("Save the result and stage the file (git add)")
                .on_disabled_hover_text("Resolve every change first")
                .clicked();
        });
        if busy {
            ui.spinner();
        }
        if let Some(message) = &state.message {
            ui.weak(message);
        }
    });
}

fn counter(count: usize, noun: &str) -> String {
    format!("{count} {noun}{} left", if count == 1 { "" } else { "s" })
}

/// Column headers: which version each pane shows.
fn side_label(doc: &MergeDoc, ours: bool) -> String {
    let (name, side) = if ours {
        ("Ours", &doc.ours)
    } else {
        ("Theirs", &doc.theirs)
    };
    if side.present {
        name.to_owned()
    } else {
        format!("{name} (deleted)")
    }
}
