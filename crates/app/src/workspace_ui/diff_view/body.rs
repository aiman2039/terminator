use super::super::super::*;
use super::super::diff_controls;
use super::super::diff_paint::{
    ComparisonHeadersInput, DiffColors, PaintDiffDocumentInput, PaintMarkdownDiffPreviewInput,
    comparison_headers, paint_diff_document, paint_markdown_diff_preview,
};
use super::{DiffFrame, DiffPass};
use crate::app::OpenFileInput;
use std::sync::Arc;

type PaneOutput = egui::scroll_area::ScrollAreaOutput<()>;

/// Per-frame layout and scroll state of the diff document panes.
pub(super) struct DiffPanes<'a> {
    pub(super) frame: &'a DiffFrame<'a>,
    pub(super) doc: &'a Arc<diff::DiffDocument>,
    pub(super) split: bool,
    pub(super) colors: DiffColors,
    pub(super) rail: egui::Rect,
    pub(super) unified_gutter: egui::Rect,
    pub(super) body: egui::Rect,
    /// Working tree split view: the right pane is a live editor.
    pub(super) editable: bool,
    /// Shared vertical offset that keeps both split panes in step.
    pub(super) sync: f32,
    /// The live re-diff of the buffer when the panes show it instead of
    /// `doc` (the editable split, or unified with unsaved edits).
    pub(super) live: Option<Arc<diff::DiffDocument>>,
    /// Row pitch of the editable split (the editor's), when it is shown.
    pub(super) row_h: Option<f32>,
}

struct DiffPaneControlsInput<'a, 'p> {
    ui: &'a mut egui::Ui,
    panes: &'a mut DiffPanes<'p>,
    pass: &'a mut DiffPass,
    outputs: &'a [PaneOutput],
}

impl App {
    pub(super) fn diff_body(
        &mut self,
        ui: &mut egui::Ui,
        frame: &DiffFrame<'_>,
        pass: &mut DiffPass,
    ) {
        if frame.doc.is_some() {
            self.diff_document(ui, frame, pass);
        } else if let Some(Err(error)) = self.diffs.get(&frame.key) {
            // Binary images and conflicted files get their own tools.
            let error = error.clone();
            if !self.binary_image_diff(ui, frame.tab, &error) {
                ui.colored_label(ui.visuals().error_fg_color, &error);
                self.offer_merge(ui, frame.tab, &error);
            }
        } else {
            ui.spinner();
        }
    }

    fn diff_document(&mut self, ui: &mut egui::Ui, frame: &DiffFrame<'_>, pass: &mut DiffPass) {
        let Some(doc) = &frame.doc else {
            return;
        };
        let split = self.diff_split.contains(&frame.key);
        comparison_headers(ComparisonHeadersInput {
            ui,
            doc,
            split,
            ratio: self.diff_split_ratio,
            path: &frame.header_path,
            editable: !frame.staged && !frame.read_only && !doc.read_only,
            staged: frame.staged,
        });
        if doc.hunks.is_empty() && doc.unified.is_empty() {
            ui.weak("No text changes");
        }
        if frame.is_md && self.diff_preview.contains(&frame.key) {
            self.diff_markdown_preview(ui, frame, doc);
        } else {
            self.diff_panes(ui, frame, pass);
        }
    }

    fn diff_markdown_preview(
        &mut self,
        ui: &mut egui::Ui,
        frame: &DiffFrame<'_>,
        doc: &diff::DiffDocument,
    ) {
        let Some(link) = paint_markdown_diff_preview(PaintMarkdownDiffPreviewInput {
            ui,
            doc,
            previews: &mut self.markdown,
            path: &frame.cwd.join(frame.path),
            scroll_key: &frame.key,
        }) else {
            return;
        };
        match link {
            markdown::Link::File(path) => self.open_file(OpenFileInput {
                path,
                line: None,
                split: None,
                external: false,
            }),
            markdown::Link::Web(url) => {
                let _ = self.jobs.send(Job::Browser(url));
            }
        }
    }

    fn diff_colors(&self) -> DiffColors {
        DiffColors::from_theme(&self.theme)
    }

