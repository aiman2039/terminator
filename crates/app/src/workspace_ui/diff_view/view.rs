use super::super::super::*;
use super::{DiffFrame, DiffPass, diff_neighbors};
use crate::app::OpenFileModeInput;
use std::path::{Path, PathBuf};

/// The diff tab's own fields, borrowed for building the frame.
struct DiffSource<'a> {
    tab: &'a Tab,
    cwd: &'a Path,
    path: &'a Path,
    staged: bool,
    remote: bool,
}

impl App {
    pub(in super::super) fn diff_view(&mut self, ui: &mut egui::Ui, tab: &Tab) {
        let Tab::Diff {
            cwd,
            path,
            staged,
            remote,
        } = tab
        else {
            return;
        };
        let source = DiffSource {
            tab,
            cwd,
            path,
            staged: *staged,
            remote: remote.is_some(),
        };
        let mut pass = DiffPass {
            review: self.diff_reviews.remove(&tab.key()).unwrap_or_default(),
            ..DiffPass::default()
        };
        let frame = self.diff_frame(ui, source, &pass.review);
        Self::diff_change_keys(ui, &frame, &mut pass);
        ui.horizontal(|ui| self.diff_toolbar(ui, &frame, &mut pass));
        Self::diff_open_source_key(ui, &frame, &mut pass);
        Self::diff_notice_bar(ui, &frame, &mut pass);
        if let Some(message) = pass.review.message.clone() {
            self.offer_merge(ui, frame.tab, &message);
        }
        self.request_missing_diff(&frame);
        self.sync_diff_options(&frame, &mut pass);
        Self::seed_diff_buffer(&frame, &mut pass.review);
        self.diff_main(ui, &frame, &mut pass);
        self.submit_diff_request(&frame, &mut pass);
        self.expand_diff_gap(&frame, &mut pass);
        self.refresh_diff_view(&frame, &mut pass);
        self.finish_diff_view(frame, pass);
    }

