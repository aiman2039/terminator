//! The mark of a pane that takes part in broadcast input: a caption chip
//! and an outline around the terminal, both in one accent color.
use super::super::scale::{FONT_CAPTION, RADIUS_SMALL};
use eframe::egui::{self, Color32};

const CHIP_HEIGHT: f32 = 16.0;
const CHIP_TEXT: &str = "BROADCAST";

/// Paints the chip right-aligned, ending at `right`, vertically centred in
/// `row`. Returns its left edge (`right` when it does not fit).
pub fn paint_broadcast_chip(ui: &egui::Ui, row: egui::Rect, right: f32, accent: Color32) -> f32 {
    let ink = if accent.intensity() > 0.5 {
        Color32::BLACK
    } else {
        Color32::WHITE
    };
    let galley = ui.painter().layout_no_wrap(
        CHIP_TEXT.to_owned(),
        egui::FontId::proportional(FONT_CAPTION),
        ink,
    );
    let width = galley.size().x + 8.0;
    let left = right - width - 4.0;
    if left < row.left() {
        return right;
    }
    let rect = egui::Rect::from_min_size(
        egui::pos2(left, row.center().y - CHIP_HEIGHT * 0.5),
        egui::vec2(width, CHIP_HEIGHT),
    );
    ui.painter().rect_filled(rect, RADIUS_SMALL, accent);
    ui.painter().galley(
        egui::pos2(rect.left() + 4.0, rect.center().y - galley.size().y * 0.5),
        galley,
        ink,
    );
    ui.interact(rect, ui.scope_id().with("broadcast-chip"), egui::Sense::hover())
        .on_hover_text("Broadcast input: keys typed here go to every terminal in this tab. Press Esc twice to stop.");
    left
}

/// A 2 px outline inside the terminal's rect.
pub fn paint_broadcast_outline(painter: &egui::Painter, pane: egui::Rect, accent: Color32) {
    painter.rect_stroke(
        pane,
        egui::CornerRadius::ZERO,
        egui::Stroke::new(2.0, accent),
        egui::StrokeKind::Inside,
    );
}
