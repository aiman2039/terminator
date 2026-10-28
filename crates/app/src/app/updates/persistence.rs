//! Layout and preference persistence results, and the error/attention
//! notices that follow them.

use crate::*;

impl App {
    pub(super) fn handle_layouts_prepared(
        &mut self,
        generation: u64,
        layouts: Vec<(String, serde_json::Value, String)>,
    ) {
        if generation != self.layout_generation || self.exit.active() {
            return;
        }
        for (project, value, text) in layouts {
            if self.can_persist_layout(&project)
                && self.layout_saved.get(&project) != Some(&text)
                && self.layout_pending.get(&project) != Some(&text)
                && self
                    .jobs
                    .send(Job::SaveLayout(project.clone(), value, text.clone()))
                    .is_ok()
            {
                self.layout_pending.insert(project, text);
            }
        }
    }

    pub(super) fn handle_layout_saved(
        &mut self,
        project: String,
        text: String,
        result: Result<(), String>,
    ) {
        if self.layout_pending.get(&project) == Some(&text) {
            self.layout_pending.remove(&project);
        }
        match result {
            Ok(()) => {
                self.layout_saved.insert(project, text);
            }
            Err(error) => {
                // Force the next pass to resend so a transient
                // failure is retried instead of being treated as
                // "already persisted" by the unchanged signature.
                self.layout_signature = None;
                self.layout_save_failed(error);
            }
        }
    }

    pub(super) fn handle_preferences_saved(&mut self, result: Result<UiPreferences, String>) {
        self.preferences_pending = false;
        match result {
            Ok(prefs) => self.preferences_saved = prefs,
            Err(error) => {
                if self.exit.active() {
                    self.cancel_exit(error);
                } else {
                    self.report_status_error(error);
                }
            }
        }
    }

    pub(super) fn handle_attention_migrated(&mut self, result: Result<(), String>) {
        self.attention_pending = false;
        match result {
            Ok(()) => self.preferences.attention_migrated = true,
            Err(error) => {
                self.error = Some(format!("Attention settings migration: {error}"));
            }
        }
    }

    pub(super) fn handle_appearance(&mut self, ctx: &egui::Context, file: AppearanceFile) {
        let dirty = self.settings_session && self.theme_draft != self.theme_committed;
        self.theme_conflict = dirty && file.config != self.theme_draft;
        self.theme_committed = file.config.clone();
        self.theme_source = file.source;
        if !self.theme_conflict {
            self.theme_draft = file.config.clone();
            self.theme = file.config;
            appearance::apply(ctx, &self.theme);
        }
    }
}
