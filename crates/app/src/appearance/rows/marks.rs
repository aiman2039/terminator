//! Row marks: the hover and selection fill, the active row's accent bar,
//! and the live-state dot at the right edge.
use super::super::scale::RADIUS_SMALL;
use super::counters::{RowExtras, RowSelection};
use eframe::egui;

/// Width kept at the right edge for the [`RowExtras::status_dot`].
const STATUS_DOT_SLOT: f32 = 14.0;
/// Width of the [`RowSelection::Bar`] accent bar and its inset from the
/// row's top and bottom (inside the rounded fill).
const SELECTION_BAR_WIDTH: f32 = 2.0;
const SELECTION_BAR_INSET: f32 = 3.0;

/// Hover fill on every row; the selection fill on a selected row unless
/// it shows its selection as text; plus the accent bar on a
/// [`RowSelection::Bar`] row.
pub(super) fn paint_row_fill(
    ui: &egui::Ui,
    response: &egui::Response,
    selected: bool,
    selection: RowSelection,
) {
    let filled = selected && selection != RowSelection::Text;
    if response.hovered() || filled {
        ui.painter().rect_filled(
            response.rect,
            RADIUS_SMALL,
            if filled {
                ui.visuals().selection.bg_fill
            } else {
                ui.visuals().widgets.hovered.bg_fill
            },
        );
    }
    if selected && selection == RowSelection::Bar {
        let rect = response.rect;
        ui.painter().rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.top() + SELECTION_BAR_INSET),
                egui::pos2(
                    rect.left() + SELECTION_BAR_WIDTH,
                    rect.bottom() - SELECTION_BAR_INSET,
                ),
            ),
            egui::CornerRadius::ZERO,
            ui.visuals().selection.stroke.color,
        );
    }
}

/// The part of `row` left for counters and trailing text: the state dot
/// owns the right edge.
pub(super) fn trailing_area(row: egui::Rect, extras: &RowExtras<'_>) -> egui::Rect {
    if extras.status_dot.is_some() {
        row.with_max_x(row.right() - STATUS_DOT_SLOT)
    } else {
        row
    }
}

/// The live-state dot at the right edge of `row`, centered on `y`.
pub(super) fn paint_edge_dot(ui: &egui::Ui, row: egui::Rect, y: f32, extras: &RowExtras<'_>) {
    if let Some(color) = extras.status_dot {
        super::paint_status_dot(
            ui.painter(),
            egui::pos2(row.right() - STATUS_DOT_SLOT * 0.5 - 1.0, y),
            color,
        );
    }
}
