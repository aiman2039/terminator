//! Button roles: one filled main action ([`primary_button`]) and one
//! outlined destructive action ([`danger_button`]). Every other button keeps
//! the default paint.
use super::super::theme::{filled_button_colors, primary_fill};
use eframe::egui;

/// The main action of a form or dialog (Commit, Settings Apply, Connect,
/// a dialog's default). Fill from the theme's `primary` (default: the
/// accent); the label is dark or white, whichever reads better. Disabled, it
/// keeps full opacity with a hover fill and secondary label, so a disabled
/// main action never looks like an error.
pub fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let enabled = ui.is_enabled();
    let (fill, label) = if enabled {
        filled_button_colors(primary_fill())
    } else {
        (
            ui.visuals().widgets.hovered.bg_fill,
            ui.visuals().weak_text_color(),
        )
    };
    let response = ui
        .scope(|ui| {
            if !enabled {
                // Undo the disabled fade only for this button.
                let alpha = ui.visuals().disabled_alpha().max(f32::EPSILON);
                ui.set_opacity((ui.opacity() / alpha).min(1.0));
            }
            ui.add(
                egui::Button::new(egui::RichText::new(text).strong().color(label))
                    .fill(fill)
                    .stroke(egui::Stroke::NONE),
            )
        })
        .inner;
    super::paint_focus(ui, &response);
    response
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

    #[test]
    fn a_blue_fill_darkens_until_white_reads() {
        let (fill, label) = filled_button_colors(Color32::from_rgb(0x35, 0x74, 0xF0));
        assert_eq!(label, Color32::WHITE);
        assert!(contrast_ratio(fill, label) >= 4.5);
        assert!(fill.r() < 0x35 || fill.g() < 0x74);
    }
}
