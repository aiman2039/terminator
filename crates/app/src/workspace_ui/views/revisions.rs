use super::super::super::*;

impl App {
    /// Start loading `tab`'s Git view if needed; shows a spinner and returns
    /// false until the result arrives.
    pub(super) fn git_view_ready(&mut self, ui: &mut egui::Ui, key: &str, tab: &Tab) -> bool {
        if !self.git_views.contains_key(key) {
            self.git_views
                .insert(key.to_owned(), crate::GitView::Loading);
            let _ = self.jobs.send(Job::GitView(tab.clone()));
        }
        if matches!(self.git_views.get(key), Some(crate::GitView::Loading)) {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak("Loading…");
            });
            return false;
        }
        true
    }
    /// Read-only diff of one file between two revisions.
    pub(crate) fn revision_diff_view(&mut self, ui: &mut egui::Ui, tab: &Tab) {
        let Tab::RevisionDiff { cwd, path, .. } = tab else {
            return;
        };
        let key = tab.key();
        if self.revision_toolbar(ui, tab, &key) {
            self.git_views.remove(&key);
        }
        if !self.git_view_ready(ui, &key, tab) {
            return;
        }
        self.revision_diff_body(ui, &key, &cwd.join(path));
    }

    /// Title plus reload and split/unified toggles. Returns whether Reload was clicked.
    fn revision_toolbar(&mut self, ui: &mut egui::Ui, tab: &Tab, key: &str) -> bool {
        let Tab::RevisionDiff { cwd, path, .. } = tab else {
            return false;
        };
        let mut refresh = false;
        let split = self.diff_split.contains(key);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.add(egui::Label::new(egui::RichText::new(tab.git_title()).strong()).truncate())
                .on_hover_text(cwd.join(path).display().to_string());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                refresh = appearance::sidebar_action(ui, "RefreshCw", "Reload").clicked();
                if appearance::selectable_icon(appearance::SelectableIconInput {
                    ui,
                    icon: "Columns2",
                    tip: "Side by side",
                    selected: split,
                })
                .clicked()
                {
                    self.diff_split.insert(key.to_owned());
                }
                if appearance::selectable_icon(appearance::SelectableIconInput {
                    ui,
                    icon: "Rows2",
                    tip: "Unified",
                    selected: !split,
                })
                .clicked()
                {
                    self.diff_split.remove(key);
                }
            });
        });
        refresh
    }

    fn revision_diff_body(&mut self, ui: &mut egui::Ui, key: &str, full_path: &std::path::Path) {
        let Some(crate::GitView::Revision(result)) = self.git_views.get(key) else {
            return;
        };
        let doc = match result {
            Ok(doc) => std::sync::Arc::clone(doc),
            Err(error) => {
                ui.colored_label(appearance::color(&self.theme.status_failed), error);
                return;
            }
        };
        let split = self.diff_split.contains(key);
        super::super::diff_paint::comparison_headers(
            super::super::diff_paint::ComparisonHeadersInput {
                ui,
                doc: &doc,
                split,
                ratio: self.diff_split_ratio,
                path: full_path,
                editable: false,
                staged: false,
            },
        );
        if doc.hunks.is_empty() {
            ui.weak("No differences");
            return;
        }
        let colors = super::super::diff_paint::DiffColors::from_theme(&self.theme);
        let mut sync = self.diff_split_scroll.get(key).copied().unwrap_or(0.0);
        super::super::diff_paint::paint_diff_document(
            super::super::diff_paint::PaintDiffDocumentInput {
                ui,
                doc: &doc,
                split,
                colors,
                scroll_key: key,
                ratio: &mut self.diff_split_ratio,
                sync: &mut sync,
            },
        );
        self.diff_split_scroll.insert(key.to_owned(), sync);
    }

    /// Commits that touched a file; each opens its change, or a comparison
    /// of that revision with the working tree.
    pub(crate) fn file_history_view(&mut self, ui: &mut egui::Ui, tab: &Tab) {
        let Tab::FileHistory { cwd, path } = tab else {
            return;
        };
        let key = tab.key();
        ui.horizontal(|ui| {
            ui.strong(tab.git_title());
            ui.weak(cwd.join(path).display().to_string());
        });
        ui.separator();
        if !self.git_view_ready(ui, &key, tab) {
            return;
        }
        let Some(crate::GitView::History(result)) = self.git_views.get(&key) else {
            return;
        };
        let entries = match result {
            Ok(entries) => entries.clone(),
            Err(error) => {
                ui.colored_label(appearance::color(&self.theme.status_failed), error);
                return;
            }
        };
        if entries.is_empty() {
            ui.weak("No commits touch this file.");
            return;
        }
        let mut open: Option<Tab> = None;
        appearance::sidebar_scroll("file-history").show(ui, |ui| {
            for entry in &entries {
                ui.horizontal(|ui| {
                    let row = ui
                        .add(egui::Button::selectable(false, &entry.subject).truncate())
                        .on_hover_text(format!(
                            "{} · {} · {}\nClick to show this commit's change",
                            entry.short_hash, entry.author, entry.date
                        ));
                    if row.clicked() {
                        open = Some(Tab::RevisionDiff {
                            cwd: cwd.clone(),
                            path: path.clone(),
                            left: format!("{}^", entry.hash),
                            right: entry.hash.clone(),
                        });
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if appearance::sidebar_action(
                            ui,
                            "FileDiff",
                            "Compare with the working tree",
                        )
                        .clicked()
                        {
                            open = Some(Tab::RevisionDiff {
                                cwd: cwd.clone(),
                                path: path.clone(),
                                left: entry.hash.clone(),
                                right: String::new(),
                            });
                        }
                        ui.weak(&entry.date);
                        ui.weak(&entry.author);
                        ui.monospace(&entry.short_hash);
                    });
                });
            }
        });
        if let Some(tab) = open {
            self.open_git_tab(tab);
        }
    }

    pub(crate) fn blame_view(&mut self, ui: &mut egui::Ui, tab: &Tab) {
        let Tab::Blame { path, .. } = tab else {
            return;
        };
        ui.weak(format!("Blame: {}", path.display()));
        ui.separator();
        let key = tab.key();
        if !self.git_view_ready(ui, &key, tab) {
            return;
        }
        let Some(crate::GitView::Blame(result)) = self.git_views.get(&key) else {
            return;
        };
        match result {
            Ok(data) => {
                appearance::sidebar_scroll("blame").show(ui, |ui| {
                    for entry in &data.entries {
                        ui.horizontal(|ui| {
                            let shown = entry.hash.len().min(7);
                            ui.weak(entry.hash.get(..shown).unwrap_or(entry.hash.as_str()));
                            ui.weak(&entry.author);
                            ui.weak(&entry.date);
                            ui.weak("| ");
                            ui.monospace(&entry.code);
                        });
                    }
                });
            }
            Err(e) => {
                ui.colored_label(appearance::color(&self.theme.status_failed), e);
            }
        }
    }
}
