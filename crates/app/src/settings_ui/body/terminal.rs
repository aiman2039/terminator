use super::super::state::BrowseTarget;
use crate::settings_controls::BinaryOption;
use crate::*;

impl App {
    /// Settings → Terminal: the shell and terminal text.
    pub(super) fn terminal_settings(
        &mut self,
        ui: &mut egui::Ui,
        grant_folder: &mut bool,
        browse: &mut Option<BrowseTarget>,
    ) {
        ui.heading("Terminal");
        let (shells, editors) = self.settings_binary_options();
        self.detect_custom_binaries(&shells, &editors);
        self.shell_override_row(ui, &shells, browse);
        self.font_size_row(ui);
        self.type_ahead_row(ui);
        self.folder_access_row(ui, grant_folder);
    }

    /// The Mode group of Settings → Editor: which editor opens files.
    pub(super) fn editor_mode_settings(
        &mut self,
        ui: &mut egui::Ui,
        browse: &mut Option<BrowseTarget>,
    ) {
        let (shells, editors) = self.settings_binary_options();
        self.detect_custom_binaries(&shells, &editors);
        self.editor_mode_row(ui);
        self.editor_executable_row(ui, &editors, browse);
        self.external_editor_row(ui);
        self.external_custom_row(ui, browse);
        self.test_draft_row(ui);
    }

