//! Full snapshots use egui's text editor for independent selections, including
//! selections that extend beyond the viewport. Git snapshots stay immutable.
use super::super::*;

pub(super) fn show(ui: &mut egui::Ui, edit: &mut diff::review::Edit, read_only: bool, key: &str) {
    ui.columns(2, |columns| {
        for (side, ui) in columns.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.strong(if side == 0 {
                    &edit.document.left_label
                } else {
                    &edit.document.right_label
                });
                ui.weak(if side == 0 || read_only {
                    "Read-only"
                } else {
                    "Editable · Save to write to disk"
                });
            });
            egui::ScrollArea::both()
                .id_salt((key, "diff-edit-scroll", side))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // A &str TextBuffer permits selection and copy but rejects edits.
                    let output = if side == 1 && !read_only {
                        text_view(ui, &mut edit.text, key, side)
                    } else {
                        let mut snapshot = if side == 0 {
                            edit.document.left_text.as_str()
                        } else {
                            edit.text.as_str()
                        };
                        text_view(ui, &mut snapshot, key, side)
                    };
                    #[cfg(feature = "test-support")]
                    diagnostics::record(
                        ui.ctx(),
                        if side == 0 {
                            "diff-edit-old"
                        } else {
                            "diff-edit-local"
                        },
                        output.response.rect,
                    );
                    let _ = output;
                });
        }
    });
}

fn text_view(
    ui: &mut egui::Ui,
    text: &mut dyn egui::TextBuffer,
    key: &str,
    side: usize,
) -> egui::text_edit::TextEditOutput {
    egui::TextEdit::multiline(text)
        .id_salt((key, "diff-edit-text", side))
        .font(egui::TextStyle::Monospace)
        .code_editor()
        .desired_width(f32::INFINITY)
        .show(ui)
}

impl App {
    pub(crate) fn block_unsaved_diff(&mut self, key: Option<&str>) -> bool {
        let dirty = self.diff_reviews.iter().any(|(id, state)| {
            key.is_none_or(|key| key == id)
                && (state.unsaved_edits() || state.saving_edits && state.busy)
        });
        if dirty {
            self.error = Some("Save or discard edits in the diff viewer before closing.".into());
        }
        dirty
    }
}
