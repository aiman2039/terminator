//! The virtualized Explorer file tree and its drag-and-drop wiring.
use super::*;

impl App {
    #[cfg(test)]
    pub(crate) fn tree(&mut self, ui: &mut egui::Ui, path: &std::path::Path, _depth: usize) {
        self.tree_with_host(ui, path, ui.scope_id());
    }

    pub(super) fn tree_with_host(&mut self, ui: &mut egui::Ui, path: &Path, host: egui::Id) {
        ui.spacing_mut().interact_size.y = ROW_HEIGHT;
        ui.spacing_mut().item_spacing.y = 0.0;
        let rows = self.explorer_rows(path);
        // The tree is a keyboard target of its own: register it every frame
        // (egui forgets unpainted focus targets), then let it read keys.
        let focus = tree_focus_id(host);
        ui.interact(
            ui.available_rect_before_wrap(),
            focus,
            egui::Sense::focusable_noninteractive(),
        );
        self.explorer_keys(ExplorerKeysInput {
            ui,
            rows: &rows,
            host,
            root: path,
        });
        let context = RowContext {
            rows: &rows,
            focus,
            focused_file: self.focused_local_file(),
            open_shortcut: self.shortcut_label(Action::OpenFile),
            split_shortcut: self.shortcut_label(Action::SplitRight),
        };
        let mut drop_dir: Option<PathBuf> = None;
        // An open rename / new-item prompt paints as a tree row: in place
        // of the renamed row, or inserted under its folder.
        let slot = self.name_prompt_slot(&rows, path);
        let insert = match slot {
            Some(PromptSlot::Insert { index, depth }) => Some((index, depth)),
            _ => None,
        };
        let replace = match slot {
            Some(PromptSlot::Replace(index)) => Some(index),
            _ => None,
        };
        let count = rows.len().saturating_add(usize::from(insert.is_some()));
        // Painted position of row `index`, and the row at a painted position
        // (`None` for the inserted prompt).
        let shown_at = |index: usize| match insert {
            Some((at, _)) if index >= at => index.saturating_add(1),
            _ => index,
        };
        let row_at = |shown: usize| match insert {
            Some((at, _)) if shown == at => None,
            Some((at, _)) if shown > at => Some(shown.saturating_sub(1)),
            _ => Some(shown),
        };
        let prompt_shown = insert.map(|(at, _)| at).or(replace.map(shown_at));
        let scroll_to = self.explorer_scroll_to.take();
        let reveal = self
            .explorer_reveal
            .as_ref()
            .filter(|reveal| reveal.host == host && reveal.root == path && reveal.pending_scroll)
            .map(|reveal| reveal.file.clone());
        // Fixed-height rows (every listing loaded) paint only the visible
        // slice; status rows ("Loading…", errors) vary in height, so a tree
        // with any of them paints every row as before.
        if rows.iter().all(|row| row.entry.is_some()) {
            let top = ui.next_widget_position().y;
            let clip = ui.clip_rect();
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "Row indices from UI coordinates."
            )]
            let (first, last) = (
                ((clip.top() - top) / ROW_HEIGHT).floor().max(0.0) as usize,
                ((clip.bottom() - top) / ROW_HEIGHT).ceil().max(0.0) as usize,
            );
            let last = last.saturating_add(1).min(count);
            let first = first.min(last);
            // A prompt that just opened scrolls into view so its field can
            // take focus (unpainted rows cannot).
            let prompt_target = prompt_shown.filter(|_| self.name_prompt_focus);
            let targets = [scroll_to.as_ref(), reveal.as_ref()]
                .into_iter()
                .flatten()
                .map(|target| {
                    (
                        rows.iter()
                            .position(|row| &row.path == target)
                            .map(shown_at),
                        Some(target) == reveal.as_ref(),
                    )
                })
                .chain(std::iter::once((prompt_target, false)));
            for (shown, center) in targets {
                if let Some(index) = shown {
                    let y = top + index as f32 * ROW_HEIGHT;
                    ui.scroll_to_rect(
                        egui::Rect::from_min_size(
                            egui::pos2(clip.left(), y),
                            egui::vec2(1.0, ROW_HEIGHT),
                        ),
                        center.then_some(egui::Align::Center),
                    );
                }
            }
            if reveal.is_some()
                && let Some(active) = self.explorer_reveal.as_mut()
            {
                active.pending_scroll = false;
            }
            ui.allocate_space(egui::vec2(ui.available_width(), first as f32 * ROW_HEIGHT));
            for shown in first..last {
                match row_at(shown) {
                    None => self.explorer_prompt_row(ExplorerPromptRowInput {
                        ui,
                        depth: insert.map_or(0, |(_, depth)| depth),
                        tree_focus: focus,
                    }),
                    Some(index) => {
                        let Some(row) = rows.get(index) else {
                            continue;
                        };
                        if replace == Some(index) {
                            self.explorer_prompt_row(ExplorerPromptRowInput {
                                ui,
                                depth: row.depth,
                                tree_focus: focus,
                            });
                        } else {
                            self.explorer_tree_row(ExplorerTreeRowInput {
                                ui,
                                row,
                                context: &context,
                                drop_dir: &mut drop_dir,
                            });
                        }
                    }
                }
            }
            ui.allocate_space(egui::vec2(
                ui.available_width(),
                count.saturating_sub(last) as f32 * ROW_HEIGHT,
            ));
        } else {
            for shown in 0..count {
                let Some(index) = row_at(shown) else {
                    self.explorer_prompt_row(ExplorerPromptRowInput {
                        ui,
                        depth: insert.map_or(0, |(_, depth)| depth),
                        tree_focus: focus,
                    });
                    continue;
                };
                let Some(row) = rows.get(index) else {
                    continue;
                };
                if replace == Some(index) {
                    self.explorer_prompt_row(ExplorerPromptRowInput {
                        ui,
                        depth: row.depth,
                        tree_focus: focus,
                    });
                    continue;
                }
                let target =
                    scroll_to.as_ref() == Some(&row.path) || reveal.as_ref() == Some(&row.path);
                if target {
                    let rect = ui.available_rect_before_wrap();
                    ui.scroll_to_rect(
                        egui::Rect::from_min_size(rect.min, egui::vec2(1.0, ROW_HEIGHT)),
                        Some(egui::Align::Center),
                    );
                    if reveal.as_ref() == Some(&row.path)
                        && let Some(active) = self.explorer_reveal.as_mut()
                    {
                        active.pending_scroll = false;
                    }
                }
                self.explorer_tree_row(ExplorerTreeRowInput {
                    ui,
                    row,
                    context: &context,
                    drop_dir: &mut drop_dir,
                });
            }
        }
        self.explorer_external_drop(ui, path, drop_dir);
        if let Some(text) = self.explorer_speed_text() {
            let clip = ui.clip_rect();
            let galley = ui.painter().layout_no_wrap(
                format!("Search: {text}"),
                egui::FontId::proportional(12.0),
                ui.visuals().strong_text_color(),
            );
            let size = galley.size();
            let rect = egui::Rect::from_min_size(
                egui::pos2(clip.right() - size.x - 12.0, clip.top() + 2.0),
                egui::vec2(size.x + 8.0, size.y + 4.0),
            );
            ui.painter().rect_filled(
                rect,
                appearance::WIDGET_RADIUS,
                ui.visuals().extreme_bg_color,
            );
            ui.painter().galley(
                egui::pos2(rect.min.x + 4.0, rect.min.y + 2.0),
                galley,
                ui.visuals().text_color(),
            );
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(1600));
        }
        if let Some(count) =
            egui::DragAndDrop::payload::<ExplorerDrag>(ui.ctx()).map(|payload| payload.0.len())
            && let Some(pointer) = ui.ctx().pointer_latest_pos()
        {
            egui::Tooltip::always_open(
                ui.ctx().clone(),
                ui.layer_id(),
                focus.with("drag-label"),
                egui::PopupAnchor::Position(egui::pos2(pointer.x + 12.0, pointer.y + 12.0)),
            )
            .show(|ui| {
                ui.label(if count == 1 {
                    "Move 1 item".to_owned()
                } else {
                    format!("Move {count} items")
                })
            });
        }
    }

    /// The open name prompt as a tree row at `depth`.
    pub(super) fn explorer_prompt_row(&mut self, input: ExplorerPromptRowInput<'_>) {
        let ExplorerPromptRowInput {
            ui,
            depth,
            tree_focus,
        } = input;
        // Exactly one row tall, so virtualized row math stays exact.
        let (mut rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), ROW_HEIGHT),
            egui::Sense::hover(),
        );
        let indent = depth as f32 * ui.spacing().indent;
        rect.min.x += indent;
        let mut row = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center))
                .id_salt("explorer-prompt-row"),
        );
        row.set_clip_rect(rect.intersect(ui.clip_rect()));
        self.name_prompt_row(&mut row, tree_focus);
    }

    pub(super) fn explorer_tree_row(&mut self, input: ExplorerTreeRowInput<'_, '_>) {
        let ExplorerTreeRowInput {
            ui,
            row,
            context,
            drop_dir,
        } = input;
        let mut rect = ui.available_rect_before_wrap();
        // Explorer indent is a UI coordinate; depth can exceed the f32 mantissa.
        let indent = row.depth as f32 * ui.spacing().indent;
        rect.min.x += indent;
        ui.push_id(&row.path, |ui| {
            ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                if let Some(entry) = &row.entry {
                    if skip_clipped_row(ui, ROW_HEIGHT) {
                        return;
                    }
                    self.explorer_entry(ExplorerEntryInput {
                        ui,
                        entry,
                        row,
                        context,
                        drop_dir,
                    });
                } else {
                    self.explorer_directory_status(ui, &row.path);
                }
            });
        });
    }

    /// Files dropped from the OS file manager onto the tree copy into the
    /// folder under the pointer (or the tree root).
    pub(super) fn explorer_external_drop(
        &mut self,
        ui: &egui::Ui,
        root: &Path,
        drop_dir: Option<PathBuf>,
    ) {
        let clip = ui.clip_rect();
        let pointer_inside = ui
            .ctx()
            .pointer_latest_pos()
            .is_none_or(|pointer| clip.contains(pointer));
        if !pointer_inside {
            return;
        }
        if ui.ctx().input(|input| !input.raw.hovered_files.is_empty()) {
            ui.painter().rect_stroke(
                clip,
                4.0,
                egui::Stroke::new(2.0, ui.visuals().selection.stroke.color),
                egui::StrokeKind::Inside,
            );
            ui.ctx().request_repaint();
        }
        let dropped: Vec<PathBuf> = ui.ctx().input_mut(|input| {
            std::mem::take(&mut input.raw.dropped_files)
                .into_iter()
                .map(|file| file.path().to_path_buf())
                .filter(|path| !path.as_os_str().is_empty())
                .collect()
        });
        if !dropped.is_empty() {
            self.queue_workspace(workspace_ops::Op::Copy {
                from: dropped,
                to_dir: drop_dir.unwrap_or_else(|| root.to_owned()),
            });
        }
    }

    /// Drag source and drop target wiring shared by file and folder rows.
    /// Dropping on a folder moves into it; on a file, into its folder.
    pub(super) fn explorer_row_dnd(&mut self, input: ExplorerRowDndInput<'_>) {
        let ExplorerRowDndInput {
            ui,
            response,
            path,
            directory,
            drop_dir,
        } = input;
        let target = if directory {
            Some(path.to_owned())
        } else {
            path.parent().map(Path::to_path_buf)
        };
        if response.contains_pointer() {
            drop_dir.clone_from(&target);
        }
        if response.dragged() {
            let paths = if self.explorer_selection.iter().any(|p| p == path) {
                self.explorer_selection.clone()
            } else {
                vec![path.to_owned()]
            };
            response.dnd_set_drag_payload(ExplorerDrag(paths));
        }
        if directory && response.dnd_hover_payload::<ExplorerDrag>().is_some() {
            ui.painter().rect_stroke(
                response.rect,
                appearance::WIDGET_RADIUS,
                egui::Stroke::new(1.5, ui.visuals().selection.stroke.color),
                egui::StrokeKind::Inside,
            );
        }
        if let Some(payload) = response.dnd_release_payload::<ExplorerDrag>()
            && let Some(target) = target
        {
            self.explorer_drop(payload.0.clone(), target);
        }
    }

    pub(super) fn explorer_directory_status(&mut self, ui: &mut egui::Ui, path: &Path) {
        if let Some(error) = self.directory_errors.get(path).cloned() {
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!(
                    "Cannot refresh {} ({:?}): {}",
                    error.path.display(),
                    error.kind,
                    error.message
                ),
            );
            if self.dirs.contains_key(path) {
                ui.weak("Showing the last successful listing.");
            }
            ui.horizontal(|ui| {
                let retry = ui.button("Retry");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "directory-retry", retry.rect);
                if retry.clicked() {
                    self.refresh_request = None;
                }
                if ui.button("Choose folder again").clicked() {
                    self.add_project = true;
                }
            });
            if cfg!(target_os = "macos") {
                ui.weak("Check System Settings → Privacy & Security → Files and Folders. Full Disk Access is optional troubleshooting; this error may have another cause. Shells and editors can have separate access.");
            }
        }
        if !self.dirs.contains_key(path) && !self.directory_errors.contains_key(path) {
            ui.weak("Loading…");
        }
    }
}
