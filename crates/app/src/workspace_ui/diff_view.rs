use super::super::*;
use super::{
    diff_controls,
    diff_paint::{
        DiffColors, DiffMetrics, SplitPane, diff_content_width, diff_doc_digits, diff_row_count,
        paint_diff_document, paint_diff_pane, paint_markdown_diff_preview, split_rects,
    },
};
use diff::actions::{Kind, Request};

/// Previous/next siblings of `current` in an ordered changed-file list.
/// Returns `(prev, next, position, total)`; position is 1-based, or 0 when
/// the current file is not in the list.
pub(super) fn diff_neighbors(
    paths: &[std::path::PathBuf],
    current: &std::path::Path,
) -> (
    Option<std::path::PathBuf>,
    Option<std::path::PathBuf>,
    usize,
    usize,
) {
    let total = paths.len();
    let Some(index) = paths
        .iter()
        .position(|path| path.components().eq(current.components()))
    else {
        return (None, None, 0, total);
    };
    (
        index.checked_sub(1).and_then(|i| paths.get(i).cloned()),
        paths.get(index.saturating_add(1)).cloned(),
        index.saturating_add(1),
        total,
    )
}

impl App {
    /// Replace the current diff pane in place with a sibling file's diff,
    /// keeping the same layout slot (RustRover-style file navigation).
    pub(super) fn navigate_diff(&mut self, current: &Tab, target: std::path::PathBuf) {
        if self.block_unsaved_diff(Some(&current.key())) {
            return;
        }
        let Tab::Diff { cwd, staged, .. } = current else {
            return;
        };
        let next = Tab::Diff {
            cwd: cwd.clone(),
            path: target,
            staged: *staged,
        };
        self.diff_replacement = Some((current.clone(), next));
    }

    /// Swap this staged (read-only) pane for the editable working-tree diff
    /// of the same file, in the same layout slot.
    pub(super) fn open_working_diff(&mut self, current: &Tab) {
        if self.block_unsaved_diff(Some(&current.key())) {
            return;
        }
        let Tab::Diff { cwd, path, .. } = current else {
            return;
        };
        let next = Tab::Diff {
            cwd: cwd.clone(),
            path: path.clone(),
            staged: false,
        };
        self.diff_replacement = Some((current.clone(), next));
    }

