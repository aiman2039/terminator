//! Agent usage chips (model, permission mode) on a pane caption or an
//! inbox card. The text comes only from what the agent reports in its
//! hooks (`agent-usage-v1`); it is display only.
use super::super::scale::{FONT_CAPTION, RADIUS_SMALL};
use eframe::egui;

/// Height of one chip.
const CHIP_HEIGHT: f32 = 16.0;
/// Space kept for the caption title left of the chips and the program
/// status badge.
pub(super) const KEEP_LEFT: f32 = 96.0;

/// Paints `chips` right-aligned, ending at `right`, vertically centred in
/// `row`. Chips that do not fit are left out. Returns the left edge of
/// the painted chips (`right` when none fit).
pub fn paint_usage_chips(ui: &egui::Ui, row: egui::Rect, right: f32, chips: &[String]) -> f32 {
    let mut left = right;
    let font = egui::FontId::proportional(FONT_CAPTION);
    let text_color = ui.visuals().weak_text_color();
    let fill = ui.visuals().widgets.inactive.bg_fill;
    for chip in chips.iter().rev() {
        let galley = ui
            .painter()
            .layout_no_wrap(chip.clone(), font.clone(), text_color);
        let width = galley.size().x + 8.0;
        let next = left - width - 4.0;
        if next < row.left() + KEEP_LEFT {
            break;
        }
        let rect = egui::Rect::from_min_size(
            egui::pos2(next, row.center().y - CHIP_HEIGHT * 0.5),
            egui::vec2(width, CHIP_HEIGHT),
        );
        ui.painter().rect_filled(rect, RADIUS_SMALL, fill);
        ui.painter().galley(
            egui::pos2(rect.left() + 4.0, rect.center().y - galley.size().y * 0.5),
            galley,
            text_color,
        );
        ui.interact(
            rect,
            ui.scope_id().with(("usage-chip", chip)),
            egui::Sense::hover(),
        )
        .on_hover_text("Reported by the agent's hooks");
        left = next;
    }
    left
}
