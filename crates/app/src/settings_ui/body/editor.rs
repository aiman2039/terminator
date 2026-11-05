//! Settings → Editor: native editor font size, line spacing, caret
//! blink, rainbow brackets and the JetBrains keymap. Saved with the appearance in `config.toml`.
use crate::*;
use terminator_core::editor_prefs::{AUTOSAVE_DELAY_MS, Autosave, FONT_SIZE, LINE_SPACING};

/// What the JetBrains keymap does, in this platform's keys.
const KEYMAP_HELP: &str = if cfg!(target_os = "macos") {
    "Works in the whole app: double ⇧ search everywhere, ⇧⌘A find action, ⇧⌘O go \
     to file. While an editor has focus, these keys also win over app keys: ⌘B \
     declaration, ⌘E recent files, ⌘L go to line, ⌘[ and ⌘] back and forward, F2 \
     next problem, ⇧F6 rename."
} else {
    "Works in the whole app: double Shift search everywhere, Ctrl+Shift+A find \
     action, Ctrl+Shift+N go to file, Ctrl+E recent files. A focused terminal keeps \
     Ctrl+E and Ctrl+Shift+A. While an editor has focus, these keys also win over \
     app keys: Ctrl+B declaration, Ctrl+G go to line, F2 next problem, Shift+F6 \
     rename."
};

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
        self.editor_keymap_setting(ui);
        self.editor_save_settings(ui);
    }

    /// The JetBrains keymap row. The keymap works in the whole app.
    fn editor_keymap_setting(&mut self, ui: &mut egui::Ui) {
        let keywords = "jetbrains intellij shortcuts keys double shift find action go to file";
        if !self.field_visible("Keymap", keywords) {
            return;
        }
        let prefs = &mut self.theme_draft.editor;
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Keymap",
            description: KEYMAP_HELP,
            add_control: |ui| {
                ui.checkbox(
                    &mut prefs.editor_jetbrains_keys,
                    "JetBrains keys in the whole app",
                );
            },
        });
    }

    /// Error lens, autosave and hover documentation rows.
    fn editor_save_settings(&mut self, ui: &mut egui::Ui) {
        let show = [
            self.field_visible("Problems", "error lens inline diagnostics"),
            self.field_visible("Autosave", "auto save focus delay"),
            self.field_visible(
                "Documentation",
                "quick documentation hover mouse pointer docs",
            ),
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
        if show[2] {
            settings_controls::settings_row(settings_controls::SettingsRowInput {
                ui,
                label: "Documentation",
                description: "Show the docs of the symbol under a resting pointer. Problems \
                              there show either way. F1 shows the docs at the caret.",
                add_control: |ui| {
                    ui.checkbox(
                        &mut prefs.editor_hover_docs,
                        "Show documentation on mouse hover",
                    );
                },
            });
        }
    }
}
