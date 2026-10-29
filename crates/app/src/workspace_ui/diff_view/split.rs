use super::super::super::*;
use super::super::diff_edit::{
    DiffEditorOptionsInput, PaintRightEditorInput, diff_editor_options, paint_right_editor,
};
use super::super::diff_paint::{
    DIFF_SPLIT_GAP, DiffColors, DiffContentWidthInput, DiffMetrics, PaintDiffPaneInput, SplitPane,
    diff_content_width, diff_doc_digits, paint_diff_pane, split_rects,
};
use super::DiffPass;
use super::body::DiffPanes;
use terminator_native_edit::highlight::language_for_path;
use terminator_native_edit::view::source_row_height;

type PaneOutput = egui::scroll_area::ScrollAreaOutput<()>;

/// Editor marks are painted in the theme's change colors and tint tokens.
pub(super) fn mark_colors(colors: DiffColors) -> diff::live::MarkColors {
    let tints = colors.tints();
    diff::live::MarkColors {
        added: colors.added,
        modified: colors.accent,
        row: tints.row,
        word: tints.word,
    }
}

impl App {
    /// Working tree split view: the left pane paints the original text of
    /// the live diff (deleted lines included), and the right pane is the
    /// native editor over the buffer, with filler rows where the left side
    /// has rows the buffer does not. Both share one row pitch and one
    /// vertical offset, so the connector band and chevrons sit on the
    /// rows they join.
    pub(super) fn paint_split_editor(
        &mut self,
        ui: &mut egui::Ui,
        panes: &mut DiffPanes<'_>,
        pass: &mut DiffPass,
    ) -> Vec<PaneOutput> {
        let frame = panes.frame;
        let busy = pass.review.busy;
        let Some(buffer) = pass.review.buffer.as_mut() else {
            return Vec::new();
        };
        let path = frame.cwd.join(frame.path);
        let (live, waiting) = buffer.live_document(&path, ui.input(|input| input.time));
        if waiting {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(50));
        }
        let marks = buffer.editor_marks(&live, mark_colors(panes.colors));
        let options = diff_editor_options(DiffEditorOptionsInput {
            vim: self.state.settings.native_vim,
            read_only: busy,
            marks: &marks,
            sync: panes.sync,
        });
        let row_h = source_row_height(ui, &options);
        let metrics = DiffMetrics::with_rows(
            ui,
            diff_doc_digits(&live, true),
            egui::FontId::monospace(options.font_size),
            row_h,
        );
        let content = diff_content_width(DiffContentWidthInput {
            ui,
            doc: &live,
            split: true,
            metrics: &metrics,
            scroll_key: &frame.key,
        });
        let (left_rect, gap_rect, right_rect) = split_rects(panes.body, self.diff_split_ratio);
        let left = paint_diff_pane(PaintDiffPaneInput {
            ui,
            rect: left_rect,
            doc: &live,
            pane: SplitPane::Left,
            colors: panes.colors,
            metrics: &metrics,
            content,
            rows: live.split.len().max(1),
            scroll_key: &frame.key,
            sync: panes.sync,
        });
        let right = paint_right_editor(PaintRightEditorInput {
            ui,
            rect: right_rect,
            buffer,
            options: &options,
            language: language_for_path(&path.to_string_lossy()),
            key: &frame.key,
        });
        pass.save_edits |= right.save && pass.review.unsaved_edits();
        self.drag_diff_split(ui, panes, gap_rect);
        panes.sync = Self::followed_split_offset(
            ui,
            panes.sync,
            [(left_rect, &left), (right_rect, &right.pane)],
        );
        panes.live = Some(live);
        panes.row_h = Some(row_h);
        vec![left, right.pane]
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
    /// whichever pane moved further from the shared offset (a wheel turn,
    /// or the editor following its caret).
    fn followed_split_offset(
        ui: &egui::Ui,
        frame_sync: f32,
        [(left_rect, left), (right_rect, right)]: [(egui::Rect, &PaneOutput); 2],
    ) -> f32 {
        let pointer = ui.ctx().input(|input| input.pointer.hover_pos());
        let over_left = pointer.is_some_and(|pos| left_rect.contains(pos));
        let over_right = pointer.is_some_and(|pos| right_rect.contains(pos));
        let (left, right) = (
            left.state.unclamped_offset().y,
            right.state.unclamped_offset().y,
        );
        match (over_left, over_right) {
            (true, false) if left != frame_sync => left,
            (false, true) if right != frame_sync => right,
            _ if (left - frame_sync).abs() >= (right - frame_sync).abs() => left,
            _ => right,
        }
    }
}
