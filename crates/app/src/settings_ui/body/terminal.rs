use super::super::state::BrowseTarget;
use crate::settings_controls::BinaryOption;
use crate::*;

impl App {
    pub(super) fn terminal_settings(
        &mut self,
        ui: &mut egui::Ui,
        grant_folder: &mut bool,
    ) -> Option<BrowseTarget> {
        ui.heading("Terminal & Editor");
        let mut browse = None;
        let (shells, editors) = self.settings_binary_options();
        self.detect_custom_binaries(&shells, &editors);
        self.shell_override_row(ui, &shells, &mut browse);
        self.pr_metadata_row(ui);
        self.font_size_row(ui);
        self.editor_mode_row(ui);
        self.native_editor_rows(ui);
        self.lsp_server_rows(ui, &mut browse);
        self.diff_viewer_row(ui);
        self.diff_close_settings(ui);
        self.editor_executable_row(ui, &editors, &mut browse);
        self.external_editor_row(ui);
        self.external_custom_row(ui, &mut browse);
        self.test_draft_row(ui);
        self.folder_access_row(ui, grant_folder);
        browse
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
        settings_controls::settings_row(
            ui,
            "Shell override",
            "Used for new terminals. Leave Automatic unless you need a specific binary.",
            |ui| {
                settings_controls::combo_or_custom(
                    ui,
                    "shell-override",
                    &mut self.settings_draft.shell,
                    shells,
                    &mut self.custom_shell,
                );
                let mut pick = false;
                if self.custom_shell {
                    settings_controls::path_field(
                        ui,
                        &mut self.settings_draft.shell,
                        "shell-path",
                        &mut pick,
                    );
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
        );
    }

    fn pr_metadata_row(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("Pull requests", "github gh metadata") {
            return;
        }
        settings_controls::settings_row(
            ui,
            "Pull requests",
            "Uses GitHub CLI when the session service supports it.",
            |ui| {
                ui.add_enabled(
                    self.state
                        .capabilities
                        .iter()
                        .any(|c| c == METADATA_SETTINGS_CAPABILITY),
                    egui::Checkbox::new(&mut self.settings_draft.pr_metadata, "Fetch PR metadata"),
                );
            },
        );
    }

    fn font_size_row(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("Font size", "type terminal") {
            return;
        }
        settings_controls::settings_row(ui, "Font size", "Applies to terminal surfaces.", |ui| {
            ui.add(egui::Slider::new(
                &mut self.settings_draft.font_size,
                9.0..=32.0,
            ));
        });
    }

    fn editor_mode_row(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("Editor mode", "embedded neovim terminal external") {
            return;
        }
        settings_controls::settings_row(
            ui,
            "Editor mode",
            "Embedded uses Neovim with your config. External opens the app you choose.",
            |ui| {
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
        );
    }

    /// Vim keybindings and inlay hints, shown only for the native editor.
    fn native_editor_rows(&mut self, ui: &mut egui::Ui) {
        let native = self.settings_draft.editor_mode == EditorMode::Native;
        if self.field_visible("Vim keybindings", "vim modal native") && native {
            settings_controls::settings_row(
                ui,
                "Vim keybindings",
                "Native editor starts in Normal mode with the minimal vim grammar.",
                |ui| {
                    ui.checkbox(&mut self.settings_draft.native_vim, "Enable vim mode");
                },
            );
        }
        if self.field_visible("Inlay hints", "inlay hints types parameters native lsp") && native {
            settings_controls::settings_row(
                ui,
                "Inlay hints",
                "Show inferred types and parameter names inline (from the language server).",
                |ui| {
                    ui.checkbox(&mut self.settings_draft.inlay_hints, "Show inlay hints");
                },
            );
        }
    }

    fn lsp_server_rows(&mut self, ui: &mut egui::Ui, browse: &mut Option<BrowseTarget>) {
        for (language, label) in [
            ("rust", "Rust language server"),
            ("python", "Python language server"),
            ("typescript", "TypeScript language server"),
        ] {
            if !self.field_visible(label, "lsp language server rust python typescript") {
                continue;
            }
            if self.lsp_server_row(ui, language, label) {
                *browse = Some(BrowseTarget::Lsp(language));
            }
        }
    }

    /// One language-server path row. Returns whether Browse was clicked.
    fn lsp_server_row(&mut self, ui: &mut egui::Ui, language: &str, label: &str) -> bool {
        let mut pick = false;
        settings_controls::settings_row(
            ui,
            label,
            "Empty finds it on PATH. Saving restarts that server; the app keeps running.",
            |ui| {
                let mut value = self
                    .settings_draft
                    .lsp_servers
                    .get(language)
                    .cloned()
                    .unwrap_or_default();
                let _ = settings_controls::path_field(
                    ui,
                    &mut value,
                    &format!("lsp-{language}"),
                    &mut pick,
                );
                if value.is_empty() {
                    self.settings_draft.lsp_servers.remove(language);
                } else {
                    self.settings_draft
                        .lsp_servers
                        .insert(language.to_owned(), value.clone());
                }
                self.lsp_server_status(ui, value.trim());
            },
        );
        pick
    }

    fn lsp_server_status(&self, ui: &mut egui::Ui, value: &str) {
        if value.is_empty() {
            return;
        }
        match self.settings_executable(value) {
            Some(path) => ui.weak(path.display().to_string()),
            None => ui.colored_label(
                appearance::color(&self.theme.status_failed),
                "Language server not found",
            ),
        };
    }

    fn diff_viewer_row(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("Diff viewer", "native neovim review") {
            return;
        }
        settings_controls::settings_row(
            ui,
            "Diff viewer",
            "Neovim review needs nvim-review-v1 on the running session service.",
            |ui| {
                settings_controls::segmented(
                    ui,
                    &mut self.settings_draft.review_mode,
                    &[
                        ("Native", ReviewMode::Native),
                        ("Neovim review", ReviewMode::Neovim),
                    ],
                );
            },
        );
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
        settings_controls::settings_row(
            ui,
            "Editor executable",
            "Embedded and terminal editors launch this Neovim.",
            |ui| {
                settings_controls::combo_or_custom(
                    ui,
                    "editor-program",
                    &mut self.settings_draft.editor_program,
                    editors,
                    &mut self.custom_editor,
                );
                let mut pick = false;
                if self.custom_editor {
                    settings_controls::path_field(
                        ui,
                        &mut self.settings_draft.editor_program,
                        "editor-path",
                        &mut pick,
                    );
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
        );
    }

    fn external_editor_row(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("External editor", "vscode cursor zed rustrover") {
            return;
        }
        settings_controls::settings_row(
            ui,
            "External editor",
            "Used when opening files outside Terminator.",
            |ui| {
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
        );
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
        settings_controls::settings_row(
            ui,
            "Executable",
            "Choose the binary. The file path is appended as {file}.",
            |ui| {
                let mut pick = false;
                let _executable = settings_controls::path_field(
                    ui,
                    &mut self.settings_draft.external_editor,
                    "external-program",
                    &mut pick,
                );
                if pick {
                    *browse = Some(BrowseTarget::External);
                }
                ui.weak("{file} is appended automatically.");
                ui.vertical(|ui| self.external_args_list(ui));
            },
        );
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
        settings_controls::settings_row(
            ui,
            "Test draft",
            "Opens a file with the unsaved external editor settings.",
            |ui| {
                if ui
                    .add_enabled(
                        !self.picker_active,
                        egui::Button::new("Choose file and test…"),
                    )
                    .clicked()
                {
                    self.test_editor = true;
                }
            },
        );
    }

    fn folder_access_row(&self, ui: &mut egui::Ui, grant_folder: &mut bool) {
        if !cfg!(target_os = "macos")
            || !self.field_visible("Folder access", "privacy files folders")
        {
            return;
        }
        settings_controls::settings_row(
            ui,
            "Folder access",
            "Choose a project folder to grant access. System Settings → Privacy & Security → Files and Folders controls protected locations.",
            |ui| {
                if ui.button("Choose folder…").clicked() {
                    *grant_folder = true;
                }
            },
        );
    }
}
