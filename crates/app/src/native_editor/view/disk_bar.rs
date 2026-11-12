//! In-file bars about the file on disk: save errors, a change on disk
//! (Reload / Compare / Keep mine), load failures, and large-file mode.
use super::*;
use crate::native_editor::disk::{Stamp, run_on_files};
use crate::native_editor::extras::{CompareLine, CompareView, compare_lines};
use crate::native_editor::file_io::{LineEnding, NativeFileIo, SaveError};
use terminator_core::async_service::Policy;

/// Most comparison lines painted under the conflict bar.
const MAX_COMPARE_LINES: usize = 2000;

impl ViewPass<'_> {
    pub(super) fn save_error_row(&mut self, ui: &mut egui::Ui) {
        if let Some(notice) = self.doc.current_notice() {
            ui.horizontal(|ui| {
                ui.colored_label(self.frame.failed_color, notice);
            });
        }
        let Some(error) = self.doc.save_error.clone() else {
            return;
        };
        ui.horizontal(|ui| {
            ui.colored_label(self.frame.failed_color, format!("Save failed: {error}"));
            if error == SaveError::ChangedOnDisk && ui.small_button("Save anyway").clicked() {
                self.save(true);
                ui.memory_mut(|memory| memory.request_focus(self.file_focus()));
            }
        });
    }

    /// A clean buffer follows the disk without asking (as an undoable
    /// edit, so undo history survives). A dirty one shows the conflict bar.
    pub(super) fn follow_disk(&mut self, ui: &mut egui::Ui) {
        let doc = &mut *self.doc;
        let on_disk = doc.disk_mtime_cached(&self.frame.services);
        if !doc.disk_conflict(on_disk) {
            doc.extras.compare = None;
        } else if !doc.dirty() {
            if doc.disk.reload_seen != Some(Stamp(on_disk)) {
                doc.disk.reload_seen = Some(Stamp(on_disk));
                doc.request_reload(false);
            }
        } else {
            self.conflict_bar(ui, on_disk);
        }
        if let Some(missing) = self.doc.disk.missing.clone() {
            ui.weak(format!("Not on disk any more: {missing}"));
        }
    }

    fn conflict_bar(&mut self, ui: &mut egui::Ui, on_disk: Option<std::time::SystemTime>) {
        let text = ui.visuals().text_color();
        let fill = ui
            .visuals()
            .extreme_bg_color
            .lerp_to_gamma(self.frame.failed_color, 0.2);
        let choice = egui::Frame::new()
            .fill(fill)
            .inner_margin(egui::Margin::symmetric(10, 5))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new("The file changed on disk and here.").color(text));
                    if ui
                        .button("Reload")
                        .on_hover_text("Discard my changes")
                        .clicked()
                    {
                        Some(0)
                    } else if ui.button("Compare").clicked() {
                        Some(1)
                    } else if ui
                        .button("Keep mine")
                        .on_hover_text("Saving overwrites the disk")
                        .clicked()
                    {
                        Some(2)
                    } else {
                        None
                    }
                })
                .inner
            })
            .inner;
        match choice {
            Some(0) => {
                self.doc.extras.compare = None;
                self.doc.request_reload(true);
            }
            Some(1) => self.start_compare(),
            Some(2) => {
                self.doc.extras.compare = None;
                // A save overwrites this disk version on purpose; a later
                // change on disk is refused again.
                self.doc.disk.keep_mine = Some(Stamp(on_disk));
            }
            _ => {}
        }
        self.compare_panel(ui);
    }

    /// Read the disk text on the file pool for the comparison.
    fn start_compare(&mut self) {
        let path = self.doc.path.clone();
        let limit = self.doc.read_limit;
        let rx = run_on_files(
            &self.frame.services,
            ("native-compare", &path.clone(), Policy::ReplaceableRead),
            move || {
                let text = NativeFileIo::read(&path, limit)?.text;
                Ok(LineEnding::decode(text).0)
            },
        );
        let error = rx
            .is_none()
            .then(|| "Could not start reading the file; try again.".to_owned());
        self.doc.extras.compare = Some(CompareView {
            rx,
            lines: Vec::new(),
            error,
        });
    }

    /// Disk (−) against this buffer (+), under the conflict bar.
    fn compare_panel(&mut self, ui: &mut egui::Ui) {
        let doc = &mut *self.doc;
        let Some(view) = doc.extras.compare.as_mut() else {
            return;
        };
        if let Some(rx) = view.rx.as_mut() {
            match rx.try_recv() {
                Ok(Ok(disk)) => {
                    let mine = doc.doc.as_ref().map(Doc::text).unwrap_or_default();
                    view.lines = compare_lines(&disk, &mine);
                    view.rx = None;
                }
                Ok(Err(error)) => {
                    view.error = Some(format!("{error:#}"));
                    view.rx = None;
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => ui.ctx().request_repaint(),
                // The read ended without a result (timed out, panicked).
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    view.error = Some("The file read stopped before it finished.".into());
                    view.rx = None;
                }
            }
        }
        let mut close = false;
        ui.horizontal(|ui| {
            ui.weak("Disk (−) and this buffer (+)");
            close = ui.small_button("Close").clicked();
        });
        if let Some(error) = &view.error {
            ui.colored_label(self.frame.failed_color, error);
        } else if view.rx.is_some() {
            ui.weak("Reading the file…");
        }
        let (added, removed) = (self.frame.change_colors[0], self.frame.failed_color);
        egui::ScrollArea::vertical()
            .id_salt(("native-compare", &self.frame.source_id))
            .max_height(220.0)
            .show(ui, |ui| {
                for line in view.lines.iter().take(MAX_COMPARE_LINES) {
                    let (prefix, text, color) = match line {
                        CompareLine::Same(text) => (" ", text.as_str(), None),
                        CompareLine::Mine(text) => ("+", text.as_str(), Some(added)),
                        CompareLine::Disk(text) => ("−", text.as_str(), Some(removed)),
                        CompareLine::Skipped => ("⋯", "", None),
                    };
                    let mut rich = egui::RichText::new(format!("{prefix} {text}")).monospace();
                    if let Some(color) = color {
                        rich = rich.color(color);
                    }
                    ui.label(rich);
                }
            });
        if close {
            self.doc.extras.compare = None;
        }
        ui.separator();
    }

    pub(super) fn load_failed(&self, ui: &mut egui::Ui) -> bool {
        let Some(error) = self.doc.load_error.clone() else {
            return false;
        };
        let retrying = self.doc.disk.retry.is_some();
        ui.colored_label(
            self.frame.failed_color,
            if retrying {
                format!("Could not open: {error} (trying again)")
            } else {
                format!("Could not open: {error}")
            },
        );
        if retrying {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
        }
        true
    }

    /// Large-file mode: say what is off and offer to turn it on anyway.
    pub(super) fn large_file_bar(&mut self, ui: &mut egui::Ui) {
        if !self.doc.large_file() {
            return;
        }
        ui.horizontal(|ui| {
            ui.weak("Large file: highlighting, folding and problem underlines are off.");
            if ui.small_button("Enable anyway").clicked() {
                self.doc.extras.large_override = true;
            }
        });
    }

    /// Status-bar cells: line breaks, encoding, indentation. Rebuilt only
    /// when one of them changes.
    pub(super) fn status_items(&mut self) -> Arc<Vec<String>> {
        let key = (
            self.doc.line_ending,
            self.doc.encoding,
            self.doc.format_options(),
        );
        if let Some((cached, items)) = &self.doc.extras.status
            && *cached == key
        {
            return Arc::clone(items);
        }
        let (ending, encoding, indent) = key;
        let indent = if indent.insert_spaces {
            format!("{} spaces", indent.tab_size)
        } else {
            "Tabs".to_owned()
        };
        let items = Arc::new(vec![ending.label().to_owned(), encoding.label(), indent]);
        self.doc.extras.status = Some((key, Arc::clone(&items)));
        items
    }
}
