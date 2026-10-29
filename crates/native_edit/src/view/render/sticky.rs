//! Sticky lines: headers of the blocks enclosing the top visible row stay
//! pinned while their first line is scrolled away.
use super::SourceView;
use super::canvas::Canvas;
use super::row_paint::RowPaint;
use crate::doc::Buffer;
use egui::{Pos2, Rect, Sense};

/// At most this many enclosing headers stay pinned.
const STICKY_HEADERS: usize = 3;

impl<B: Buffer> SourceView<'_, B> {
    pub(super) fn paint_sticky_headers(&mut self, canvas: &Canvas<'_>) {
        let Some(top) = canvas
            .rows
            .get(canvas.first_row)
            .filter(|_| canvas.first_row > 0)
        else {
            return;
        };
        let headers = canvas.folds.enclosing(top.line);
        let headers: Vec<(usize, usize)> = headers
            .iter()
            .rev()
            .take(STICKY_HEADERS)
            .rev()
            .copied()
            .collect();
        for (index, (first, _)) in headers.iter().enumerate() {
            self.paint_sticky_header(canvas, index, *first);
        }
        if headers.is_empty() {
            return;
        }
        let bottom = canvas.clip.top() + headers.len() as f32 * self.geometry.row_height;
        canvas.painter.hline(
            canvas.origin.x..=canvas.clip.right(),
            bottom,
            egui::Stroke::new(
                1.0,
                canvas.ui.visuals().widgets.noninteractive.bg_stroke.color,
            ),
        );
    }

    /// Header `index` (from the top) showing buffer line `first`; a click
    /// jumps to it.
    fn paint_sticky_header(&mut self, canvas: &Canvas<'_>, index: usize, first: usize) {
        let geometry = self.geometry;
        let visuals = canvas.ui.visuals();
        let origin = canvas.origin;
        let y = canvas.clip.top() + index as f32 * geometry.row_height;
        let band = Rect::from_min_max(
            Pos2::new(origin.x, y),
            Pos2::new(canvas.clip.right(), y + geometry.row_height),
        );
        canvas
            .painter
            .rect_filled(band, 0.0, visuals.extreme_bg_color);
        if self.options.show_line_numbers {
            canvas.painter.text(
                Pos2::new(
                    origin.x + geometry.rule_x - geometry.advance,
                    y + geometry.text_dy,
                ),
                egui::Align2::RIGHT_TOP,
                format!("{}", first.saturating_add(1)),
                geometry.font.clone(),
                visuals.weak_text_color(),
            );
        }
        let header = self.doc.line_text(first);
        RowPaint {
            painter: &canvas.painter,
            text: &header,
            hints: &[],
            spans: canvas.highlight.line(first),
            x: origin.x + geometry.gutter,
            y: y + geometry.text_dy,
            geometry,
            plain: visuals.text_color(),
            theme_plain: crate::highlight::theme(visuals.dark_mode)
                .and_then(|theme| theme.settings.foreground)
                .map(|color| [color.r, color.g, color.b]),
        }
        .paint();
        let hit = canvas
            .ui
            .interact(
                band,
                geometry.focus_id.with(("sticky-line", first)),
                Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if hit.clicked() {
            let col = self
                .doc
                .line_text(first)
                .chars()
                .take_while(|c| c.is_whitespace())
                .count();
            self.out
                .effects
                .extend(self.engine.place_cursor(self.doc, first, col));
            self.engine.request_scroll();
        }
    }
}
