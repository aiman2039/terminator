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
            ui.colored_label(ui.visuals().error_fg_color, error);
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
        DiffColors {
            added: appearance::color(&self.theme.git_added),
            deleted: appearance::color(&self.theme.git_deleted),
            accent: appearance::color(&self.theme.accent),
            text: appearance::color(&self.theme.text),
        }
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
            // Working tree: split right pane and unified rows are live
            // editors beside/between the painted lines.
            editable: !frame.read_only && split && !frame.staged && pass.review.buffer.is_some(),
            sync,
        };
        ui.scope_builder(egui::UiBuilder::new().max_rect(body), |ui| {
            ui.set_clip_rect(full);
            let outputs = self.diff_pane_outputs(ui, &mut panes, pass);
            Self::diff_pane_controls(ui, &mut panes, pass, &outputs);
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
        let frame = panes.frame;
        let unified_editor =
            !frame.read_only && !panes.split && !frame.staged && pass.review.buffer.is_some();
        if unified_editor {
            return match &mut pass.review.buffer {
                Some(buffer) => vec![super::super::diff_edit::paint_unified_editor(
                    super::super::diff_edit::PaintUnifiedEditorInput {
                        ui,
                        doc: panes.doc,
                        buffer,
                        path: &frame.cwd.join(frame.path),
                        colors: panes.colors,
                        key: &frame.key,
                    },
                )],
                None => self.paint_plain_diff(ui, panes),
            };
        }
        if panes.editable {
            return self.paint_split_editor(ui, panes, pass);
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
            doc: panes.doc,
            split: panes.split,
            colors: panes.colors,
            scroll_key: &panes.frame.key,
            ratio: &mut self.diff_split_ratio,
            sync: &mut panes.sync,
        })
    }

    /// Hunk rail and gutter controls over the painted panes, then the
    /// pending jump to the selected change.
    fn diff_pane_controls(
        ui: &mut egui::Ui,
        panes: &mut DiffPanes<'_>,
        pass: &mut DiffPass,
        outputs: &[PaneOutput],
    ) {
        let frame = panes.frame;
        let interaction = diff_controls::paint(diff_controls::PaintInput {
            ui,
            doc: panes.doc,
            split: panes.split,
            staged: frame.staged,
            editor_pane: panes.editable
                || (!panes.split && !frame.staged && pass.review.buffer.is_some()),
            enabled: frame.enabled,
            review: &mut pass.review,
            scroll: outputs,
            rail: panes.rail,
            unified_gutter: panes.unified_gutter,
            colors: panes.colors,
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
            doc: panes.doc,
            split: panes.split,
            aligned: panes.editable,
            review: &mut pass.review,
            scroll: outputs,
            sync: &mut panes.sync,
        });
    }
}
