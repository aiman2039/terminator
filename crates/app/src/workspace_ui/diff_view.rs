use super::super::*;
use super::{
    diff_controls,
    diff_paint::{DiffColors, paint_diff_document, paint_markdown_diff_preview},
};
use diff::actions::{Kind, Request};

impl App {
    pub(super) fn diff_view(&mut self, ui: &mut egui::Ui, tab: &Tab) {
        let Tab::Diff { cwd, path, staged } = tab else {
            return;
        };
        let key = tab.key();
        let mut review = self.diff_reviews.remove(&key).unwrap_or_default();
        let doc = self
            .diffs
            .get(&key)
            .and_then(|doc| doc.as_ref().ok())
            .map(std::sync::Arc::clone);
        let enabled = !review.busy
            && !self.loading.contains(&key)
            && !self.diff_reviews.values().any(|state| state.busy)
            && doc.as_ref().is_some_and(|doc| doc.source.is_some());
        let count = doc.as_ref().map_or(0, |doc| doc.hunks.len());
        let split = self.diff_split.contains(&key);
        let focus_id = ui.id().with("diff-review-focus");
        let focused = ui.memory(|memory| memory.has_focus(focus_id)) || ui.ui_contains_pointer();
        if focused && !ui.ctx().egui_wants_keyboard_input() {
            if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::F7)) {
                review.advance(true, count);
            }
            if ui.input_mut(|input| input.consume_key(egui::Modifiers::SHIFT, egui::Key::F7)) {
                review.advance(false, count);
            }
        }
        let mut action = None;
        let mut undo = false;
        let mut open_line = None;
        let mut context = None;
        let mut refresh = false;
        let is_md = crate::markdown::supported(path);
        let mut gap = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let previous = ui
                .add_enabled_ui(count > 0 && review.selected != Some(0), |ui| {
                    appearance::sidebar_action(ui, "ArrowUp", "Previous change (Shift+F7)")
                })
                .inner;
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "diff-previous", previous.rect);
            if previous.clicked() {
                review.advance(false, count);
            }
            let next = ui
                .add_enabled_ui(count > 0 && review.selected != count.checked_sub(1), |ui| {
                    appearance::sidebar_action(ui, "ArrowDown", "Next change (F7)")
                })
                .inner;
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "diff-next", next.rect);
            if next.clicked() {
                review.advance(true, count);
            }
            ui.separator();
            let source = appearance::sidebar_action(ui, "FileText", "Open at selected change (F4)");
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "diff-open-source", source.rect);
            if source.clicked() {
                open_line = review
                    .selected
                    .and_then(|index| doc.as_ref()?.hunks.get(index))
                    .and_then(|hunk| u32::try_from(hunk.new.start.saturating_add(1)).ok())
                    .or(Some(1));
            }
            refresh = appearance::sidebar_action(ui, "RefreshCw", "Refresh diff").clicked();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let menu = appearance::icon_menu_button(ui, "Settings2", |ui| {
                    if appearance::menu_item(ui, "Show more unchanged lines", "Plus", "").clicked()
                    {
                        context = Some(review.context.max(3).saturating_add(20));
                        ui.close();
                    }
                    if appearance::menu_item(ui, "Show all unchanged lines", "FileText", "")
                        .clicked()
                    {
                        context = Some(usize::MAX / 4);
                        ui.close();
                    }
                    if appearance::menu_item(ui, "Collapse unchanged lines", "ChevronsDownUp", "")
                        .clicked()
                    {
                        context = Some(3);
                        ui.close();
                    }
                    if is_md
                        && appearance::menu_item(
                            ui,
                            if self.diff_preview.contains(&key) {
                                "Hide preview"
                            } else {
                                "Preview"
                            },
                            "FileText",
                            "",
                        )
                        .clicked()
                    {
                        if !self.diff_preview.remove(&key) {
                            self.diff_preview.insert(key.clone());
                        }
                        ui.close();
                    }
                });
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "diff-menu", menu.response.rect);
                let _ = menu;
                let side = appearance::selectable_icon(ui, "Columns2", "Side by side", split);
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "diff-side-by-side", side.rect);
                if side.clicked() {
                    self.diff_split.insert(key.clone());
                    review.jump = true;
                }
                let unified = appearance::selectable_icon(ui, "Rows2", "Unified", !split);
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "diff-unified", unified.rect);
                if unified.clicked() {
                    self.diff_split.remove(&key);
                    review.jump = true;
                }
                ui.separator();
                let undo_button = ui
                    .add_enabled_ui(enabled && !review.undo.is_empty(), |ui| {
                        appearance::sidebar_action(ui, "History", "Undo last hunk action")
                    })
                    .inner;
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "diff-undo", undo_button.rect);
                undo = undo_button.clicked();
                ui.weak(format!(
                    "Change {} of {count}",
                    review.selected.map_or(0, |index| index.saturating_add(1))
                ));
            });
        });
        if focused
            && !ui.ctx().egui_wants_keyboard_input()
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::F4))
        {
            open_line = review
                .selected
                .and_then(|index| doc.as_ref()?.hunks.get(index))
                .and_then(|hunk| u32::try_from(hunk.new.start.saturating_add(1)).ok())
                .or(Some(1));
        }
        let mut confirmed = None;
        if let Some(index) = review.confirm_discard {
            ui.horizontal_wrapped(|ui| {
                ui.label(format!(
                    "Discard change {} from disk?",
                    index.saturating_add(1)
                ));
                let discard = ui.add_enabled(enabled, egui::Button::new("Discard hunk"));
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "diff-confirm-discard", discard.rect);
                if discard.clicked() {
                    confirmed = Some(index);
                }
                if ui.button("Cancel").clicked() {
                    review.confirm_discard = None;
                }
            });
        }
        if let Some(message) = &review.message {
            ui.label(message);
        }
        if review.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Applying hunk…");
            });
        }
        if !self.diffs.contains_key(&key) && self.loading.insert(key.clone()) {
            let _ = self.jobs.send(Job::Diff(tab.clone()));
        }
        if let Some(doc) = &doc {
            let split = self.diff_split.contains(&key);
            super::diff_paint::comparison_headers(
                ui,
                doc,
                split,
                self.diff_split_ratio,
                &cwd.join(path),
            );
            if doc.hunks.is_empty() && doc.unified.is_empty() {
                ui.weak("No text changes");
            }
            if is_md && self.diff_preview.contains(&key) {
                if let Some(link) =
                    paint_markdown_diff_preview(ui, doc, &mut self.markdown, &cwd.join(path), &key)
                {
                    match link {
                        markdown::Link::File(path) => self.open_file(path, None, None, false),
                        markdown::Link::Web(url) => {
                            let _ = self.jobs.send(Job::Browser(url));
                        }
                    }
                }
            } else {
                let split = self.diff_split.contains(&key);
                let colors = DiffColors {
                    added: appearance::color(&self.theme.git_added),
                    deleted: appearance::color(&self.theme.git_deleted),
                    accent: appearance::color(&self.theme.accent),
                    text: appearance::color(&self.theme.text),
                };
                let mut sync = self.diff_split_scroll.get(&key).copied().unwrap_or(0.0);
                let full = ui.available_rect_before_wrap();
                let rail = egui::Rect::from_min_max(
                    egui::pos2((full.right() - 8.0).max(full.left()), full.top()),
                    full.max,
                );
                let gutter_w = if split { 0.0 } else { 52.0 };
                let unified_gutter =
                    egui::Rect::from_min_size(full.min, egui::vec2(gutter_w, full.height()));
                let body = egui::Rect::from_min_max(
                    egui::pos2(full.left() + gutter_w, full.top()),
                    egui::pos2(rail.left() - 2.0, full.bottom()),
                );
                ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
                    ui.set_clip_rect(full);
                    let outputs = paint_diff_document(
                        ui,
                        doc,
                        split,
                        colors,
                        &key,
                        &mut self.diff_split_ratio,
                        &mut sync,
                    );
                    let interaction = diff_controls::paint(
                        ui,
                        doc,
                        split,
                        *staged,
                        enabled,
                        &mut review,
                        &outputs,
                        rail,
                        unified_gutter,
                        colors,
                    );
                    if interaction.action.is_some() {
                        action = interaction.action;
                    }
                    if interaction.open_line.is_some() {
                        open_line = interaction.open_line;
                    }
                    gap = interaction.gap;
                    diff_controls::jump(ui, doc, split, &mut review, &outputs, &mut sync);
                });
                ui.allocate_rect(full, egui::Sense::hover());
                self.diff_split_scroll.insert(key.clone(), sync);
                if ui.rect_contains_pointer(full) && ui.input(|input| input.pointer.any_click()) {
                    ui.memory_mut(|memory| memory.request_focus(focus_id));
                }
            }
        } else if let Some(Err(error)) = self.diffs.get(&key) {
            ui.colored_label(ui.visuals().error_fg_color, error);
        } else {
            ui.spinner();
        }

        let request = if let Some(index) = confirmed {
            doc.as_ref().map(|document| Request::Hunk {
                document: std::sync::Arc::clone(document),
                hunk: index,
                kind: Kind::Discard,
            })
        } else if undo {
            review.undo.last().cloned().map(Request::Undo)
        } else if let Some((index, kind)) = action {
            review.selected = Some(index);
            if kind == Kind::Discard {
                review.confirm_discard = Some(index);
                None
            } else {
                doc.as_ref().map(|document| Request::Hunk {
                    document: std::sync::Arc::clone(document),
                    hunk: index,
                    kind,
                })
            }
        } else {
            None
        };
        if enabled && let Some(request) = request {
            review.busy = true;
            review.confirm_discard = None;
            review.message = None;
            let _ = self
                .jobs
                .send(Job::DiffAction(tab.clone(), Box::new(request)));
        }
        if !review.busy
            && let Some(gap) = gap
            && let Some(doc) = &doc
        {
            review.confirm_discard = None;
            self.loading.insert(key.clone());
            let _ = self.jobs.send(Job::DiffExpand(
                tab.clone(),
                std::sync::Arc::clone(doc),
                gap,
            ));
        }
        if !review.busy && (refresh || context.is_some()) {
            let context = context.unwrap_or(review.context.max(3));
            review.jump = true;
            review.confirm_discard = None;
            self.loading.insert(key.clone());
            let _ = self.jobs.send(Job::DiffContext(tab.clone(), context));
        }
        self.diff_reviews.insert(key, review);
        if let Some(line) = open_line {
            self.open_file_mode(cwd.join(path), Some(line), None, false, true);
        }
    }
}
