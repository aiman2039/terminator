//! Form controls that read at rest: a checkbox or text field outline, and a
//! slider rail, in `control_line` (the first mix of `surface` toward `text`
//! with 3:1 contrast on `surface`). Checkboxes and text fields get it from
//! egui widget style providers, so framed buttons keep `border`; sliders
//! use [`slider`]. Hover and focus keep egui's own look.
use super::super::theme::{contrast_ratio, control_line};
use eframe::egui::{self, Color32};
use egui::theme::{DefaultStyle, StyleProvider};
use egui::widget_style::{CheckboxStyle, StyleArgs, TextEditStyle, WidgetState};

/// Lowest contrast of a control outline on `surface` (WCAG 1.4.11).
const MIN_CONTRAST: f32 = 3.0;

/// The first 4 % step from `surface` toward `text` that reaches 3:1 on
/// `surface`; `text` when none does.
pub(in crate::appearance) fn control_line_for(surface: Color32, text: Color32) -> Color32 {
    (1..=25u8)
        .map(|step| surface.lerp_to_gamma(text, f32::from(step) * 0.04))
        .find(|line| contrast_ratio(*line, surface) >= MIN_CONTRAST)
        .unwrap_or(text)
}

struct RestOutline;

fn at_rest(stroke: egui::Stroke) -> egui::Stroke {
    egui::Stroke::new(stroke.width, control_line())
}

impl StyleProvider<CheckboxStyle> for RestOutline {
    fn style(&mut self, args: &StyleArgs<'_>) -> CheckboxStyle {
        let mut style: CheckboxStyle = DefaultStyle.style(args);
        if args.state == WidgetState::Inactive {
            style.checkbox_frame.stroke = at_rest(style.checkbox_frame.stroke);
        }
        style
    }
}

impl StyleProvider<TextEditStyle> for RestOutline {
    fn style(&mut self, args: &StyleArgs<'_>) -> TextEditStyle {
        let mut style: TextEditStyle = DefaultStyle.style(args);
        if args.state == WidgetState::Inactive && !args.has_class(&egui::TextEdit::CLASS_READ_ONLY)
        {
            let frame = &mut style.atom_layout.frame;
            frame.stroke = at_rest(frame.stroke);
        }
        style
    }
}

/// Registers the providers; [`super::super::install`] calls it once.
pub(in crate::appearance) fn install_rest_outlines(ctx: &egui::Context) {
    ctx.add_widget_theme::<CheckboxStyle>(RestOutline);
    ctx.add_widget_theme::<TextEditStyle>(RestOutline);
}

/// A slider whose rail shows on the surface: egui fills the rail (and the
/// handle) with the inactive widget fill, which is the surface itself.
pub fn slider(ui: &mut egui::Ui, slider: egui::Slider<'_>) -> egui::Response {
    ui.scope(|ui| {
        ui.visuals_mut().widgets.inactive.bg_fill = control_line();
        ui.add(slider)
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rest_outline_reaches_three_to_one_on_the_surface() {
        // Carbon: surface #1A191B, text #EEEDEB.
        let surface = Color32::from_rgb(0x1A, 0x19, 0x1B);
        let line = control_line_for(surface, Color32::from_rgb(0xEE, 0xED, 0xEB));
        assert!(contrast_ratio(line, surface) >= MIN_CONTRAST);
        // A light theme too.
        let light = Color32::from_rgb(0xF7, 0xF8, 0xFA);
        let line = control_line_for(light, Color32::from_rgb(0x1F, 0x23, 0x28));
        assert!(contrast_ratio(line, light) >= MIN_CONTRAST);
    }

    /// An unchecked checkbox at rest is outlined in `control_line`; a framed
    /// button keeps the theme border.
    #[test]
    fn checkboxes_and_fields_get_the_rest_outline_but_buttons_do_not() {
        let ctx = egui::Context::default();
        crate::appearance::install(&ctx);
        let controls = |ui: &mut egui::Ui| {
            ui.checkbox(&mut false, "Option");
            ui.text_edit_singleline(&mut String::new());
            let _ = ui.button("Run");
        };
        // The second frame has laid out every widget.
        ctx.run_ui(egui::RawInput::default(), controls)
            .textures_delta
            .clear();
        let mut output = ctx.run_ui(egui::RawInput::default(), controls);
        output.textures_delta.clear();
        let strokes: Vec<Color32> = output
            .shapes
            .into_iter()
            .filter_map(|shape| match shape.shape {
                egui::epaint::Shape::Rect(rect) => Some(rect.stroke.color),
                _ => None,
            })
            .collect();
        // `control_line` is one value for the process, and a theme test on
        // another thread can change it between frames. So compare the painted
        // strokes with each other, not with a value read before the frame.
        let painted: Vec<Color32> = strokes
            .iter()
            .copied()
            .filter(|color| *color != Color32::TRANSPARENT)
            .collect();
        let [checkbox, field, button] = painted[..] else {
            panic!("checkbox, text field and button: {strokes:?}");
        };
        assert_eq!(checkbox, field, "one rest outline: {strokes:?}");
        assert_ne!(checkbox, button, "a button keeps the border: {strokes:?}");
    }
}
