//! Local History page body: timeline on the left, native diff on the right.
use super::super::super::*;
use super::super::diff_paint;
use terminator_core::local_history::{Entry, Source};

const TIMELINE_WIDTH: f32 = 280.0;

/// Short age such as `5m ago` or `3d ago`.
fn age(timestamp_ms: u64, now_ms: u64) -> String {
    let secs = now_ms.saturating_sub(timestamp_ms) / 1000;
    match secs {
        0..90 => format!("{secs}s ago"),
        90..5400 => format!("{}m ago", secs / 60),
        5400..172_800 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}

fn describe(entry: &Entry) -> String {
    let source = match entry.source {
        Source::Save => "Saved",
        Source::External => "Changed on disk",
        Source::Open => "Baseline",
        Source::Label => "Label",
    };
    entry.label.clone().unwrap_or_else(|| source.to_owned())
}

impl App {
    pub(crate) fn local_history_center(&mut self, ui: &mut egui::Ui) {
        let Some(path) = self.local_history.as_ref().map(|page| page.path.clone()) else {
            self.hide_center_overlay();
            return;
        };
        ui.set_min_size(ui.available_size());
        ui.horizontal(|ui| {
            ui.strong("Local History");
            ui.add(
                egui::Label::new(egui::RichText::new(path.display().to_string()).weak()).truncate(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if appearance::sidebar_action(ui, "X", "Close").clicked() {
                    self.hide_center_overlay();
                }
                if appearance::sidebar_action(ui, "RefreshCw", "Reload").clicked() {
                    let _ = self
                        .jobs
                        .send(Job::LocalHistory(local_history::Op::List(path.clone())));
                }
            });
        });
        ui.separator();
        if self.local_history.is_none() {
            return;
        }
        let height = ui.available_height();
        ui.horizontal_top(|ui| {
            ui.allocate_ui(egui::vec2(TIMELINE_WIDTH, height), |ui| {
                ui.set_width(TIMELINE_WIDTH);
                self.local_history_timeline(ui);
            });
            ui.separator();
            ui.vertical(|ui| self.local_history_diff(ui, &path));
        });
    }

    fn local_history_timeline(&mut self, ui: &mut egui::Ui) {
        let Some(page) = self.local_history.as_ref() else {
            return;
        };
        let entries = match &page.entries {
            None => {
                ui.spinner();
                return;
            }
            Some(Err(error)) => {
                ui.colored_label(appearance::color(&self.theme.status_failed), error);
                return;
            }
            Some(Ok(entries)) if entries.is_empty() => {
                ui.weak("No snapshots yet. Changes are recorded while the file is in a project.");
                return;
            }
            Some(Ok(entries)) => entries.clone(),
        };
        let selected = page.selected.clone();
        let now = terminator_core::local_history::now_ms();
        let mut pick = None;
        appearance::sidebar_scroll("local-history").show(ui, |ui| {
            for entry in &entries {
                if entry.source == Source::Label {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(format!("◆ {}", describe(entry))).strong());
                        ui.weak(age(entry.timestamp_ms, now));
                    });
                    continue;
                }
                let chosen = selected.as_deref() == Some(entry.id.as_str());
                let text = format!(
                    "{}  ·  {}  ·  {} B",
                    age(entry.timestamp_ms, now),
                    describe(entry),
                    entry.size
                );
                if ui
                    .add(egui::Button::selectable(chosen, text).truncate())
                    .clicked()
                {
                    pick = Some(entry.id.clone());
                }
            }
        });
        if let Some(id) = pick {
            self.select_local_history(id);
        }
    }

    fn local_history_diff(&mut self, ui: &mut egui::Ui, path: &std::path::Path) {
        let Some(page) = self.local_history.as_ref() else {
            return;
        };
        if page.selected.is_none() {
            ui.weak("Select a snapshot to compare it with the file on disk.");
            return;
        }
        let doc = match &page.doc {
            None => {
                ui.spinner();
                return;
            }
            Some(Err(error)) => {
                ui.colored_label(appearance::color(&self.theme.status_failed), error);
                return;
            }
            Some(Ok(doc)) => std::sync::Arc::clone(doc),
        };
        self.local_history_actions(ui);
        let key = format!("local-history:{}", path.display());
        let split = self.diff_split.contains(&key);
        diff_paint::comparison_headers(diff_paint::ComparisonHeadersInput {
            ui,
            doc: &doc,
            split,
            ratio: self.diff_split_ratio,
            path,
            editable: false,
            staged: false,
        });
        if doc.hunks.is_empty() {
            ui.weak("No differences: the file matches this snapshot.");
            return;
        }
        let colors = diff_paint::DiffColors::from_theme(&self.theme);
        let mut sync = self.diff_split_scroll.get(&key).copied().unwrap_or(0.0);
        diff_paint::paint_diff_document(diff_paint::PaintDiffDocumentInput {
            ui,
            doc: &doc,
            split,
            colors,
            scroll_key: &key,
            ratio: &mut self.diff_split_ratio,
            sync: &mut sync,
        });
        self.diff_split_scroll.insert(key, sync);
    }

    /// Split toggle and Revert, which asks for confirmation first.
    fn local_history_actions(&mut self, ui: &mut egui::Ui) {
        let Some(page) = self.local_history.as_mut() else {
            return;
        };
        let key = format!("local-history:{}", page.path.display());
        let mut revert = false;
        ui.horizontal(|ui| {
            if page.confirm_revert {
                ui.label("Replace the file on disk with this snapshot? The current content is kept in Local History.");
                if ui.button("Revert").clicked() {
                    revert = true;
                }
                if ui.button("Cancel").clicked() {
                    page.confirm_revert = false;
                }
            } else if ui
                .add_enabled(!page.busy, egui::Button::new("Revert to this snapshot…"))
                .clicked()
            {
                page.confirm_revert = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let split = self.diff_split.contains(&key);
                if appearance::selectable_icon(appearance::SelectableIconInput {
                    ui,
                    icon: "Columns2",
                    tip: "Side by side",
                    selected: split,
                })
                .clicked()
                {
                    self.diff_split.insert(key.clone());
                }
                if appearance::selectable_icon(appearance::SelectableIconInput {
                    ui,
                    icon: "Rows2",
                    tip: "Unified",
                    selected: !split,
                })
                .clicked()
                {
                    self.diff_split.remove(&key);
                }
            });
        });
        if revert {
            self.revert_local_history();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_read_in_the_largest_whole_unit() {
        assert_eq!(age(0, 5_000), "5s ago");
        assert_eq!(age(0, 600_000), "10m ago");
        assert_eq!(age(0, 3 * 3_600_000), "3h ago");
        assert_eq!(age(0, 3 * 86_400_000), "3d ago");
    }
}
