//! Text tabs for tool-window headers (`JetBrains` Commit | Stash) and the
//! accent-filled primary button of a form footer.
use super::super::scale::WIDGET_RADIUS;
use super::super::theme::contrast_text;
use super::TOOLBAR_BUTTON;
use eframe::egui::{self, Color32};

/// Horizontal text padding inside a [`text_tab`].
const TAB_PADDING: f32 = 8.0;

/// A text tab one toolbar square tall. The selected tab gets a frame in the
/// selection color; any tab fills on hover.
pub fn text_tab(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let galley = egui::WidgetText::from(label).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Button,
    );
    let size = egui::vec2(galley.size().x + 2.0 * TAB_PADDING, TOOLBAR_BUTTON);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.visuals();
        if selected {
            let stroke = egui::Stroke::new(1.0, visuals.selection.stroke.color);
            let fill = visuals.selection.bg_fill.gamma_multiply(0.35);
            ui.painter()
                .rect(rect, WIDGET_RADIUS, fill, stroke, egui::StrokeKind::Inside);
        } else if response.hovered() {
            ui.painter()
                .rect_filled(rect, WIDGET_RADIUS, visuals.widgets.hovered.bg_fill);
        }
        let color = if selected {
            visuals.strong_text_color()
        } else {
            visuals.text_color()
        };
        let at = egui::Align2::CENTER_CENTER
            .anchor_size(rect.center(), galley.size())
            .min;
        ui.painter().galley(at, galley, color);
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, selected, label));
    super::paint_focus(ui, &response);
    response
}

/// The form's main action: filled with `accent`, its text in a contrasting
/// color (Settings Apply, Git Commit).
pub fn primary_button(label: &str, accent: Color32) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(label.to_owned()).color(contrast_text(accent)))
        .fill(accent)
}
