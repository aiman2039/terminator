use crate::*;

#[derive(Default)]
pub(super) struct SettingsFrame {
    pub(super) apply: bool,
    pub(super) cancel: bool,
    pub(super) hide: bool,
    pub(super) browse: Option<BrowseTarget>,
    pub(super) grant_folder: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum SettingsPending {
    Section(SettingsSection),
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingsSection {
    Appearance,
    Terminal,
    Notifications,
    History,
    Shortcuts,
    AgentHooks,
    Updates,
}

impl SettingsSection {
    pub const ALL: [Self; 7] = [
        Self::Appearance,
        Self::Terminal,
        Self::Notifications,
        Self::History,
        Self::Shortcuts,
        Self::AgentHooks,
        Self::Updates,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Appearance => "Appearance",
            Self::Terminal => "Terminal & Editor",
            Self::Notifications => "Notifications",
            Self::History => "History",
            Self::Shortcuts => "Shortcuts",
            Self::AgentHooks => "Agent Hooks",
            Self::Updates => "Updates",
        }
    }

    pub(super) fn icon(self) -> &'static str {
        match self {
            Self::Appearance | Self::Updates => "Settings2",
            Self::Terminal => "Terminal",
            Self::Notifications => "PanelsTopLeft",
            Self::History => "FileText",
            Self::Shortcuts => "SquareDashed",
            Self::AgentHooks => "GitBranch",
        }
    }

    pub(super) fn keywords(self) -> &'static str {
        match self {
            Self::Appearance => "theme accent density color hex font",
            Self::Terminal => {
                "shell nvim editor neovim zsh bash fish folder access privacy diff split close timeout"
            }
            Self::Notifications => "alert os desktop dismiss sound ntfy channel machine test",
            Self::History => "days mib scrollback disk",
            Self::Shortcuts => "keymap command shortcut chord palette",
            Self::AgentHooks => "claude codex opencode muse grok install hook test hello",
            Self::Updates => "sparkle install repair session service",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BrowseTarget {
    Shell,
    Editor,
    External,
    WorktreeDest,
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
        self.shortcut_capture = None;
        let _ = self.jobs.send(Job::HookStatus);
        self.settings_session = true;
    }

    pub(crate) fn open_settings(&mut self) {
        if !self.settings_session {
            self.begin_settings_session();
        }
        self.hide_center_overlay();
        self.settings_open = true;
    }

    pub(crate) fn end_settings_session(&mut self) {
        self.settings_open = false;
        self.settings_session = false;
        self.settings_pending = None;
        self.shortcut_capture = None;
    }

    pub(crate) fn settings_dirty(&self) -> bool {
        self.settings_session
            && (self.settings_draft != self.state.settings
                || self.theme_draft != self.theme_committed)
    }

    pub(crate) fn request_settings_section(&mut self, section: SettingsSection) {
        if section == self.settings_section || !self.settings_dirty() {
            self.settings_section = section;
            return;
        }
        self.settings_pending = Some(SettingsPending::Section(section));
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
            SettingsPending::Section(section) => self.settings_section = section,
            SettingsPending::Close => self.end_settings_session(),
        }
    }

    pub(crate) fn settings_unsaved_dialog(&mut self, ctx: &egui::Context) {
        if self.settings_pending.is_none() {
            return;
        }
        let mut save = false;
        let mut discard = false;
        let mut cancel = false;
        let valid = self.settings_validation().is_ok();
        self.popups
            .window(ctx, "Unsaved settings")
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label("You changed settings on this page. Save them before moving on?");
                ui.horizontal(|ui| {
                    let save_button = ui.add_enabled(valid, egui::Button::new("Save"));
                    #[cfg(feature = "test-support")]
                    diagnostics::record(ui.ctx(), "settings-unsaved-save", save_button.rect);
                    if save_button.clicked() {
                        save = true;
                    }
                    let discard_button = ui.button("Discard changes");
                    #[cfg(feature = "test-support")]
                    diagnostics::record(ui.ctx(), "settings-unsaved-discard", discard_button.rect);
                    if discard_button.clicked() {
                        discard = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if save {
            self.resolve_settings_pending(true);
        } else if discard {
            self.resolve_settings_pending(false);
        } else if cancel {
            self.settings_pending = None;
        }
    }

    pub(super) fn section_visible(&self, section: SettingsSection) -> bool {
        settings_controls::matches_search(
            &self.settings_search,
            &[section.title(), section.keywords()],
        )
    }

    pub(super) fn field_visible(&self, label: &str, keywords: &str) -> bool {
        if self.section_visible(self.settings_section)
            && settings_controls::matches_search(
                &self.settings_search,
                &[self.settings_section.title()],
            )
            && self.settings_search.trim().is_empty()
        {
            return true;
        }
        settings_controls::matches_search(
            &self.settings_search,
            &[self.settings_section.title(), label, keywords],
        )
    }

    pub(super) fn settings_validation(&self) -> Result<(), String> {
        self.theme_draft
            .validate()
            .map_err(|error| error.to_string())?;
        self.settings_draft
            .validate()
            .map_err(|error| error.to_string())?;
        if !self.settings_draft.shell.is_empty()
            && terminator_core::find_executable(&self.settings_draft.shell).is_none()
        {
            return Err(format!("Shell not found: {}", self.settings_draft.shell));
        }
        if self.editor_preset == external_editor::CUSTOM
            && terminator_core::find_executable(&self.settings_draft.external_editor).is_none()
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
        let next = if !self.settings_open {
            self.theme_committed.clone()
        } else if self.theme_draft.validate().is_ok() {
            self.theme_draft.clone()
        } else {
            return;
        };
        if self.theme != next {
            self.theme = next;
            appearance::apply(ctx, &self.theme);
        }
    }
}
