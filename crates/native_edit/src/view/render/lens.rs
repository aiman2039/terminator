//! Error lens: a line's worst diagnostic message, painted after its text.
use super::SourceView;
use super::canvas::{Canvas, PaintRow};
use super::helpers::RenderHelpers;
use crate::doc::Buffer;
use crate::lsp::Severity;
use egui::{Align2, Pos2};

/// Gap between the line's text and the message, in cells.
const GAP_CELLS: usize = 3;

impl<B: Buffer> SourceView<'_, B> {
    /// On the last row of a line, the first line of its worst diagnostic.
    pub(super) fn paint_error_lens(&self, canvas: &Canvas<'_>, row: &PaintRow<'_>) {
        let visual = row.visual;
        if !self.options.error_lens || visual.end != visual.line_len {
            return;
        }
        let squiggles = &self.options.squiggles;
        let first = squiggles.partition_point(|squiggle| squiggle.line < visual.line);
        let Some(worst) = squiggles
            .get(first..)
            .unwrap_or_default()
            .iter()
            .take_while(|squiggle| squiggle.line == visual.line)
            .filter(|squiggle| !matches!(squiggle.severity, Severity::Spelling))
            .min_by_key(|squiggle| squiggle.severity)
        else {
            return;
        };
        let message = worst.message.lines().next().unwrap_or_default();
        if message.is_empty() {
            return;
        }
        let end = row
            .cells
            .caret_x(visual.end.saturating_sub(visual.start))
            .saturating_add(GAP_CELLS);
        let color =
            RenderHelpers::severity_color(canvas.ui.visuals(), worst.severity).gamma_multiply(0.85);
        canvas.painter.text(
            Pos2::new(canvas.text_x(end), row.y + self.geometry.row_height * 0.5),
            Align2::LEFT_CENTER,
            message,
            self.geometry.italic_font.clone(),
            color,
        );
    }
}