    /// Apply navigation after the checked-out main or floating dock returns.
    pub(crate) fn apply_diff_replacement(&mut self) {
        let Some((current, next)) = self.diff_replacement.clone() else {
            return;
        };
        let mut replaced = false;
        if let Some(project) = self.selected.clone()
            && let Some(workspace) = self.layouts.get_mut(&project)
        {
            for group in &mut workspace.tabs {
                if group.primary.as_ref() == Some(&current) {
                    group.primary = Some(next.clone());
                    replaced = true;
                }
                for (_, pane) in group.layout.iter_all_tabs_mut() {
                    if pane == &current {
                        *pane = next.clone();
                        replaced = true;
                    }
                }
            }
        }
        for window in &mut self.floating {
            if let Some(dock) = &mut window.dock {
                for (_, pane) in dock.iter_all_tabs_mut() {
                    if pane == &current {
                        *pane = next.clone();
                        replaced = true;
                    }
                }
            }
        }
        if !replaced {
            return;
        }
        self.diff_replacement = None;
        self.focus_tab = Some(next.clone());
        self.loading.insert(next.key());
        if self.state.settings.diff_split_default {
            self.diff_split.insert(next.key());
        } else {
            self.diff_split.remove(&next.key());
        }
        let _ = self.jobs.send(Job::Diff(next));
    }

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
        // Focus must belong to a real widget. A synthetic focus ID has no
        // AccessKit node and crashes its macOS consumer when window focus changes.
        let focused = ui.ui_contains_pointer();
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
        let mut navigate = None;
        let mut open_working = false;
        let mut open_line = None;
        let mut context = None;
        let mut refresh = false;
        let mut save_edits = false;
        // Sibling files on the same side (staged or working tree), in Git's
        // reported order, for prev/next-file navigation.
        let siblings: Vec<std::path::PathBuf> = self
            .context
            .as_ref()
            .map(|context| {
                context
                    .changes
                    .iter()
                    .filter(|change| {
                        if *staged {
                            change.in_group(terminator_git::GitGroup::Staged)
                        } else {
                            change.in_group(terminator_git::GitGroup::Changes)
                                || change.in_group(terminator_git::GitGroup::Untracked)
                        }
                    })
                    .map(|change| change.path.clone())
                    .collect()
            })
            .unwrap_or_default();
        let current_abs = if path.is_absolute() {
            path.clone()
        } else {
            cwd.join(path)
        };
        let (prev_file, next_file, file_pos, file_total) = diff_neighbors(&siblings, &current_abs);
        let is_md = crate::markdown::supported(path);
        let mut gap = None;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if *staged {
                let open = appearance::sidebar_action(ui, "Pencil", "Open working tree (editable)");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "diff-open-working", open.rect);
                open_working = open.clicked();
            }
            let buffer_dirty = !*staged && review.unsaved_edits();
            if buffer_dirty {
                let save = ui
                    .add_enabled_ui(!review.busy, |ui| {
                        appearance::sidebar_action(ui, "Save", "Save edits to the working tree")
                    })
                    .inner;
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "diff-edit-save", save.rect);
                save_edits = save.clicked();
                let discard = ui
                    .add_enabled_ui(!review.busy, |ui| {
                        appearance::sidebar_action(ui, "X", "Discard edits")
                    })
                    .inner;
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "diff-edit-cancel", discard.rect);
                if discard.clicked() {
                    review.buffer = None;
                }
            }
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
            refresh = ui
                .add_enabled_ui(!review.busy, |ui| {
                    appearance::sidebar_action(ui, "RefreshCw", "Refresh diff")
                })
                .inner
                .clicked();
            ui.separator();
            let prev_file_btn = ui
                .add_enabled_ui(prev_file.is_some(), |ui| {
                    appearance::sidebar_action(ui, "ArrowLeft", "Previous file")
                })
                .inner;
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "diff-prev-file", prev_file_btn.rect);
            if prev_file_btn.clicked() {
                navigate.clone_from(&prev_file);
            }
            if file_total > 0 {
                ui.weak(format!("{file_pos}/{file_total} files"));
            }
            let next_file_btn = ui
                .add_enabled_ui(next_file.is_some(), |ui| {
                    appearance::sidebar_action(ui, "ArrowRight", "Next file")
                })
                .inner;
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "diff-next-file", next_file_btn.rect);
            if next_file_btn.clicked() {
                navigate.clone_from(&next_file);
            }
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
        // Keep a live buffer of the working-tree side. It is re-seeded from
        // a refreshed document only while it holds no unsaved edits.
        if !*staged
            && let Some(document) = &doc
            && review.buffer.as_ref().is_none_or(|buffer| {
                !std::sync::Arc::ptr_eq(&buffer.document, document)
                    && buffer.text == buffer.document.right_text
            })
        {
            review.buffer = Some(diff::review::Edit {
                document: std::sync::Arc::clone(document),
                text: document.right_text.clone(),
                right_spans: None,
                live_tints: None,
                live_rows: None,
                undo: Vec::new(),
                redo: Vec::new(),
                burst: 0.0,
                left_spans: None,
            });
        }
        if let Some(doc) = &doc {
            let split = self.diff_split.contains(&key);
            super::diff_paint::comparison_headers(
                ui,
                doc,
                split,
                self.diff_split_ratio,
                &cwd.join(path),
                !*staged,
                *staged,
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
                    // Working tree: split right pane and unified rows are
                    // live editors beside/between the painted lines.
                    let editable = split && !*staged && review.buffer.is_some();
                    let outputs = if !split && !*staged && review.buffer.is_some() {
                        match &mut review.buffer {
                            Some(buffer) => vec![super::diff_edit::paint_unified_editor(
                                ui,
                                doc,
                                buffer,
                                &cwd.join(path),
                                colors,
                                &key,
                            )],
                            None => paint_diff_document(
                                ui,
                                doc,
                                split,
                                colors,
                                &key,
                                &mut self.diff_split_ratio,
                                &mut sync,
                            ),
                        }
                    } else if editable {
                        let metrics = DiffMetrics::measure(ui, diff_doc_digits(doc, true));
                        let content = diff_content_width(ui, doc, true, &metrics, &key);
                        let rows = diff_row_count(doc, true);
                        let (left_rect, gap_rect, right_rect) =
                            split_rects(body, self.diff_split_ratio);
                        let frame_sync = sync;
                        let left = paint_diff_pane(
                            ui,
                            left_rect,
                            doc,
                            SplitPane::Left,
                            colors,
                            &metrics,
                            content,
                            rows,
                            &key,
                            frame_sync,
                        );
                        let right = match &mut review.buffer {
                            Some(buffer) => super::diff_edit::paint_right_editor(
                                ui,
                                right_rect,
                                buffer,
                                &cwd.join(path),
                                colors,
                                frame_sync,
                                &key,
                            ),
                            None => paint_diff_pane(
                                ui,
                                right_rect,
                                doc,
                                SplitPane::Right,
                                colors,
                                &metrics,
                                content,
                                rows,
                                &key,
                                frame_sync,
                            ),
                        };
                        let handle = egui::Rect::from_center_size(
                            gap_rect.center(),
                            egui::vec2(6.0, gap_rect.height()),
                        );
                        let response = ui.interact(
                            handle,
                            ui.scope_id().with(("diff-split-drag", &key)),
                            egui::Sense::drag(),
                        );
                        if response.hovered() || response.dragged() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                        }
                        if response.dragged()
                            && let Some(pos) = response.interact_pointer_pos()
                        {
                            let usable =
                                (body.width() - super::diff_paint::DIFF_SPLIT_GAP).max(1.0);
                            self.diff_split_ratio =
                                ((pos.x - body.left()) / usable).clamp(0.05, 0.95);
                        }
                        ui.painter().vline(
                            gap_rect.center().x,
                            gap_rect.y_range(),
                            egui::Stroke::new(
                                1.0,
                                ui.visuals().widgets.noninteractive.bg_stroke.color,
                            ),
                        );
                        let pointer = ui.ctx().input(|input| input.pointer.hover_pos());
                        let over_left = pointer.is_some_and(|pos| left_rect.contains(pos));
                        let over_right = pointer.is_some_and(|pos| right_rect.contains(pos));
                        sync = match (over_left, over_right) {
                            (true, false) => left.state.unclamped_offset().y,
                            (false, true) => right.state.unclamped_offset().y,
                            _ => {
                                if (left.state.unclamped_offset().y - frame_sync).abs()
                                    >= (right.state.unclamped_offset().y - frame_sync).abs()
                                {
                                    left.state.unclamped_offset().y
                                } else {
                                    right.state.unclamped_offset().y
                                }
                            }
                        };
                        vec![left, right]
                    } else {
                        paint_diff_document(
                            ui,
                            doc,
                            split,
                            colors,
                            &key,
                            &mut self.diff_split_ratio,
                            &mut sync,
                        )
                    };
                    let interaction = diff_controls::paint(
                        ui,
                        doc,
                        split,
                        *staged,
                        editable || (!split && !*staged && review.buffer.is_some()),
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
            }
        } else if let Some(Err(error)) = self.diffs.get(&key) {
            ui.colored_label(ui.visuals().error_fg_color, error);
        } else {
            ui.spinner();
        }

        let request = if save_edits {
            review.buffer.as_ref().map(|edit| Request::Save {
                document: std::sync::Arc::clone(&edit.document),
                text: edit.text.clone(),
            })
        } else if let Some(index) = confirmed {
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
        if (enabled || save_edits)
            && let Some(request) = request
        {
            review.saving_edits = save_edits;
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
        if let Some(target) = navigate {
            self.navigate_diff(tab, target);
        }
        if open_working {
            self.open_working_diff(tab);
        }
    }
}
