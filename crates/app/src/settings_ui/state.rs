use crate::*;

#[derive(Default)]
pub(super) struct SettingsFrame {
    pub(super) apply: bool,
    pub(super) cancel: bool,
    pub(super) hide: bool,
    pub(super) browse: Option<BrowseTarget>,
    pub(super) grant_folder: bool,
}

/// A question the Settings draft waits on. Only closing with unsaved
/// changes asks; a section change keeps the draft and never asks.
#[derive(Clone, Copy, Debug)]
pub(crate) enum SettingsPending {
    Close,
}

/// Top-level fields that differ between two serialized values.
fn changed_fields(draft: &impl serde::Serialize, saved: &impl serde::Serialize) -> usize {
    let (Ok(serde_json::Value::Object(draft)), Ok(serde_json::Value::Object(saved))) =
        (serde_json::to_value(draft), serde_json::to_value(saved))
    else {
        return 0;
    };
    draft
        .iter()
        .filter(|(key, value)| saved.get(*key) != Some(*value))
        .count()
}

pub(crate) use super::sections::SettingsSection;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BrowseTarget {
    Shell,
    Editor,
    External,
    WorktreeDest,
    Lsp(&'static str),
    /// A terminal theme file for Settings → Appearance import.
    ThemeFile,
}

impl App {
    fn begin_settings_session(&mut self) {
        self.settings_draft = self.state.settings.clone();
        shortcuts::fill_defaults(&mut self.settings_draft.keybindings);
        self.editor_preset = external_editor::selected(&self.settings_draft);
        self.theme_draft = self.theme_committed.clone();
        self.theme_conflict = false;
        self.settings_search.clear();
        self.custom_shell = false;
        self.custom_editor = false;
        self.settings_probes.get_mut().clear();
        self.shortcut_capture = None;
        let _ = self.jobs.send(Job::HookStatus);
        self.settings_session = true;
    }

    pub(crate) fn open_settings(&mut self) {
        // Already showing: hiding first would ask to discard the draft
        // that stays on screen.
        if self.settings_open() {
            return;
        }
        if !self.settings_session {
            self.begin_settings_session();
        }
        // Back on the page: a Close question from hiding it is moot.
        if matches!(self.settings_pending, Some(SettingsPending::Close)) {
            self.settings_pending = None;
        }
        self.show_center(CenterPage::Settings);
    }

    pub(crate) fn end_settings_session(&mut self) {
        self.settings_session = false;
        self.settings_pending = None;
        self.shortcut_capture = None;
        self.close_center_page(&CenterPage::Settings);
    }

    pub(crate) fn settings_dirty(&self) -> bool {
        self.settings_session
            && (self.settings_draft != self.state.settings
                || self.theme_draft != self.theme_committed)
    }

    /// How many settings the draft changes (Apply saves them all).
    pub(crate) fn settings_change_count(&self) -> usize {
        if !self.settings_session {
            return 0;
        }
        changed_fields(&self.settings_draft, &self.state.settings)
            .saturating_add(changed_fields(&self.theme_draft, &self.theme_committed))
    }

    /// The draft is kept across sections, so a section change never asks.
    pub(crate) fn request_settings_section(&mut self, section: SettingsSection) {
        self.settings_section = section.canonical();
    }

    /// Discard changes: back to the saved settings, without a question.
    pub(super) fn discard_settings_draft(&mut self) {
        self.revert_settings_draft();
    }

    pub(crate) fn request_settings_close(&mut self) {
        if !self.settings_dirty() {
            self.end_settings_session();
            return;
        }
        self.settings_pending = Some(SettingsPending::Close);
    }

    pub(super) fn apply_settings(&mut self) {
        let _ = self.jobs.send(Job::SaveAppearance(
            Box::new(self.theme_draft.clone()),
            self.theme_source.clone(),
        ));
        self.send(Request::Settings(self.settings_draft.clone()));
    }

    fn revert_settings_draft(&mut self) {
        self.settings_draft = self.state.settings.clone();
        shortcuts::fill_defaults(&mut self.settings_draft.keybindings);
        self.editor_preset = external_editor::selected(&self.settings_draft);
        self.theme_draft = self.theme_committed.clone();
        self.theme_conflict = false;
        self.shortcut_capture = None;
    }

    pub(crate) fn resolve_settings_pending(&mut self, save: bool) {
        let Some(pending) = self.settings_pending.take() else {
            return;
        };
        if save {
            if self.settings_validation().is_err() {
                self.settings_pending = Some(pending);
                return;
            }
            self.apply_settings();
        } else {
            self.revert_settings_draft();
        }
        match pending {
            SettingsPending::Close => self.end_settings_session(),
        }
    }

    pub(crate) fn settings_unsaved_dialog(&mut self, ctx: &egui::Context) {
        if self.settings_pending.is_none() {
            return;
        }
        let invalid = self.settings_validation().err();
        let window = self.popups.window(ctx, "Unsaved settings");
        let count = self.settings_change_count();
        let dialog = appearance::Confirm::new(
            format!(
                "{} not applied. Apply before closing?",
                appearance::count_noun(count, "settings change")
            ),
            "",
        )
        .cancel(appearance::ConfirmButton::new("Keep editing"))
        .destructive(
            appearance::ConfirmButton::new("Discard and close").id("settings-unsaved-discard"),
        )
        .default_action(
            appearance::ConfirmButton::new("Apply and close")
                .id("settings-unsaved-save")
                .disabled(invalid.as_deref()),
        );
        match appearance::confirm(ctx, window, dialog, |_| {}) {
            Some(appearance::ConfirmAnswer::Default) => self.resolve_settings_pending(true),
            Some(appearance::ConfirmAnswer::Destructive) => self.resolve_settings_pending(false),
            Some(appearance::ConfirmAnswer::Cancel) => self.settings_pending = None,
            None => {}
        }
    }

    /// A section shows while the search matches its title or one of its
    /// rows (every word, any order).
    pub(super) fn section_visible(&self, section: SettingsSection) -> bool {
        self.settings_search.trim().is_empty()
            || settings_controls::matches_search(&self.settings_search, &[&section.haystack()])
    }

    /// A row of the open section shows while the search matches it or the
    /// section title.
    pub(super) fn field_visible(&self, label: &str, keywords: &str) -> bool {
        #[cfg(test)]
        super::settings_tests::seen_row(label, keywords);
        let shown = settings_controls::matches_search(
            &self.settings_search,
            &[self.settings_section.title(), label, keywords],
        );
        if shown && !self.settings_search.trim().is_empty() {
            let mut probes = self.settings_probes.borrow_mut();
            let count = probes
                .matches
                .entry(self.settings_section.canonical().title())
                .or_default();
            *count = count.saturating_add(1);
        }
        shown
    }

    /// Rows of `section` the search matched when the body last painted.
    pub(super) fn section_match_count(&self, section: SettingsSection) -> usize {
        self.settings_probes
            .borrow()
            .matches
            .get(section.title())
            .copied()
            .unwrap_or(0)
    }

    /// Start a new count of matched rows for this frame's body.
    pub(super) fn reset_section_matches(&self) {
        self.settings_probes.borrow_mut().matches.clear();
    }

    pub(super) fn settings_validation(&self) -> Result<(), String> {
        self.theme_draft
            .validate()
            .map_err(|error| error.to_string())?;
        self.settings_draft
            .validate()
            .map_err(|error| error.to_string())?;
        if !self.settings_draft.shell.is_empty()
            && self
                .settings_executable(&self.settings_draft.shell)
                .is_none()
        {
            return Err(format!("Shell not found: {}", self.settings_draft.shell));
        }
        if self.editor_preset == external_editor::CUSTOM
            && self
                .settings_executable(&self.settings_draft.external_editor)
                .is_none()
        {
            return Err(format!(
                "External editor not found: {}",
                self.settings_draft.external_editor
            ));
        }
        if let Some(error) = shortcuts::invalid(&self.settings_draft.keybindings) {
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn preview_appearance(&mut self, ctx: &egui::Context) {
        let next = if !self.settings_open() {
            self.theme_committed.clone()
        } else if self.theme_draft.validate().is_ok() {
            self.theme_draft.clone()
        } else {
            return;
        };
        let next = next.for_os(appearance::os_dark(ctx));
        if self.theme != next {
            self.theme = next;
            appearance::apply(ctx, &self.theme);
        }
    }
}

/// PATH probe results cached for one Settings session.
#[derive(Default)]
pub(crate) struct Probes {
    executables: std::collections::HashMap<String, Option<std::path::PathBuf>>,
    agents: std::collections::HashMap<String, Option<std::path::PathBuf>>,
    options: Option<(
        Vec<settings_controls::BinaryOption>,
        Vec<settings_controls::BinaryOption>,
    )>,
    /// Search matches per section title, from the last painted body.
    matches: std::collections::HashMap<&'static str, usize>,
}

impl Probes {
    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }

    /// Agent binary for `kind`, probed once per Settings session.
    fn agent(
        &mut self,
        kind: &str,
        probe: impl FnOnce(&str) -> Option<std::path::PathBuf>,
    ) -> Option<std::path::PathBuf> {
        self.agents
            .entry(kind.to_owned())
            .or_insert_with(|| probe(kind))
            .clone()
    }
}

impl App {
    /// `find_executable`, probed once per value for the Settings session.
    pub(super) fn settings_executable(&self, name: &str) -> Option<std::path::PathBuf> {
        self.settings_probes
            .borrow_mut()
            .executables
            .entry(name.to_owned())
            .or_insert_with(|| terminator_core::find_executable(name))
            .clone()
    }

    /// Installed agent binary, probed once per kind for the Settings session.
    pub(super) fn settings_agent_binary(&self, kind: &str) -> Option<std::path::PathBuf> {
        self.settings_probes
            .borrow_mut()
            .agent(kind, settings_controls::agent_binary)
    }

    /// Detected shell and editor choices, probed once per Settings session.
    pub(super) fn settings_binary_options(
        &self,
    ) -> (
        Vec<settings_controls::BinaryOption>,
        Vec<settings_controls::BinaryOption>,
    ) {
        self.settings_probes
            .borrow_mut()
            .options
            .get_or_insert_with(|| {
                (
                    settings_controls::shell_options(),
                    settings_controls::editor_options(),
                )
            })
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::Probes;

    #[test]
    fn agent_binary_probe_runs_once_per_settings_session() {
        let mut probes = Probes::default();
        let mut calls = 0;
        for _ in 0..3 {
            probes.agent("codex", |_| {
                calls += 1;
                Some("/bin/codex".into())
            });
        }
        assert_eq!(calls, 1);
        probes.clear();
        probes.agent("codex", |_| {
            calls += 1;
            None
        });
        assert_eq!(calls, 2);
    }
}