    fn diff_frame<'a>(
        &self,
        ui: &egui::Ui,
        source: DiffSource<'a>,
        review: &diff::review::State,
    ) -> DiffFrame<'a> {
        let DiffSource {
            tab,
            cwd,
            path,
            staged,
            remote,
        } = source;
        let header_path = PathBuf::from(
            self.remote_file_label(path)
                .unwrap_or_else(|| cwd.join(path).display().to_string()),
        );
        let key = tab.key();
        let doc = self
            .diffs
            .get(&key)
            .and_then(|doc| doc.as_ref().ok())
            .map(std::sync::Arc::clone);
        let read_only = remote || doc.as_ref().is_some_and(|doc| doc.read_only);
        let enabled = !read_only
            && !review.busy
            && !self.loading.contains(&key)
            && !self.diff_reviews.values().any(|state| state.busy)
            && doc.as_ref().is_some_and(|doc| doc.source.is_some());
        let count = doc.as_ref().map_or(0, |doc| doc.hunks.len());
        let split = self.diff_split.contains(&key);
        let focused = ui.ui_contains_pointer() || self.diff_tab_active(tab);
        let siblings = self.diff_siblings(staged && !read_only);
        let current_abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            cwd.join(path)
        };
        let (prev_file, next_file, file_pos, file_total) = diff_neighbors(&siblings, &current_abs);
        DiffFrame {
            tab,
            cwd,
            path,
            staged,
            key,
            header_path,
            doc,
            remote,
            read_only,
            enabled,
            count,
            split,
            focused,
            is_md: crate::markdown::supported(path),
            prev_file,
            next_file,
            file_pos,
            file_total,
        }
    }

    /// Focus must belong to a real widget. A synthetic focus ID has no
    /// AccessKit node and crashes its macOS consumer when window focus
    /// changes. Keys follow the pointer or the active pane, so F7 works
    /// without hovering the diff once it is the focused tab.
    fn diff_tab_active(&self, tab: &Tab) -> bool {
        self.selected
            .as_ref()
            .and_then(|project| self.layouts.get(project))
            .and_then(|workspace| workspace.active_pane())
            == Some(tab)
    }

    /// Sibling files on the same side (staged or working tree), in Git's
    /// reported order, for prev/next-file navigation.
    fn diff_siblings(&self, staged_side: bool) -> Vec<PathBuf> {
        self.diff_changed_files(staged_side)
            .into_iter()
            .map(|file| file.path)
            .collect()
    }

    /// The changed-files panel (when open) beside the diff body.
    fn diff_main(&mut self, ui: &mut egui::Ui, frame: &DiffFrame<'_>, pass: &mut DiffPass) {
        if !self.preferences.diff_file_tree || frame.remote {
            self.diff_body(ui, frame, pass);
            return;
        }
        let full = ui.available_rect_before_wrap();
        let tree_w = (full.width() * 0.3).clamp(140.0, 260.0);
        let tree = egui::Rect::from_min_size(full.min, egui::vec2(tree_w, full.height()));
        let body = egui::Rect::from_min_max(egui::pos2(tree.right() + 1.0, full.top()), full.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(tree), |ui| {
            ui.set_clip_rect(tree.intersect(ui.clip_rect()));
            self.diff_file_tree(ui, frame, pass);
        });
        ui.painter().vline(
            ui.painter().round_to_pixel_center(tree.right()),
            full.y_range(),
            ui.visuals().widgets.noninteractive.bg_stroke,
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
            self.diff_body(ui, frame, pass);
        });
        ui.allocate_rect(full, egui::Sense::hover());
    }

    /// F7 / Shift+F7 walk changes, and past the last (or first) change of
    /// this file continue into the next (or previous) file.
    fn diff_change_keys(ui: &mut egui::Ui, frame: &DiffFrame<'_>, pass: &mut DiffPass) {
        if !frame.focused || ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let count = frame.count;
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::F7)) {
            if count == 0 || pass.review.selected == count.checked_sub(1) {
                pass.navigate.clone_from(&frame.next_file);
            } else {
                pass.review.advance(true, count);
            }
        }
        if ui.input_mut(|input| input.consume_key(egui::Modifiers::SHIFT, egui::Key::F7)) {
            if count == 0 || pass.review.selected == Some(0) {
                pass.navigate.clone_from(&frame.prev_file);
            } else {
                pass.review.advance(false, count);
            }
        }
    }

    fn diff_open_source_key(ui: &mut egui::Ui, frame: &DiffFrame<'_>, pass: &mut DiffPass) {
        if frame.focused
            && !ui.ctx().egui_wants_keyboard_input()
            && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::F4))
        {
            pass.open_line = pass.selected_change_line(frame.doc.as_ref());
        }
    }

    /// A document built under other compare options (loaded before the
    /// options were applied, or changed from another diff) is rebuilt once
    /// with the current ones.
    fn sync_diff_options(&self, frame: &DiffFrame<'_>, pass: &mut DiffPass) {
        let wanted = self.preferences.diff_options;
        if let Some(doc) = &frame.doc
            && doc.options != wanted
            && pass.review.options_requested != Some(wanted)
            && !pass.review.busy
            && !pass.review.unsaved_edits()
            && !self.loading.contains(&frame.key)
        {
            pass.refresh = true;
            pass.review.options_requested = Some(wanted);
        }
    }

    fn request_missing_diff(&mut self, frame: &DiffFrame<'_>) {
        if !self.diffs.contains_key(&frame.key) && self.loading.insert(frame.key.clone()) {
            let _ = self.jobs.send(Job::Diff(frame.tab.clone()));
        }
    }

    /// Keep a live buffer of the working-tree side. It is re-seeded from a
    /// refreshed document only while it holds no unsaved edits.
    fn seed_diff_buffer(frame: &DiffFrame<'_>, review: &mut diff::review::State) {
        if !frame.read_only
            && !frame.staged
            && let Some(document) = &frame.doc
            && !document.read_only
            && review.buffer.as_ref().is_none_or(|buffer| {
                !std::sync::Arc::ptr_eq(&buffer.document, document)
                    && buffer.text == buffer.document.right_text
            })
        {
            review.buffer = Some(diff::review::Edit::new(std::sync::Arc::clone(document)));
        }
    }

    /// Check the review back in, then open, navigate, or swap as asked.
    fn finish_diff_view(&mut self, frame: DiffFrame<'_>, pass: DiffPass) {
        self.diff_reviews.insert(frame.key, pass.review);
        if let Some(line) = pass.open_line {
            self.open_file_mode(OpenFileModeInput {
                path: frame.cwd.join(frame.path),
                line: Some(line),
                split: None,
                external: false,
                text: true,
            });
        }
        if let Some(target) = pass.navigate {
            self.navigate_diff(frame.tab, target);
        }
        if pass.open_working {
            self.open_working_diff(frame.tab);
        }
    }
}
