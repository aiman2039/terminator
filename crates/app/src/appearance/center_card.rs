//! The inset card for center-pane pages (Settings, SSH, VNC, worktree):
//! a margin on each side keeps the window visible and clickable, a click
//! on that margin dismisses the page, and the card scrolls inside.
use super::{PANEL_RADIUS, color};
use eframe::egui;
use terminator_core::appearance::AppearanceConfig;

/// Widest a center card grows on a wide window.
pub const CENTER_CARD_MAX_WIDTH: f32 = 1100.0;
/// Inner padding of a center card.
pub const CENTER_CARD_PADDING: i8 = 14;

/// Card rect inside `outer`: a margin on each side and a capped width.
pub fn center_card_rect(outer: egui::Rect) -> egui::Rect {
    let margin_x = (outer.width() * 0.06).clamp(12.0, 64.0);
    let margin_y = (outer.height() * 0.06).clamp(12.0, 48.0);
    let width = (outer.width() - 2.0 * margin_x).clamp(0.0, CENTER_CARD_MAX_WIDTH);
    let height = (outer.height() - 2.0 * margin_y).max(0.0);
    egui::Rect::from_center_size(outer.center(), egui::vec2(width, height))
}

/// Theme colors a center card paints with; copied out of the theme so the
/// card body can borrow the app mutably.
#[derive(Clone, Copy)]
pub struct CenterCardStyle {
    window: egui::Color32,
    surface: egui::Color32,
    border: egui::Stroke,
}

impl CenterCardStyle {
    pub fn new(theme: &AppearanceConfig) -> Self {
        Self {
            window: color(&theme.window),
            surface: color(&theme.surface),
            border: egui::Stroke::new(theme.border_width.max(1.0), color(&theme.border)),
        }
    }

    /// The card's frame: surface fill, border, panel radius and padding.
    pub fn frame(self) -> egui::Frame {
        egui::Frame::new()
            .fill(self.surface)
            .stroke(self.border)
            .corner_radius(PANEL_RADIUS)
            .inner_margin(egui::Margin::same(CENTER_CARD_PADDING))
    }

    /// This style with a dim backdrop in place of the window color, for a
    /// card over panes that stay visible behind it (pane zoom).
    pub fn scrim(self) -> Self {
        Self {
            window: self.window.gamma_multiply(0.7),
            ..self
        }
    }
}

/// Paints the window background and the card, runs `add` in the card, and
/// returns its result with whether the margin was clicked this frame.
pub fn center_card<R>(
    ui: &mut egui::Ui,
    style: CenterCardStyle,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> (R, bool) {
    let outer = ui.available_rect_before_wrap();
    center_card_over(ui, outer, style, add)
}

/// [`center_card`] over `outer`: a page paints the window color there, a
/// card over visible panes uses [`CenterCardStyle::scrim`].
pub fn center_card_over<R>(
    ui: &mut egui::Ui,
    outer: egui::Rect,
    style: CenterCardStyle,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> (R, bool) {
    let card = center_card_rect(outer);
    ui.painter()
        .rect_filled(outer, egui::CornerRadius::ZERO, style.window);
    let mut card_ui = ui.new_child(egui::UiBuilder::new().max_rect(card));
    // Wide content must not grow the card past the pane: that hides its
    // right edge and the scroll bars of the page inside it.
    card_ui.shrink_clip_rect(card);
    let inner = style
        .frame()
        .show(&mut card_ui, |ui| {
            let size = card.shrink(f32::from(CENTER_CARD_PADDING)).size();
            ui.set_max_size(size);
            ui.shrink_clip_rect(egui::Rect::from_min_size(ui.max_rect().min, size));
            add(ui)
        })
        .inner;
    ui.allocate_rect(outer, egui::Sense::hover());
    (inner, margin_clicked(ui, outer, card))
}

/// A one-row, right-aligned button strip for a form footer. A plain
/// `with_layout(right_to_left)` in a vertical form takes all remaining
/// height and pushes the buttons to the bottom of the pane.
pub fn footer_row<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let row = egui::vec2(ui.available_width(), ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), add)
        .inner
}

/// A primary click this frame in `outer` but outside `card`, on this
/// layer (so a popup or dialog above it does not count).
fn margin_clicked(ui: &egui::Ui, outer: egui::Rect, card: egui::Rect) -> bool {
    let Some(pos) = ui.input(|input| {
        input
            .pointer
            .primary_clicked()
            .then(|| input.pointer.interact_pos())
            .flatten()
    }) else {
        return false;
    };
    outer.contains(pos) && !card.contains(pos) && ui.ctx().layer_id_at(pos) == Some(ui.layer_id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footer_row_takes_one_row_not_the_remaining_height() {
        let ctx = egui::Context::default();
        let mut height = 0.0;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let before = ui.cursor().min.y;
            footer_row(ui, |ui| ui.button("Save"));
            height = ui.cursor().min.y - before;
        });
        output.textures_delta.clear();
        assert!(height > 0.0 && height < 60.0, "footer height {height}");
    }

    #[test]
    fn wide_content_is_clipped_to_the_card() {
        let ctx = egui::Context::default();
        let outer = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0));
        let card = center_card_rect(outer);
        let mut clip = egui::Rect::NOTHING;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let style = CenterCardStyle {
                window: egui::Color32::BLACK,
                surface: egui::Color32::BLACK,
                border: egui::Stroke::NONE,
            };
            center_card_over(ui, outer, style, |ui| {
                ui.allocate_space(egui::vec2(2000.0, 20.0));
                clip = ui.clip_rect();
            });
        });
        output.textures_delta.clear();
        assert!(card.contains_rect(clip), "clip {clip:?} card {card:?}");
    }

    #[test]
    fn center_card_leaves_a_margin_on_every_side() {
        let outer = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 900.0));
        let card = center_card_rect(outer);
        assert!(card.min.x > outer.min.x && card.max.x < outer.max.x);
        assert!(card.min.y > outer.min.y && card.max.y < outer.max.y);
    }
}
