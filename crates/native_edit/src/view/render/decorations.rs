//! Host decorations: filler rows and line / char-range backgrounds (a diff
//! viewer's alignment rows, row tints and word marks).
use super::SourceView;
use super::canvas::{Canvas, PaintRow};
use crate::doc::Buffer;
use egui::{Pos2, Rect};

impl<B: Buffer> SourceView<'_, B> {
    /// A filler row: only its background, from the gutter rule to the
    /// right edge.
    pub(super) fn paint_spacer(&self, canvas: &Canvas<'_>, y: f32) {
        let Some(fill) = self.options.spacer_fill else {
            return;
        };
        canvas
            .painter
            .rect_filled(self.text_band(canvas, y), 0.0, fill);
    }

    /// The row's line fill across the text area, then its range fills.
    pub(super) fn paint_fills(&self, canvas: &Canvas<'_>, row: &PaintRow<'_>) {
        let line = row.line();
        let line_fills = &self.options.line_fills;
        let first = line_fills.partition_point(|fill| fill.line < line);
        for fill in line_fills
            .get(first..)
            .unwrap_or_default()
            .iter()
            .take_while(|fill| fill.line == line)
        {
            canvas
                .painter
                .rect_filled(self.text_band(canvas, row.y), 0.0, fill.color);
        }
        let range_fills = &self.options.range_fills;
        let first = range_fills.partition_point(|fill| fill.line < line);
        for fill in range_fills
            .get(first..)
            .unwrap_or_default()
            .iter()
            .take_while(|fill| fill.line == line)
        {
            if let Some(rect) = row.span_rect(canvas, fill.start, fill.end) {
                canvas.painter.rect_filled(rect, 0.0, fill.color);
            }
        }
    }

    /// One row's band right of the gutter rule.
    fn text_band(&self, canvas: &Canvas<'_>, y: f32) -> Rect {
        Rect::from_min_max(
            Pos2::new(canvas.origin.x + self.geometry.rule_x + 1.0, y),
            Pos2::new(canvas.clip.right(), y + self.geometry.row_height),
        )
    }
}
