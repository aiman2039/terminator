//! The program status badge on a terminal caption (OSC 7501). Display
//! only: it is separate from the agent status glyph and the Resume chip.
use super::super::scale::{FONT_CAPTION, RADIUS_XSMALL};
use super::usage::KEEP_LEFT;
use eframe::egui::{self, Color32, FontId};

/// Paint `text` as a small tag in `row`, ending at `right` (the caption's
/// controls). `tint` fills it; `ink` is the text. A tag that would cover
/// the title's minimum width is left out. Returns the left edge of the
/// painted tag (`right` when none), so agent usage chips go left of it.
pub fn program_status_badge(
    ui: &egui::Ui,
    (row, right): (egui::Rect, f32),
    text: &str,
    (tint, ink): (Color32, Color32),
) -> f32 {
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), FontId::proportional(FONT_CAPTION), ink);
    let size = galley.size();
    let end = right - 4.0;
    let left = end - (size.x + 8.0);
    if left < row.left() + KEEP_LEFT {
        return right;
    }
    let top = row.center().y - size.y / 2.0 - 1.0;
    let tag = egui::Rect::from_min_max(egui::pos2(left, top), egui::pos2(end, top + size.y + 2.0));
    let painter = ui.painter().with_clip_rect(row);
    painter.rect_filled(tag, RADIUS_XSMALL, tint);
    painter.galley(egui::pos2(tag.left() + 4.0, tag.top() + 1.0), galley, ink);
    left
}

#[cfg(test)]
mod tests {
    use super::super::paint_usage_chips;
    use super::*;

    /// Badge and chips laid out in one caption row, as the pane caption
    /// does: `(badge left, chips left)`.
    fn layout(width: f32) -> (f32, f32) {
        let ctx = egui::Context::default();
        let mut edges = (0.0, 0.0);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let row = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(width, 24.0));
            let colors = (Color32::RED, Color32::WHITE);
            let badge = program_status_badge(ui, (row, row.right()), "brew: working 30%", colors);
            let chips = ["Opus".to_owned(), "Plan".to_owned()];
            edges = (badge, paint_usage_chips(ui, row, badge, &chips));
        });
        output.textures_delta.clear();
        edges
    }

    #[test]
    fn usage_chips_sit_left_of_the_program_badge() {
        let (badge, chips) = layout(600.0);
        assert!(badge < 600.0, "badge painted");
        assert!(chips < badge - 4.0, "chips painted left of the badge");
        assert!(chips >= KEEP_LEFT, "title keeps its minimum width");
    }

    #[test]
    fn a_narrow_caption_keeps_the_title_width() {
        let (badge, chips) = layout(KEEP_LEFT + 12.0);
        assert_eq!(badge, KEEP_LEFT + 12.0, "no room: no badge");
        assert_eq!(chips, badge, "no room: no chips");
    }
}
