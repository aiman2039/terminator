//! Settings → Editor: native editor font size, line spacing and caret
//! blink. Saved with the appearance in `config.toml`.
use crate::*;
use terminator_core::editor_prefs::{FONT_SIZE, LINE_SPACING};

impl App {
    pub(super) fn editor_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Editor");
        ui.weak("Native editor text. Apply saves your changes.");
        ui.add_space(8.0);
        let show = [
            self.field_visible("Font size", "editor text points"),
            self.field_visible("Line spacing", "line height leading"),
            self.field_visible("Caret", "cursor blink"),
        ];
        let prefs = &mut self.theme_draft.editor;
        if show[0] {
            settings_controls::settings_row(settings_controls::SettingsRowInput {
                ui,
                label: "Font size",
                description: "Code font size in points. Markdown is one point larger.",
                add_control: |ui| {
                    ui.add(
                        egui::Slider::new(&mut prefs.editor_font_size, FONT_SIZE)
                            .step_by(1.0)
                            .suffix(" pt"),
                    );
                },
            });
        }
        if show[1] {
            settings_controls::settings_row(settings_controls::SettingsRowInput {
                ui,
                label: "Line spacing",
                description: "Row height as a multiple of the font height.",
                add_control: |ui| {
                    ui.add(
                        egui::Slider::new(&mut prefs.editor_line_spacing, LINE_SPACING)
                            .step_by(0.05)
                            .fixed_decimals(2),
                    );
                },
            });
        }
        if show[2] {
            settings_controls::settings_row(settings_controls::SettingsRowInput {
                ui,
                label: "Caret",
                description: "Blink the caret while the editor has focus.",
                add_control: |ui| {
                    ui.checkbox(&mut prefs.editor_caret_blink, "Blink caret");
                },
            });
        }
    }
}
