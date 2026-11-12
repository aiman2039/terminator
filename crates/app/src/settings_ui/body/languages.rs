//! Settings → Languages: language-server paths and spell checking.
use super::super::state::BrowseTarget;
use crate::*;

impl App {
    pub(super) fn language_settings(
        &mut self,
        ui: &mut egui::Ui,
        browse: &mut Option<BrowseTarget>,
    ) {
        ui.heading("Languages");
        self.lsp_server_rows(ui, browse);
        self.spell_check_row(ui);
    }

    /// Spell checking for both editors. Neovim needs a daemon that
    /// passes the flag on (`editor-spell-v1`); the native editor does not.
    fn spell_check_row(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("Spell checking", "spell spelling typo harper dictionary") {
            return;
        }
        let native = self.settings_draft.editor_mode == EditorMode::Native;
        let enabled = native
            || self
                .state
                .capabilities
                .iter()
                .any(|c| c == terminator_core::EDITOR_SPELL_CAPABILITY);
        let description = if native {
            "Underline typos in text and comments (harper-ls on PATH). Alt+Enter lists fixes."
        } else {
            "Turn on Neovim 'spell' for new editors. z= lists fixes, zg adds a word."
        };
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Spell checking",
            description,
            add_control: |ui| {
                ui.add_enabled(
                    enabled,
                    egui::Checkbox::new(&mut self.settings_draft.spell_check, "Check spelling"),
                )
                .on_disabled_hover_text("Restart the daemon to use this with Neovim editors.");
            },
        });
    }

    fn lsp_server_rows(&mut self, ui: &mut egui::Ui, browse: &mut Option<BrowseTarget>) {
        for family in terminator_native_edit::lsp::SERVER_FAMILIES {
            let (language, label) = (family.id, family.label);
            let keywords = format!(
                "lsp language server {language} {}",
                family.candidates.join(" ")
            );
            if !self.field_visible(label, &keywords) {
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
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label,
            description: "Empty finds it on PATH. Saving restarts that server; the app keeps running.",
            add_control: |ui| {
                let mut value = self
                    .settings_draft
                    .lsp_servers
                    .get(language)
                    .cloned()
                    .unwrap_or_default();
                let _ = settings_controls::path_field(settings_controls::PathFieldInput {
                    ui,
                    value: &mut value,
                    browse_id: &format!("lsp-{language}"),
                    browse: &mut pick,
                });
                if value.is_empty() {
                    self.settings_draft.lsp_servers.remove(language);
                } else {
                    self.settings_draft
                        .lsp_servers
                        .insert(language.to_owned(), value.clone());
                }
                self.lsp_server_status(ui, value.trim());
            },
        });
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
}
