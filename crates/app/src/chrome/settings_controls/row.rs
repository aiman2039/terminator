//! Settings row layout: a label column, a control column, and the row
//! description under the control.
use crate::*;

/// Widest label column; a narrower card takes room from the label column
/// first so controls such as a segmented row stay on one line.
const LABEL_COL: f32 = 220.0;
const LABEL_COL_MIN: f32 = 150.0;
/// Share of the row width that the label column takes.
const LABEL_COL_SHARE: f32 = 0.28;
/// Space between the label column and the control column.
const LABEL_GAP: f32 = 16.0;
/// Below this width a row stacks its control under the label.
const NARROW_ROW: f32 = 460.0;
/// Space after each row.
const ROW_GAP: f32 = 10.0;
/// Width of a single-line text field in a settings row, so fields in one
/// section share a right edge.
pub const FIELD_WIDTH: f32 = 280.0;

pub struct SettingsRowInput<'a, F: FnOnce(&mut egui::Ui)> {
    pub ui: &'a mut egui::Ui,
    pub label: &'a str,
    pub description: &'a str,
    pub add_control: F,
}

/// One setting: the label in a fixed left column, the control in the
/// column right of it, and the description in small secondary text under
/// the control. A narrow card puts the control under the label.
pub fn settings_row(input: SettingsRowInput<'_, impl FnOnce(&mut egui::Ui)>) {
    let SettingsRowInput {
        ui,
        label,
        description,
        add_control,
    } = input;
    let control = |ui: &mut egui::Ui| {
        add_control(ui);
        if !description.is_empty() {
            description_line(ui, description);
        }
    };
    if ui.available_width() < NARROW_ROW {
        ui.label(label);
        control(ui);
    } else {
        let label_col = label_column_width(ui.available_width());
        ui.horizontal_top(|ui| {
            let line = egui::vec2(label_col, ui.spacing().interact_size.y);
            ui.allocate_ui_with_layout(
                line,
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_width(label_col);
                    ui.add(egui::Label::new(label).truncate());
                },
            );
            ui.add_space(LABEL_GAP);
            ui.vertical(|ui| {
                ui.set_min_width(ui.available_width().clamp(180.0, 360.0));
                control(ui);
            });
        });
    }
    ui.add_space(ROW_GAP);
}

/// Content that belongs to the rows above it but has no label of its own
/// (a caption, a status line): indented to the control column.
pub fn control_column<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    if ui.available_width() < NARROW_ROW {
        return add(ui);
    }
    let label_col = label_column_width(ui.available_width());
    ui.horizontal_top(|ui| {
        ui.add_space(label_col + LABEL_GAP + ui.spacing().item_spacing.x);
        ui.vertical(add).inner
    })
    .inner
}

fn label_column_width(row_width: f32) -> f32 {
    (row_width * LABEL_COL_SHARE).clamp(LABEL_COL_MIN, LABEL_COL)
}

/// Width of a button or selectable label that shows `text` on one line.
pub fn button_width(ui: &egui::Ui, text: &str) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), font, egui::Color32::PLACEHOLDER);
    galley.size().x + 2.0 * ui.spacing().button_padding.x
}

/// A row description: small secondary text that wraps at the column edge.
pub fn description_line(ui: &mut egui::Ui, text: &str) {
    ui.add(
        egui::Label::new(
            egui::RichText::new(text)
                .size(appearance::FONT_SMALL)
                .color(ui.visuals().weak_text_color()),
        )
        .wrap(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROW_WIDTH: f32 = 720.0;

    fn paint_rows(ctx: &egui::Context) -> (f32, f32) {
        let mut xs = (0.0, 0.0);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(ROW_WIDTH, 400.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            settings_row(SettingsRowInput {
                ui,
                label: "Days",
                description: "short",
                add_control: |ui| {
                    xs.0 = ui.next_widget_position().x;
                    ui.label("control-a");
                },
            });
            settings_row(SettingsRowInput {
                ui,
                label: "A much longer label",
                description: "a longer description that should wrap without shifting the control column",
                add_control: |ui| {
                    xs.1 = ui.next_widget_position().x;
                    ui.label("control-b");
                },
            });
        });
        output.textures_delta.clear();
        xs
    }

    #[test]
    fn settings_controls_share_a_left_edge() {
        let ctx = egui::Context::default();
        let _ = paint_rows(&ctx);
        let (first, second) = paint_rows(&ctx);
        assert!(
            (first - second).abs() < 1.0,
            "first={first} second={second}"
        );
        assert!(
            first > label_column_width(ROW_WIDTH),
            "control column starts after the label"
        );
    }

    #[test]
    fn label_column_narrows_with_the_row_within_bounds() {
        assert_eq!(label_column_width(2000.0), LABEL_COL);
        assert_eq!(label_column_width(NARROW_ROW), LABEL_COL_MIN);
        assert!(label_column_width(ROW_WIDTH) < LABEL_COL);
    }
}