    /// Settings → Git & Diff.
    pub(super) fn git_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Git & Diff");
        self.diff_viewer_row(ui);
        self.diff_close_settings(ui);
        self.pr_metadata_row(ui);
    }

    fn detect_custom_binaries(&mut self, shells: &[BinaryOption], editors: &[BinaryOption]) {
        self.custom_shell |= shells
            .iter()
            .all(|option| option.value != self.settings_draft.shell);
        self.custom_editor |= editors
            .iter()
            .all(|option| option.value != self.settings_draft.editor_program);
    }

    fn shell_override_row(
        &mut self,
        ui: &mut egui::Ui,
        shells: &[BinaryOption],
        browse: &mut Option<BrowseTarget>,
    ) {
        if !self.field_visible("Shell override", "zsh bash fish sh") {
            return;
        }
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Shell override",
            description: "Used for new terminals. Leave Automatic unless you need a specific binary.",
            add_control: |ui| {
                settings_controls::combo_or_custom(settings_controls::ComboOrCustomInput {
                    ui,
                    id: "shell-override",
                    value: &mut self.settings_draft.shell,
                    options: shells,
                    custom: &mut self.custom_shell,
                });
                let mut pick = false;
                if self.custom_shell {
                    settings_controls::path_field(settings_controls::PathFieldInput {
                        ui,
                        value: &mut self.settings_draft.shell,
                        browse_id: "shell-path",
                        browse: &mut pick,
                    });
                } else if ui.button("Browse…").clicked() {
                    pick = true;
                }
                if pick {
                    *browse = Some(BrowseTarget::Shell);
                }
                if !self.settings_draft.shell.is_empty()
                    && self
                        .settings_executable(&self.settings_draft.shell)
                        .is_none()
                {
                    ui.colored_label(
                        appearance::color(&self.theme.status_failed),
                        "Shell not found",
                    );
                }
            },
        });
    }

    fn pr_metadata_row(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("Pull requests", "github gh metadata") {
            return;
        }
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Pull requests",
            description: "Uses GitHub CLI when the session service supports it.",
            add_control: |ui| {
                ui.add_enabled(
                    self.state
                        .capabilities
                        .iter()
                        .any(|c| c == METADATA_SETTINGS_CAPABILITY),
                    egui::Checkbox::new(&mut self.settings_draft.pr_metadata, "Fetch PR metadata"),
                );
            },
        });
    }

    fn font_size_row(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("Font size", "type terminal") {
            return;
        }
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Font size",
            description: "Applies to terminal surfaces.",
            add_control: |ui| {
                ui.add(egui::Slider::new(
                    &mut self.settings_draft.font_size,
                    9.0..=32.0,
                ));
            },
        });
    }

    /// GUI preference: saved with navigation preferences, not the daemon.
    fn type_ahead_row(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("Type-ahead", "type ahead prediction latency ssh echo") {
            return;
        }
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Type-ahead",
            description: "Show typed keys at once, dimmed, until the shell echoes them. Auto turns on when the echo takes more than 100 ms.",
            add_control: |ui| {
                settings_controls::segmented(
                    ui,
                    &mut self.preferences.type_ahead,
                    &[
                        ("Auto", crate::preferences::TypeAhead::Auto),
                        ("On", crate::preferences::TypeAhead::On),
                        ("Off", crate::preferences::TypeAhead::Off),
                    ],
                );
            },
        });
    }

    fn editor_mode_row(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("Editor mode", "embedded neovim terminal external") {
            return;
        }
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Editor mode",
            description: "Embedded uses Neovim with your config. External opens the app you choose.",
            add_control: |ui| {
                settings_controls::segmented(
                    ui,
                    &mut self.settings_draft.editor_mode,
                    &[
                        ("Embedded Neovim", EditorMode::Embedded),
                        ("Terminal editor", EditorMode::Terminal),
                        ("External editor", EditorMode::External),
                        ("Native editor", EditorMode::Native),
                    ],
                );
            },
        });
    }

    /// Vim keybindings and inlay hints, shown only for the native editor.
    pub(super) fn native_editor_rows(&mut self, ui: &mut egui::Ui) {
        let native = self.settings_draft.editor_mode == EditorMode::Native;
        if self.field_visible("Vim keybindings", "vim modal native") && native {
            settings_controls::settings_row(settings_controls::SettingsRowInput {
                ui,
                label: "Vim keybindings",
                description: "Native editor starts in Normal mode with the minimal vim grammar.",
                add_control: |ui| {
                    ui.checkbox(&mut self.settings_draft.native_vim, "Enable vim mode");
                },
            });
        }
        if self.field_visible("Inlay hints", "inlay hints types parameters native lsp") && native {
            settings_controls::settings_row(settings_controls::SettingsRowInput {
                ui,
                label: "Inlay hints",
                description: "Show inferred types and parameter names inline (from the language server).",
                add_control: |ui| {
                    ui.checkbox(&mut self.settings_draft.inlay_hints, "Show inlay hints");
                },
            });
        }
    }

    fn diff_viewer_row(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("Diff viewer", "native neovim review") {
            return;
        }
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Diff viewer",
            description: "Neovim review needs nvim-review-v1 on the running session service.",
            add_control: |ui| {
                settings_controls::segmented(
                    ui,
                    &mut self.settings_draft.review_mode,
                    &[
                        ("Native", ReviewMode::Native),
                        ("Neovim review", ReviewMode::Neovim),
                    ],
                );
            },
        });
    }

    fn editor_executable_row(
        &mut self,
        ui: &mut egui::Ui,
        editors: &[BinaryOption],
        browse: &mut Option<BrowseTarget>,
    ) {
        if !self.field_visible("Editor executable", "nvim neovim")
            || matches!(
                self.settings_draft.editor_mode,
                EditorMode::External | EditorMode::Native
            )
        {
            return;
        }
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Editor executable",
            description: "Embedded and terminal editors launch this Neovim.",
            add_control: |ui| {
                settings_controls::combo_or_custom(settings_controls::ComboOrCustomInput {
                    ui,
                    id: "editor-program",
                    value: &mut self.settings_draft.editor_program,
                    options: editors,
                    custom: &mut self.custom_editor,
                });
                let mut pick = false;
                if self.custom_editor {
                    settings_controls::path_field(settings_controls::PathFieldInput {
                        ui,
                        value: &mut self.settings_draft.editor_program,
                        browse_id: "editor-path",
                        browse: &mut pick,
                    });
                } else if ui.button("Browse…").clicked() {
                    pick = true;
                }
                if pick {
                    *browse = Some(BrowseTarget::Editor);
                }
                match self.settings_executable(&self.settings_draft.editor_program) {
                    Some(path) => ui.weak(path.display().to_string()),
                    None => ui.colored_label(
                        appearance::color(&self.theme.status_failed),
                        "nvim was not found on PATH",
                    ),
                };
            },
        });
    }

    fn external_editor_row(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("External editor", "vscode cursor zed rustrover") {
            return;
        }
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "External editor",
            description: "Used when opening files outside Terminator.",
            add_control: |ui| {
                egui::ComboBox::from_id_salt("external-preset")
                    .width(ui.available_width().clamp(180.0, 320.0))
                    .selected_text(
                        external_editor::PRESETS
                            .get(self.editor_preset)
                            .copied()
                            .unwrap_or("Custom"),
                    )
                    .show_ui(ui, |ui| self.external_preset_options(ui));
            },
        });
    }

    fn external_preset_options(&mut self, ui: &mut egui::Ui) {
        for (index, label) in external_editor::PRESETS.iter().enumerate() {
            if ui
                .selectable_value(&mut self.editor_preset, index, *label)
                .changed()
                && index != external_editor::CUSTOM
            {
                (
                    self.settings_draft.external_editor,
                    self.settings_draft.external_args,
                ) = external_editor::preset(index);
            }
        }
    }

    fn external_custom_row(&mut self, ui: &mut egui::Ui, browse: &mut Option<BrowseTarget>) {
        if self.editor_preset != external_editor::CUSTOM
            || !self.field_visible("Executable", "custom external binary")
        {
            return;
        }
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Executable",
            description: "Choose the binary. The file path is appended as {file}.",
            add_control: |ui| {
                let mut pick = false;
                let _executable =
                    settings_controls::path_field(settings_controls::PathFieldInput {
                        ui,
                        value: &mut self.settings_draft.external_editor,
                        browse_id: "external-program",
                        browse: &mut pick,
                    });
                if pick {
                    *browse = Some(BrowseTarget::External);
                }
                ui.weak("{file} is appended automatically.");
                ui.vertical(|ui| self.external_args_list(ui));
            },
        });
    }

    fn external_args_list(&mut self, ui: &mut egui::Ui) {
        let mut remove = None;
        for (index, arg) in self.settings_draft.external_args.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.text_edit_singleline(arg);
                if appearance::sidebar_action(ui, "X", "Remove argument").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            self.settings_draft.external_args.remove(index);
        }
        if ui.button("Add argument").clicked() {
            self.settings_draft.external_args.push(String::new());
        }
    }

    fn test_draft_row(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("Test draft", "launch editor") {
            return;
        }
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Test draft",
            description: "Opens a file with the unsaved external editor settings.",
            add_control: |ui| {
                if ui
                    .add_enabled(
                        !self.picker.busy(),
                        egui::Button::new("Choose file and test…"),
                    )
                    .clicked()
                {
                    self.request_pick(PickKind::TestEditor);
                }
            },
        });
    }

    fn folder_access_row(&self, ui: &mut egui::Ui, grant_folder: &mut bool) {
        if !cfg!(target_os = "macos")
            || !self.field_visible("Folder access", "privacy files folders")
        {
            return;
        }
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Folder access",
            description: "Choose a project folder to grant access. System Settings → Privacy & Security → Files and Folders controls protected locations.",
            add_control: |ui| {
                if ui.button("Choose folder…").clicked() {
                    *grant_folder = true;
                }
            },
        });
    }
}
