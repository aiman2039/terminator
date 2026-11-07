//! Layout and preference persistence results, and the error/attention
//! notices that follow them.

use crate::*;

/// Wait after a failed preferences save before writing again.
const PREFERENCES_RETRY_DELAY: Duration = Duration::from_secs(5);

impl App {
    pub(super) fn handle_layouts_prepared(
        &mut self,
        generation: u64,
        layouts: Vec<(String, serde_json::Value, String)>,
    ) {
        // The exit checkpoint saves layouts itself; a cancelled exit
        // resets the signature so these are sent again (`cancel_exit`).
        if generation != self.layout_generation || self.exit.active() {
            return;
        }
        for (project, value, text) in layouts {
            if !self.can_persist_layout(&project)
                || self.layout_saved.get(&project) == Some(&text)
                || self.layout_pending.get(&project) == Some(&text)
            {
                continue;
            }
            let job = Job::SaveLayout(project.clone(), value, text.clone());
            if self.jobs.send(job).is_ok() {
                self.layout_pending.insert(project, text);
            } else {
                // Not sent: the next pass prepares and sends it again.
                self.layout_signature = None;
            }
        }
    }

    /// The prepare job of `generation` ended without its layouts.
    pub(super) fn layout_prepare_failed(&mut self, generation: u64) {
        if generation == self.layout_generation {
            self.layout_signature = None;
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

    pub(crate) fn handle_preferences_saved(&mut self, result: Result<UiPreferences, String>) {
        self.preferences_pending = false;
        match result {
            Ok(prefs) => {
                self.preferences_saved = prefs;
                self.preferences_retry_at = None;
            }
            Err(error) => {
                self.preferences_retry_at = Instant::now().checked_add(PREFERENCES_RETRY_DELAY);
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
                self.error
                    .set(format!("Attention settings migration: {error}"));
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
