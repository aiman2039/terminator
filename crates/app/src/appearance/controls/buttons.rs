//! Button roles: one filled main action ([`primary_button`]), its outlined
//! companion ([`secondary_button`]), one outlined destructive action
//! ([`danger_button`]), and a text [`link`]. Every other button keeps the
//! default paint.
use super::super::theme::{control_line, filled_button_colors, primary_fill};
use eframe::egui;

/// The main action of a form or dialog (Commit, Settings Apply, Connect,
/// a dialog's default). Fill from the theme's `primary` (default: the
/// accent); the label is dark or white, whichever reads better. Disabled, it
/// has no fill: an outline in `control_line` and a secondary label, so it
/// reads as unavailable next to a filled one and never looks like an error.
pub fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let (fill, label, stroke) = primary_paint(ui);
    let response = ui.add(
        egui::Button::new(egui::RichText::new(text).strong().color(label))
            .fill(fill)
            .stroke(stroke),
    );
    super::paint_focus(ui, &response);
    response
}

/// Fill, label and outline of [`primary_button`] in this `ui`'s state.
fn primary_paint(ui: &egui::Ui) -> (egui::Color32, egui::Color32, egui::Stroke) {
    if ui.is_enabled() {
        let (fill, label) = filled_button_colors(primary_fill());
        (fill, label, egui::Stroke::NONE)
    } else {
        (
            egui::Color32::TRANSPARENT,
            ui.visuals().weak_text_color(),
            egui::Stroke::new(1.0, control_line()),
        )
    }
}

/// The action beside a [`primary_button`] (Commit and Push beside Commit):
/// no fill, an outline in `control_line`, and the body label.
pub fn secondary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let response = ui.add(
        egui::Button::new(text)
            .fill(egui::Color32::TRANSPARENT)
            .stroke(egui::Stroke::new(1.0, control_line())),
    );
    super::paint_focus(ui, &response);
    response
}

/// A text action that reads as a link: the theme's link color with an
/// underline. `ui.link` alone paints body text, because the app sets
/// `override_text_color`.
pub fn link(ui: &mut egui::Ui, text: egui::RichText) -> egui::Response {
    let color = ui.visuals().hyperlink_color;
    ui.link(text.color(color).underline())
}

/// An action that stops, removes or discards something: default fill, with
/// outline and label in the failure color (the menu danger paint).
pub fn danger_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let failed = ui.visuals().error_fg_color;
    let response = ui.add(
        egui::Button::new(egui::RichText::new(text).color(failed))
            .stroke(egui::Stroke::new(1.0, failed)),
    );
    super::paint_focus(ui, &response);
    response
}

#[cfg(test)]
mod tests {
    use super::super::super::theme::contrast_ratio;
    use super::*;
    use eframe::egui::Color32;

    #[test]
    fn primary_label_reaches_four_and_a_half_to_one_on_every_preset() {
        use terminator_core::appearance::AppearanceConfig;
        for theme in [
            AppearanceConfig::carbon(),
            AppearanceConfig::islands_dark(),
            AppearanceConfig::intellij_dark(),
            AppearanceConfig::high_contrast(),
            AppearanceConfig::terminator_dark(),
            AppearanceConfig::intellij_light(),
        ] {
            let (fill, label) =
                filled_button_colors(super::super::super::theme::color(theme.primary_color()));
            assert!(contrast_ratio(fill, label) >= 4.5, "{theme:?}");
        }
    }

    #[test]
    fn carbon_main_action_is_neutral_with_a_dark_label() {
        let theme = terminator_core::appearance::AppearanceConfig::carbon();
        let (fill, label) =
            filled_button_colors(super::super::super::theme::color(theme.primary_color()));
        assert_eq!(fill, Color32::from_rgb(0xEE, 0xED, 0xEB));
        assert_eq!(label, Color32::from_rgb(0x14, 0x14, 0x16));
    }

    /// Enabled, the main action is filled; disabled, it loses the fill and
    /// keeps only an outline, so it reads as unavailable.
    #[test]
    fn a_disabled_main_action_is_outlined_not_filled() {
        let ctx = egui::Context::default();
        crate::appearance::install(&ctx);
        let mut paints = Vec::new();
        ctx.run_ui(egui::RawInput::default(), |ui| {
            paints.push(primary_paint(ui));
            ui.add_enabled_ui(false, |ui| paints.push(primary_paint(ui)));
        })
        .textures_delta
        .clear();
        let [(fill, _, stroke), (off_fill, off_label, off_stroke)] = paints[..] else {
            panic!("two paints: {paints:?}");
        };
        assert_ne!(fill, Color32::TRANSPARENT);
        assert_eq!(stroke, egui::Stroke::NONE);
        assert_eq!(off_fill, Color32::TRANSPARENT);
        assert_ne!(off_stroke.color, Color32::TRANSPARENT);
        assert_ne!(off_label, Color32::TRANSPARENT);
    }

    /// The app sets `override_text_color`, which `ui.link` alone would
    /// paint: a link keeps the hyperlink color.
    #[test]
    fn a_link_paints_in_the_link_color_not_body_text() {
        let ctx = egui::Context::default();
        crate::appearance::install(&ctx);
        let mut link_color = Color32::TRANSPARENT;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            link_color = ui.visuals().hyperlink_color;
            let _ = link(ui, egui::RichText::new("Stage all (3)"));
        });
        output.textures_delta.clear();
        let colors: Vec<Color32> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => text
                    .galley
                    .job
                    .sections
                    .first()
                    .map(|section| section.format.color),
                _ => None,
            })
            .collect();
        assert_eq!(colors, vec![link_color]);
    }

    #[test]
    fn a_blue_fill_darkens_until_white_reads() {
        let (fill, label) = filled_button_colors(Color32::from_rgb(0x35, 0x74, 0xF0));
        assert_eq!(label, Color32::WHITE);
        assert!(contrast_ratio(fill, label) >= 4.5);
        assert!(fill.r() < 0x35 || fill.g() < 0x74);
    }
}
