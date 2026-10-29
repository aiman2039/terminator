//! The painted file area of one frame: allocation, pointer mapping and
//! per-row cell geometry.
use super::SourceView;
use super::geometry::{Geometry, Viewport, snap};
use crate::doc::Buffer;
use crate::fold::Folds;
use crate::highlight::HighlightCache;
use crate::inlay::InlayHint;
use crate::rows::{RowCells, RowHint, VisualRow, row_hints};
use egui::{Pos2, Rect, Sense, Vec2};

/// Per-frame drawing context inside the scroll area: the allocated rect
/// and painter, its origin, this frame's row layout, and the visible rows.
pub(super) struct Canvas<'c> {
    pub ui: &'c egui::Ui,
    pub geometry: &'c Geometry,
    pub inlay_hints: &'c [InlayHint],
    pub rows: &'c [VisualRow],
    pub folds: &'c Folds,
    pub highlight: &'c HighlightCache,
    pub response: egui::Response,
    pub painter: egui::Painter,
    pub origin: Pos2,
    pub width: f32,
    pub clip: Rect,
    /// Only the rows inside the clip rect: a long file's off-screen rows
    /// cost nothing per frame.
    pub first_row: usize,
    pub last_row: usize,
}

impl<'a, B: Buffer> SourceView<'a, B> {
    /// Allocate the file area and record its rect and activation.
    pub(super) fn open_canvas<'c>(
        &mut self,
        ui: &'c mut egui::Ui,
        viewport: &Viewport,
        highlight: &'c HighlightCache,
    ) -> Canvas<'c>
    where
        'a: 'c,
    {
        let geometry: &'a Geometry = self.geometry;
        let width = if viewport.h_scroll {
            viewport.content_width
        } else {
            ui.available_width().max(geometry.gutter + geometry.advance)
        };
        let (response, painter) = ui.allocate_painter(
            Vec2::new(width, viewport.content_height.max(viewport.max_scroll)),
            Sense::click_and_drag(),
        );
        // I-beam over file text; the default Sense::click pointer reads
        // as "clickable chrome" instead of an editor.
        let response = response.on_hover_cursor(egui::CursorIcon::Text);
        self.out.content_rect = response.rect;
        self.out.activated = response.clicked() || response.secondary_clicked();
        let rows = highlight.rows();
        // Rows, gutter and text start on whole physical pixels; the
        // scroll offset may be fractional.
        let origin = Pos2::new(
            snap(response.rect.min.x, geometry.ppp),
            snap(response.rect.min.y, geometry.ppp),
        );
        let clip = painter.clip_rect();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "Row indices from UI coordinates."
        )]
        let (first_row, last_row) = (
            ((clip.top() - origin.y) / geometry.row_height)
                .floor()
                .max(0.0) as usize,
            (((clip.bottom() - origin.y) / geometry.row_height)
                .ceil()
                .max(0.0) as usize)
                .saturating_add(1)
                .min(rows.len()),
        );
        let options: &'a super::SourceOptions = self.options;
        Canvas {
            ui,
            geometry,
            inlay_hints: &options.inlay_hints,
            rows,
            folds: highlight.folds(),
            highlight,
            response,
            painter,
            origin,
            width,
            clip,
            first_row,
            last_row,
        }
    }
}

impl Canvas<'_> {
    /// Pointer mapping: painted row and display cell back to a buffer
    /// position. A pointer below the last row clamps to it, so a drag
    /// that leaves the pane keeps selecting.
    pub fn cursor_at(&self, pos: Pos2) -> Option<(usize, usize)> {
        let Geometry {
            advance,
            row_height,
            gutter,
            ..
        } = *self.geometry;
        let origin = self.response.rect.min;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "UI coordinate"
        )]
        let row_index = ((pos.y - origin.y) / row_height).floor().max(0.0) as usize;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "UI coordinate"
        )]
        let cells = ((pos.x - origin.x - gutter) / advance).round().max(0.0) as usize;
        // Wide chars occupy two cells: resolve the display cell back to
        // a char column on the hit row.
        let row = self.rows.get(row_index).or_else(|| self.rows.last())?;
        let hints = row_hints(self.inlay_hints, row);
        let geometry = RowCells {
            text: &row.text,
            hints: &hints,
        };
        Some((row.line, row.start.saturating_add(geometry.col_at(cells))))
    }

    /// The pointer position of this frame's click on the file area.
    pub fn click_pos(&self) -> Option<Pos2> {
        self.response
            .clicked()
            .then(|| self.response.interact_pointer_pos())
            .flatten()
    }

    pub fn request_focus(&self) {
        let focus_id = self.geometry.focus_id;
        self.ui.memory_mut(|memory| memory.request_focus(focus_id));
    }

    /// Screen x of display cell `cells` in the text area, snapped to a
    /// physical pixel.
    pub fn text_x(&self, cells: usize) -> f32 {
        snap(
            self.origin.x + self.geometry.gutter + cells as f32 * self.geometry.advance,
            self.geometry.ppp,
        )
    }

    /// The rows inside the clip rect, with their layout index.
    pub fn visible_rows(&self) -> impl Iterator<Item = (usize, &VisualRow)> {
        self.rows
            .iter()
            .enumerate()
            .skip(self.first_row)
            .take(self.last_row.saturating_sub(self.first_row))
    }
}

/// One painted row: its layout, inline-hint geometry and screen y.
pub(super) struct PaintRow<'r> {
    pub visual: &'r VisualRow,
    pub cells: RowCells<'r>,
    pub y: f32,
}

impl<'r> PaintRow<'r> {
    pub fn new(visual: &'r VisualRow, hints: &'r [RowHint<'r>], y: f32) -> Self {
        Self {
            visual,
            cells: RowCells {
                text: &visual.text,
                hints,
            },
            y,
        }
    }

    pub fn line(&self) -> usize {
        self.visual.line
    }

    /// Background for chars `start_col..end_col` clipped to this row, or
    /// `None` when nothing of it is on the row. Char columns map to
    /// display cells: wide chars shift every cell after them.
    pub fn span_rect(&self, canvas: &Canvas<'_>, start_col: usize, end_col: usize) -> Option<Rect> {
        let visual = self.visual;
        let (start_col, end_col) = (start_col.max(visual.start), end_col.min(visual.end));
        if start_col >= end_col {
            return None;
        }
        let start_cell = self.cells.char_x(start_col.saturating_sub(visual.start));
        let end_cell = self.cells.caret_x(end_col.saturating_sub(visual.start));
        Some(Rect::from_min_max(
            Pos2::new(canvas.text_x(start_cell), self.y),
            Pos2::new(
                canvas.text_x(end_cell.max(start_cell)),
                self.y + canvas.geometry.row_height,
            ),
        ))
    }

    /// Rect of `cells` chars from buffer column `col`, at least one cell wide.
    pub fn cell_rect(&self, canvas: &Canvas<'_>, col: usize, cells: usize) -> Rect {
        let local = col.saturating_sub(self.visual.start);
        let start = self.cells.char_x(local);
        let end = self
            .cells
            .caret_x(local.saturating_add(cells))
            .max(start.saturating_add(1));
        Rect::from_min_max(
            Pos2::new(canvas.text_x(start), self.y),
            Pos2::new(canvas.text_x(end), self.y + canvas.geometry.row_height),
        )
    }
}
