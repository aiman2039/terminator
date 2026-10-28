use super::super::super::*;
use super::super::diff_edit::{
    PaintDeletedMarkersInput, PaintRightEditorInput, aligned_left, paint_deleted_markers,
    paint_right_editor,
};
use super::super::diff_paint::{
    DIFF_SPLIT_GAP, DiffContentWidthInput, DiffMetrics, PaintDiffPaneInput, SplitPane,
    diff_content_width, diff_doc_digits, diff_row_count, paint_diff_pane, split_rects,
};
use super::DiffPass;
use super::body::DiffPanes;

type PaneOutput = egui::scroll_area::ScrollAreaOutput<()>;

/// Shared geometry for the two panes of the editable split view.
struct SplitGeometry<'a> {
    metrics: &'a DiffMetrics,
    content: f32,
    rows: usize,
    left_rect: egui::Rect,
    right_rect: egui::Rect,
    frame_sync: f32,
}

struct PaintSplitRightInput<'a> {
    ui: &'a mut egui::Ui,
    panes: &'a DiffPanes<'a>,
    pass: &'a mut DiffPass,
    geometry: &'a SplitGeometry<'a>,
}

impl App {
    /// Working tree split view. The right pane is the whole buffer, one row
    /// per line; the left pane is laid out on those rows so both scroll in
    /// step (no collapsed or filler rows that only one side has).
    pub(super) fn paint_split_editor(
        &mut self,
        ui: &mut egui::Ui,
        panes: &mut DiffPanes<'_>,
        pass: &mut DiffPass,
    ) -> Vec<PaneOutput> {
        let frame = panes.frame;
        let doc = panes.doc;
        let metrics = DiffMetrics::measure(ui, diff_doc_digits(doc, true));
        let content = diff_content_width(DiffContentWidthInput {
            ui,
            doc,
            split: true,
            metrics: &metrics,
            scroll_key: &frame.key,
        });
        let aligned = pass
            .review
            .buffer
            .as_mut()
            .map(|buffer| aligned_left(buffer, &frame.cwd.join(frame.path), panes.colors));
        let (left_doc, rows) = match &aligned {
            Some(aligned) => (&aligned.doc, aligned.doc.split.len().max(1)),
            None => (doc.as_ref(), diff_row_count(doc, true)),
        };
        let (left_rect, gap_rect, right_rect) = split_rects(panes.body, self.diff_split_ratio);
        let geometry = SplitGeometry {
            metrics: &metrics,
            content,
            rows,
            left_rect,
            right_rect,
            frame_sync: panes.sync,
        };
        let left = paint_diff_pane(PaintDiffPaneInput {
            ui,
            rect: left_rect,
            doc: left_doc,
            pane: SplitPane::Left,
            colors: panes.colors,
            metrics: &metrics,
            content,
            rows,
            scroll_key: &frame.key,
            sync: geometry.frame_sync,
        });
        if let Some(aligned) = &aligned {
            paint_deleted_markers(PaintDeletedMarkersInput {
                ui,
                output: &left,
                deleted: &aligned.deleted,
                row_h: metrics.row_h,
                color: panes.colors.deleted,
                key: &frame.key,
            });
        }
        let right = Self::paint_split_right(PaintSplitRightInput {
            ui,
            panes,
            pass,
            geometry: &geometry,
        });
        self.drag_diff_split(ui, panes, gap_rect);
        panes.sync = Self::followed_split_offset(ui, &geometry, [&left, &right]);
        vec![left, right]
    }

    /// The right pane: the live buffer editor, or the plain right side.
    fn paint_split_right(input: PaintSplitRightInput<'_>) -> PaneOutput {
        let PaintSplitRightInput {
            ui,
            panes,
            pass,
            geometry,
        } = input;
        let frame = panes.frame;
        match &mut pass.review.buffer {
            Some(buffer) => paint_right_editor(PaintRightEditorInput {
                ui,
                rect: geometry.right_rect,
                buffer,
                path: &frame.cwd.join(frame.path),
                colors: panes.colors,
                sync: geometry.frame_sync,
                key: &frame.key,
            }),
            None => paint_diff_pane(PaintDiffPaneInput {
                ui,
                rect: geometry.right_rect,
                doc: panes.doc,
                pane: SplitPane::Right,
                colors: panes.colors,
                metrics: geometry.metrics,
                content: geometry.content,
                rows: geometry.rows,
                scroll_key: &frame.key,
                sync: geometry.frame_sync,
            }),
        }
    }

    /// The draggable divider between the panes, and its line.
    fn drag_diff_split(&mut self, ui: &mut egui::Ui, panes: &DiffPanes<'_>, gap_rect: egui::Rect) {
        let body = panes.body;
        let handle =
            egui::Rect::from_center_size(gap_rect.center(), egui::vec2(6.0, gap_rect.height()));
        let response = ui.interact(
            handle,
            ui.scope_id().with(("diff-split-drag", &panes.frame.key)),
            egui::Sense::drag(),
        );
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if response.dragged()
            && let Some(pos) = response.interact_pointer_pos()
        {
            let usable = (body.width() - DIFF_SPLIT_GAP).max(1.0);
            self.diff_split_ratio = ((pos.x - body.left()) / usable).clamp(0.05, 0.95);
        }
        ui.painter().vline(
            gap_rect.center().x,
            gap_rect.y_range(),
            egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
        );
    }

    /// The offset both panes follow next frame: the hovered pane's, else
    /// whichever pane moved further from the shared offset.
    fn followed_split_offset(
        ui: &egui::Ui,
        geometry: &SplitGeometry<'_>,
        [left, right]: [&PaneOutput; 2],
    ) -> f32 {
        let frame_sync = geometry.frame_sync;
        let pointer = ui.ctx().input(|input| input.pointer.hover_pos());
        let over_left = pointer.is_some_and(|pos| geometry.left_rect.contains(pos));
        let over_right = pointer.is_some_and(|pos| geometry.right_rect.contains(pos));
        match (over_left, over_right) {
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
        }
    }
}
