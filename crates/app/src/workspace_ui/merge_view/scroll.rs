//! Line numbers and the shared horizontal scroll of the merge columns. The
//! three text columns move together: trackpad, Shift+wheel, or the bar
//! under the body; the caret of the edited result stays in view.
use super::super::super::*;

/// Width of a column's line-number gutter (4 digits).
pub(super) const NUMBERS: f32 = 36.0;
/// Height of the horizontal scroll bar.
const BAR: f32 = 8.0;
/// Room kept between the caret and a column edge.
const CARET_MARGIN: f32 = 24.0;

/// The text part of a column: the column after its line-number gutter.
pub(super) fn text_area(column: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(
            (column.left() + NUMBERS + 4.0).min(column.right()),
            column.top(),
        ),
        column.max,
    )
}

/// Painted line numbers, one per row from the top of `column`.
pub(super) struct Numbers {
    pub(super) first: usize,
    pub(super) count: usize,
    pub(super) row_height: f32,
}

/// Right-aligned 1-based numbers in the gutter, then a 1 px rule.
pub(super) fn paint_numbers(ui: &egui::Ui, column: egui::Rect, numbers: Numbers) {
    let gutter = egui::Rect::from_min_size(
        column.min,
        egui::vec2(NUMBERS.min(column.width()), column.height()),
    );
    let painter = ui
        .painter()
        .with_clip_rect(gutter.intersect(ui.clip_rect()));
    let font = egui::FontId::monospace(appearance::FONT_SMALL);
    let color = ui.visuals().weak_text_color();
    for row in 0..numbers.count {
        let y = gutter.top() + numbers.row_height * (row as f32 + 0.5);
        painter.text(
            egui::pos2(gutter.right() - 2.0, y),
            egui::Align2::RIGHT_CENTER,
            numbers
                .first
                .saturating_add(row)
                .saturating_add(1)
                .to_string(),
            font.clone(),
            color,
        );
    }
    let x = gutter.right() + 1.5;
    ui.painter().vline(
        x,
        column.y_range(),
        egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
    );
}

/// The largest offset that still shows the widest line's end.
pub(super) fn max_offset(widest: f32, view: f32) -> f32 {
    (widest + CARET_MARGIN - view).max(0.0)
}

/// Trackpad and Shift+wheel over the body move the shared offset.
pub(super) fn wheel(ui: &egui::Ui, body: egui::Rect, offset: f32, max: f32) -> f32 {
    let hovered = ui.rect_contains_pointer(body);
    let delta = if hovered {
        ui.input(|input| input.smooth_scroll_delta.x)
    } else {
        0.0
    };
    (offset - delta).clamp(0.0, max)
}

/// The offset that keeps a caret at `caret` (galley x) in a view of
/// `view` points: unchanged when it already shows.
pub(super) fn follow(offset: f32, caret: f32, view: f32) -> f32 {
    if caret < offset + CARET_MARGIN {
        (caret - CARET_MARGIN).max(0.0)
    } else if caret > offset + view - CARET_MARGIN {
        caret - view + CARET_MARGIN
    } else {
        offset
    }
}

/// One bar under the body for the shared offset; returns the new offset.
pub(super) fn bar(ui: &mut egui::Ui, offset: f32, max: f32) -> f32 {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), BAR),
        egui::Sense::click_and_drag(),
    );
    if max <= 0.0 {
        return 0.0;
    }
    let total = rect.width() + max;
    let thumb_width = (rect.width() * rect.width() / total).max(BAR * 3.0);
    let travel = (rect.width() - thumb_width).max(1.0);
    let mut offset = offset;
    if let Some(pointer) = response.interact_pointer_pos() {
        let at = (pointer.x - rect.left() - thumb_width / 2.0).clamp(0.0, travel);
        offset = at / travel * max;
    }
    let left = rect.left() + offset / max * travel;
    let thumb = egui::Rect::from_min_size(
        egui::pos2(left, rect.top()),
        egui::vec2(thumb_width, rect.height()),
    );
    let visuals = ui.visuals();
    let fill = if response.hovered() || response.dragged() {
        visuals.widgets.hovered.bg_fill
    } else {
        visuals.widgets.inactive.bg_fill
    };
    ui.painter()
        .rect_filled(rect, egui::CornerRadius::ZERO, visuals.extreme_bg_color);
    ui.painter()
        .rect_filled(thumb, appearance::RADIUS_SMALL, fill);
    response.on_hover_text("Scroll the three columns sideways (Shift+wheel)");
    offset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caret_past_the_right_edge_scrolls_the_columns() {
        assert_eq!(follow(0.0, 100.0, 300.0), 0.0);
        let moved = follow(0.0, 500.0, 300.0);
        assert!(moved > 0.0 && 500.0 < moved + 300.0);
        assert_eq!(follow(400.0, 10.0, 300.0), 0.0);
    }

    #[test]
    fn short_lines_leave_no_horizontal_range() {
        assert_eq!(max_offset(100.0, 300.0), 0.0);
        assert!(max_offset(1000.0, 300.0) > 700.0);
    }
}
