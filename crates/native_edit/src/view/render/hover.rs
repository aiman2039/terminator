//! The text under the pointer, for a host hover popup (Quick
//! Documentation): an identifier, or the one char under the pointer.
use super::SourceView;
use super::canvas::{Canvas, PaintRow};
use crate::doc::Buffer;
use crate::rows::{RowCells, row_hints};
use egui::Pos2;

/// The text under the pointer, in buffer line and char columns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HoverSpot {
    pub line: usize,
    /// The char under the pointer.
    pub col: usize,
    /// The identifier that holds `col`, else just `col..col + 1`.
    pub start: usize,
    pub end: usize,
    /// `start..end` is an identifier.
    pub word: bool,
    /// Screen rect of `start..end` on the hovered row, to place a popup.
    pub rect: egui::Rect,
}

/// Char columns of the identifier (letters, digits, `_`) that holds char
/// `col` of `text`; `None` when that char is not part of one.
#[must_use]
pub fn word_span(text: &str, col: usize) -> Option<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let word = |at: usize| {
        chars
            .get(at)
            .is_some_and(|c| c.is_alphanumeric() || *c == '_')
    };
    if !word(col) {
        return None;
    }
    let mut start = col;
    while start > 0 && word(start.saturating_sub(1)) {
        start = start.saturating_sub(1);
    }
    let mut end = col;
    while word(end) {
        end = end.saturating_add(1);
    }
    Some((start, end))
}

impl<B: Buffer> SourceView<'_, B> {
    /// Report the text under the pointer for the host popup: only while
    /// the pointer is over this view's text and no button is down (no
    /// click, drag or selection in progress).
    pub(super) fn report_hover(&mut self, canvas: &Canvas<'_>) {
        if !self.options.hover_popup
            || !canvas.response.hovered()
            || canvas.ui.input(|input| input.pointer.any_down())
        {
            return;
        }
        if let Some(pos) = canvas.response.hover_pos() {
            self.out.hover = self.spot_at(canvas, pos);
        }
    }

    /// The char under `pos`: not the gutter, an inlay hint, or the space
    /// after a line (an empty line keeps its first cell, where a problem
    /// on it is underlined).
    fn spot_at(&self, canvas: &Canvas<'_>, pos: Pos2) -> Option<HoverSpot> {
        let geometry = self.geometry;
        if pos.x < canvas.gutter_x + geometry.rule_x {
            return None;
        }
        let row = (pos.y - canvas.origin.y) / geometry.row_height;
        let cell = (pos.x - canvas.origin.x - geometry.gutter) / geometry.advance;
        if row < 0.0 || cell < 0.0 {
            return None;
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "Non-negative UI coordinates become row and cell indices."
        )]
        let (row, cell) = (row.floor() as usize, cell.floor() as usize);
        let visual = canvas.rows.get(row).filter(|visual| !visual.spacer)?;
        let hints = row_hints(canvas.inlay_hints, visual);
        let cells = RowCells {
            text: &visual.text,
            hints: &hints,
            tab: geometry.tab,
        };
        let local = cells.col_at(cell);
        let len = visual.end.saturating_sub(visual.start);
        if cell < cells.char_x(local) || local >= len.max(1) || (len == 0 && cell > 0) {
            return None;
        }
        let col = visual.start.saturating_add(local);
        let text = self.doc.line_text(visual.line);
        let (start, end, word) = match word_span(&text, col) {
            Some((start, end)) => (start, end, true),
            None => (col, col.saturating_add(1), false),
        };
        // The part of the word on this row (a wrapped word spans rows).
        let from = start.max(visual.start);
        let to = end.min(visual.end).max(from.saturating_add(1));
        #[expect(clippy::cast_precision_loss, reason = "Row indices are small.")]
        let y = canvas.origin.y + row as f32 * geometry.row_height;
        let rect = PaintRow::new(visual, &hints, y, geometry.tab).cell_rect(
            canvas,
            from,
            to.saturating_sub(from),
        );
        Some(HoverSpot {
            line: visual.line,
            col,
            start,
            end,
            word,
            rect,
        })
    }
}