    /// The document panes inside the body rect, with the hunk rail and
    /// unified gutter controls layered over them.
    fn diff_panes(&mut self, ui: &mut egui::Ui, frame: &DiffFrame<'_>, pass: &mut DiffPass) {
        let Some(doc) = &frame.doc else {
            return;
        };
        let split = self.diff_split.contains(&frame.key);
        let colors = self.diff_colors();
        let sync = self
            .diff_split_scroll
            .get(&frame.key)
            .copied()
            .unwrap_or(0.0);
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
        let mut panes = DiffPanes {
            frame,
            doc,
            split,
            colors,
            rail,
            unified_gutter,
            body,
            // Working tree: the split right pane is the native editor.
            editable: !frame.read_only && split && !frame.staged && pass.review.buffer.is_some(),
            sync,
            live: None,
            row_h: None,
        };
        ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
            ui.set_clip_rect(full);
            let outputs = self.diff_pane_outputs(ui, &mut panes, pass);
            Self::diff_pane_controls(DiffPaneControlsInput {
                ui,
                panes: &mut panes,
                pass,
                outputs: &outputs,
            });
        });
        ui.allocate_rect(full, egui::Sense::hover());
        self.diff_split_scroll.insert(frame.key.clone(), panes.sync);
    }

    fn diff_pane_outputs(
        &mut self,
        ui: &mut egui::Ui,
        panes: &mut DiffPanes<'_>,
        pass: &mut DiffPass,
    ) -> Vec<PaneOutput> {
        if panes.editable {
            return self.paint_split_editor(ui, panes, pass);
        }
        // Unified working tree is read-only; with unsaved edits (made in
        // the split editor) it shows the live diff of the buffer.
        if !panes.split
            && !panes.frame.staged
            && pass.review.unsaved_edits()
            && let Some(buffer) = pass.review.buffer.as_mut()
        {
            let path = panes.frame.cwd.join(panes.frame.path);
            let (live, waiting) = buffer.live_document(&path, ui.input(|input| input.time));
            if waiting {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(50));
            }
            panes.live = Some(live);
        }
        self.paint_plain_diff(ui, panes)
    }

    fn paint_plain_diff(
        &mut self,
        ui: &mut egui::Ui,
        panes: &mut DiffPanes<'_>,
    ) -> Vec<PaneOutput> {
        paint_diff_document(PaintDiffDocumentInput {
            ui,
            doc: panes.live.as_ref().unwrap_or(panes.doc),
            split: panes.split,
            colors: panes.colors,
            scroll_key: &panes.frame.key,
            ratio: &mut self.diff_split_ratio,
            sync: &mut panes.sync,
        })
    }

    /// Hunk rail and gutter controls over the painted panes, then the
    /// pending jump to the selected change.
    fn diff_pane_controls(input: DiffPaneControlsInput<'_, '_>) {
        let DiffPaneControlsInput {
            ui,
            panes,
            pass,
            outputs,
        } = input;
        let frame = panes.frame;
        let doc = panes.live.as_ref().unwrap_or(panes.doc);
        // Hunk actions address the document's hunks: a live diff of
        // unsaved edits has other hunks, so they wait for Save.
        let enabled = frame.enabled && (panes.live.is_none() || !pass.review.unsaved_edits());
        let interaction = diff_controls::paint(diff_controls::PaintInput {
            ui,
            doc,
            split: panes.split,
            staged: frame.staged,
            editor_pane: panes.editable,
            enabled,
            review: &mut pass.review,
            scroll: outputs,
            rail: panes.rail,
            unified_gutter: panes.unified_gutter,
            colors: panes.colors,
            row_h: panes.row_h,
        });
        if interaction.action.is_some() {
            pass.action = interaction.action;
        }
        if interaction.open_line.is_some() {
            pass.open_line = interaction.open_line;
        }
        pass.gap = interaction.gap;
        diff_controls::jump(diff_controls::JumpInput {
            ui,
            doc,
            split: panes.split,
            row_h: panes.row_h,
            review: &mut pass.review,
            scroll: outputs,
            sync: &mut panes.sync,
        });
    }
}
