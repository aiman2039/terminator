//! Settings → Editor: native editor font size, line spacing, caret
//! blink, rainbow brackets and the JetBrains keymap. Saved with the appearance in `config.toml`.
use crate::*;
use terminator_core::editor_prefs::{AUTOSAVE_DELAY_MS, Autosave, FONT_SIZE, LINE_SPACING};

impl App {
    pub(super) fn editor_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Editor");
        ui.weak("Native editor text. Apply saves your changes.");
        ui.add_space(8.0);
        let show = [
            self.field_visible("Font size", "editor text points"),
            self.field_visible("Line spacing", "line height leading"),
            self.field_visible("Caret", "cursor blink"),
            self.field_visible("Brackets", "rainbow bracket pair colorization nesting"),
            self.field_visible("Keymap", "jetbrains intellij shortcuts keys"),
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
        if show[3] {
            settings_controls::settings_row(settings_controls::SettingsRowInput {
                ui,
                label: "Brackets",
                description: "Color brackets by nesting depth.",
                add_control: |ui| {
                    ui.checkbox(&mut prefs.editor_rainbow_brackets, "Rainbow brackets");
                },
            });
        }
        if show[4] {
            settings_controls::settings_row(settings_controls::SettingsRowInput {
                ui,
                label: "Keymap",
                description: if cfg!(target_os = "macos") {
                    "While an editor has focus, JetBrains keys win over app keys: ⌘B \
                     declaration, ⌘E recent files, ⌘L go to line, ⌘[ and ⌘] back and \
                     forward, F2 next problem, ⇧F6 rename, ⇧⌘A or double ⇧ search."
                } else {
                    "While an editor has focus, JetBrains keys win over app keys: Ctrl+B \
                     declaration, Ctrl+E recent files, Ctrl+G go to line, F2 next problem, \
                     Shift+F6 rename, Ctrl+Shift+A or double Shift search."
                },
                add_control: |ui| {
                    ui.checkbox(&mut prefs.editor_jetbrains_keys, "JetBrains keys");
                },
            });
        }
        self.editor_save_settings(ui);
    }

    /// Error lens and autosave rows.
    fn editor_save_settings(&mut self, ui: &mut egui::Ui) {
        let show = [
            self.field_visible("Problems", "error lens inline diagnostics"),
            self.field_visible("Autosave", "auto save focus delay"),
        ];
        let prefs = &mut self.theme_draft.editor;
        if show[0] {
            settings_controls::settings_row(settings_controls::SettingsRowInput {
                ui,
                label: "Problems",
                description: "Show each line's worst problem after its text.",
                add_control: |ui| {
                    ui.checkbox(&mut prefs.editor_error_lens, "Inline problems");
                },
            });
        }
        if show[1] {
            settings_controls::settings_row(settings_controls::SettingsRowInput {
                ui,
                label: "Autosave",
                description: "Save native editor files on their own. A file that \
                              changed on disk is never saved automatically.",
                add_control: |ui| {
                    settings_controls::segmented(
                        ui,
                        &mut prefs.editor_autosave,
                        &[
                            ("Off", Autosave::Off),
                            ("On focus loss", Autosave::FocusLoss),
                            ("After delay", Autosave::Delay),
                        ],
                    );
                    if prefs.editor_autosave == Autosave::Delay {
                        ui.add(
                            egui::Slider::new(
                                &mut prefs.editor_autosave_delay_ms,
                                AUTOSAVE_DELAY_MS,
                            )
                            .logarithmic(true)
                            .suffix(" ms"),
                        );
                    }
                },
            });
        }
    }
}
