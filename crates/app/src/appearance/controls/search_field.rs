//! A button drawn like a search field: the window header's Search
//! everywhere entry. It opens a search; it is not a text input.
use super::{HEADER_BUTTON, PaintCenteredIconInput, TOOLBAR_ICON, accessible_icon};
use crate::appearance::scale::{FONT_CAPTION, FONT_SMALL, WIDGET_RADIUS};
use crate::appearance::theme::control_line;
use eframe::egui::{self, FontId};

/// Inner padding at each end of the field.
const PAD: f32 = 8.0;
/// Space between the glyph, the label, and the key hint.
const GAP: f32 = 6.0;

pub struct SearchFieldButtonInput<'a> {
    pub ui: &'a mut egui::Ui,
    /// Muted text after the glyph, such as "Search everywhere".
    pub label: &'a str,
    /// The bound key, right-aligned; empty shows no hint.
    pub keys: &'a str,
    pub tip: &'a str,
    pub width: f32,
}

fn label_font() -> FontId {
    FontId::proportional(FONT_SMALL)
}

fn keys_font() -> FontId {
    FontId::proportional(FONT_CAPTION)
}

fn text_width(ui: &egui::Ui, text: &str, font: FontId) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    ui.painter()
        .layout_no_wrap(text.to_owned(), font, egui::Color32::PLACEHOLDER)
        .size()
        .x
}

/// The width that shows the glyph, the label, and the key hint in full.
pub fn search_field_width(ui: &egui::Ui, label: &str, keys: &str) -> f32 {
    let keys = text_width(ui, keys, keys_font());
    let hint = if keys > 0.0 { GAP * 2.0 + keys } else { 0.0 };
    PAD + TOOLBAR_ICON + GAP + text_width(ui, label, label_font()) + hint + PAD
}

/// One [`HEADER_BUTTON`] high: field fill and outline, the Search glyph,
/// the muted label, and the key hint at the right end.
pub fn search_field_button(input: SearchFieldButtonInput<'_>) -> egui::Response {
    let SearchFieldButtonInput {
        ui,
        label,
        keys,
        tip,
        width,
    } = input;
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, HEADER_BUTTON), egui::Sense::click());
    let response = response.on_hover_text(tip);
    let visuals = ui.visuals();
    let fill = if response.hovered() {
        visuals.widgets.hovered.bg_fill
    } else {
        visuals.extreme_bg_color
    };
    let weak = visuals.weak_text_color();
    let painter = ui.painter();
    painter.rect(
        rect,
        WIDGET_RADIUS,
        fill,
        egui::Stroke::new(1.0, control_line()),
        egui::StrokeKind::Inside,
    );
    let icon = egui::Rect::from_min_size(
        egui::pos2(rect.left() + PAD, rect.top()),
        egui::vec2(TOOLBAR_ICON, rect.height()),
    );
    super::paint_centered_icon(PaintCenteredIconInput {
        ui,
        rect: icon,
        icon: "Search",
        size: TOOLBAR_ICON,
        tint: weak,
    });
    let painter = ui.painter();
    if !keys.is_empty() {
        painter.text(
            egui::pos2(rect.right() - PAD, rect.center().y),
            egui::Align2::RIGHT_CENTER,
            keys,
            keys_font(),
            weak,
        );
    }
    painter.text(
        egui::pos2(icon.right() + GAP, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        label_font(),
        weak,
    );
    accessible_icon(ui, &response, label);
    response
}
