//! Indent guides: one hairline per indent level a line sits inside, one
//! level of the document's indent style apart.
use super::canvas::{Canvas, PaintRow};
use super::geometry::snap;
use crate::rows::display_x;
use egui::Color32;

/// Display cells of `text`'s leading spaces and tabs (tabs `tab` wide);
/// `None` for a blank line, which keeps the guides of the line above.
pub(super) fn lead_cells(text: &str, tab: usize) -> Option<usize> {
    let lead = text.chars().take_while(|c| *c == ' ' || *c == '\t').count();
    (lead < text.chars().count()).then(|| display_x(text, lead, tab))
}

/// Cells where guides stand inside a lead `cells` wide: one per indent
/// level of `level` cells.
pub(super) fn guide_stops(cells: usize, level: usize) -> impl Iterator<Item = usize> {
    (0..cells).step_by(level.max(1))
}

impl Canvas<'_> {
    /// One guide per indent level the line sits inside; returns the
    /// guide width the next row inherits.
    pub(super) fn paint_indent_guides(
        &self,
        row: &PaintRow<'_>,
        guide_cells: usize,
        color: Color32,
    ) -> usize {
        if row.visual.start != 0 {
            return guide_cells;
        }
        let guide_cells = lead_cells(&row.visual.text, self.geometry.tab).unwrap_or(guide_cells);
        let y = row.y;
        let ppp = self.geometry.ppp;
        for cell in guide_stops(guide_cells, self.geometry.indent) {
            // Centered on a physical pixel so the 1px guide stays sharp.
            let x = snap(self.text_x(cell) + 1.0, ppp) + 0.5 / ppp;
            self.painter.vline(
                x,
                y..=y + self.geometry.row_height,
                egui::Stroke::new(1.0 / ppp, color),
            );
        }
        guide_cells
    }
}
