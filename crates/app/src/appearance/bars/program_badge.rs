//! The program status badge on a terminal caption (OSC 7501). Display
//! only: it is separate from the agent status glyph and the Resume chip.
use super::super::scale::{FONT_CAPTION, RADIUS_XSMALL};
use super::TerminalBar;
use eframe::egui::{self, Color32, FontId};

/// Paint `text` as a small tag at the right end of the caption's title
/// area, before the branch and buttons. `tint` fills it; `ink` is the text.
pub fn program_status_badge(
    ui: &egui::Ui,
    bar: &TerminalBar,
    text: &str,
    (tint, ink): (Color32, Color32),
) {
    let rect = bar.bar.rect;
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), FontId::proportional(FONT_CAPTION), ink);
    let size = galley.size();
    let width = size.x + 8.0;
    let right = bar.controls_left - 4.0;
    let left = (right - width).max(rect.left());
    let top = rect.center().y - size.y / 2.0 - 1.0;
    let tag =
        egui::Rect::from_min_max(egui::pos2(left, top), egui::pos2(right, top + size.y + 2.0));
    if tag.width() < 12.0 {
        return;
    }
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(tag, RADIUS_XSMALL, tint);
    painter.galley(egui::pos2(tag.left() + 4.0, tag.top() + 1.0), galley, ink);
}
