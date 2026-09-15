//! Settings window and appearance preview.
use super::*;

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

    fn icon(self) -> &'static str {
        match self {
            Self::Appearance | Self::Updates => "Settings2",
            Self::Terminal => "Terminal",
            Self::Notifications => "PanelsTopLeft",
            Self::History => "FileText",
            Self::Shortcuts => "SquareDashed",
            Self::AgentHooks => "GitBranch",
        }
    }

    fn keywords(self) -> &'static str {
        match self {
            Self::Appearance => "theme accent density color hex font",
            Self::Terminal => "shell nvim editor neovim zsh bash fish folder access privacy",
            Self::Notifications => "alert os desktop dismiss",
            Self::History => "days mib scrollback disk",
            Self::Shortcuts => "keymap command shortcut chord palette",
            Self::AgentHooks => "claude codex opencode muse grok install hook",
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
    pub(super) fn open_settings(&mut self) {
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
        self.settings_open = true;
    }

    pub(super) fn settings(&mut self, ctx: &egui::Context) {
        let mut open = true;
        let height = (ctx.content_rect().height() - 160.0).clamp(220.0, 580.0);
        let mut apply = false;
        let mut cancel = false;
        let mut browse = None;
        let mut grant_folder = false;
        self.popups
            .window(ctx, "Settings")
            .open(&mut open)
            .collapsible(false)
            .default_size([760.0, height + 110.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings_search)
                            .hint_text("Search settings")
                            .desired_width(280.0),
                    );
                    if !self.settings_search.is_empty() && ui.small_button("Clear").clicked() {
                        self.settings_search.clear();
                    }
                });
                ui.add_space(8.0);
                if !self.section_visible(self.settings_section)
                    && let Some(section) = SettingsSection::ALL
                        .into_iter()
                        .find(|section| self.section_visible(*section))
                {
                    self.settings_section = section;
                }
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(142.0);
                        ui.set_min_height(height);
                        ui.spacing_mut().item_spacing.y = 4.0;
                        for section in SettingsSection::ALL {
                            if !self.section_visible(section) {
                                continue;
                            }
                            let row = appearance::row(
                                ui,
                                section.title(),
                                section.icon(),
                                self.settings_section == section,
                                30.0,
                                "",
                                ui.visuals().weak_text_color(),
                            );
                            #[cfg(feature = "test-support")]
                            diagnostics::record(
                                ui.ctx(),
                                &format!("settings-section:{}", section.title()),
                                row.rect,
                            );
                            if row.clicked() {
                                self.settings_section = section;
                            }
                        }
                    });
                    let divider = ui.cursor().min;
                    ui.painter().line_segment(
                        [divider, divider + egui::vec2(0.0, height)],
                        ui.visuals().widgets.noninteractive.bg_stroke,
                    );
                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        ui.set_width(560.0);
                        egui::ScrollArea::vertical()
                            .id_salt(("settings-section", self.settings_section as u8))
                            .max_height(height)
                            .show(ui, |ui| match self.settings_section {
                                SettingsSection::Appearance => self.appearance_settings(ui),
                                SettingsSection::Terminal => {
                                    browse = self.terminal_settings(ui, &mut grant_folder);
                                }
                                SettingsSection::Notifications => self.notification_settings(ui),
                                SettingsSection::History => self.history_settings(ui),
                                SettingsSection::Shortcuts => self.shortcut_settings(ui),
                                SettingsSection::AgentHooks => self.hook_settings(ui),
                                SettingsSection::Updates => {
                                    self.installation_settings(ui);
                                    ui.add_space(12.0);
                                    ui.separator();
                                    self.updater.settings(ui);
                                }
                            });
                    });
                });
                ui.separator();
                let validation = self.settings_validation();
                if let Err(error) = &validation {
                    ui.colored_label(appearance::color(&self.theme.status_failed), error);
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(
                            validation.is_ok() && !self.theme_conflict,
                            egui::Button::new("Apply"),
                        )
                        .clicked()
                    {
                        apply = true;
                    }
                    let response = ui.button("Cancel");
                    #[cfg(feature = "test-support")]
                    diagnostics::record(ui.ctx(), "settings-cancel", response.rect);
                    if response.clicked() {
                        cancel = true;
                    }
                });
            });
        if grant_folder {
            self.add_project = true;
        }
        if let Some(target) = browse {
            self.browse_target = Some(target);
        }
        if apply {
            let _ = self.jobs.send(Job::SaveAppearance(
                Box::new(self.theme_draft.clone()),
                self.theme_source.clone(),
            ));
            self.send(Request::Settings(self.settings_draft.clone()));
        }
        if cancel {
            self.settings_open = false;
            self.shortcut_capture = None;
        }
        self.settings_open &= open;
        if !self.settings_open {
            self.shortcut_capture = None;
        }
        self.preview_appearance(ctx);
    }

    fn section_visible(&self, section: SettingsSection) -> bool {
        settings_controls::matches_search(
            &self.settings_search,
            &[section.title(), section.keywords()],
        )
    }

    fn field_visible(&self, label: &str, keywords: &str) -> bool {
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

    fn appearance_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Appearance");
        if self.theme_conflict {
            ui.colored_label(
                appearance::color(&self.theme.status_failed),
                "Configuration changed externally. Reload before applying.",
            );
            if ui.button("Reload appearance").clicked() {
                self.theme_draft = self.theme_committed.clone();
                self.theme_conflict = false;
            }
        }
        ui.weak("Preview colors and borders. Apply saves your changes.");
        ui.add_space(8.0);
        if self.field_visible("Theme", "preset dark contrast") {
            settings_controls::settings_row(
                ui,
                "Theme",
                "Presets fill colors. Advanced hex stays available below.",
                |ui| {
                    let mut choice = if self.theme_draft.colors_match(&self.theme_committed) {
                        0
                    } else if self.theme_draft.colors_match(&AppearanceConfig::default()) {
                        1
                    } else if self
                        .theme_draft
                        .colors_match(&AppearanceConfig::high_contrast())
                    {
                        2
                    } else {
                        3
                    };
                    if settings_controls::segmented(
                        ui,
                        &mut choice,
                        &[
                            ("Current saved", 0),
                            ("Terminator dark", 1),
                            ("High contrast", 2),
                        ],
                    ) {
                        let density = self.theme_draft.density;
                        let divider = self.theme_draft.pane_divider_width;
                        let border = self.theme_draft.border_width;
                        self.theme_draft = match choice {
                            1 => AppearanceConfig::default(),
                            2 => AppearanceConfig::high_contrast(),
                            _ => self.theme_committed.clone(),
                        };
                        self.theme_draft.density = density;
                        self.theme_draft.pane_divider_width = divider;
                        self.theme_draft.border_width = border;
                    }
                    if choice == 3 {
                        ui.weak("Custom colors");
                    }
                },
            );
        }
        if self.field_visible("Accent", "accent selection") {
            settings_controls::settings_row(
                ui,
                "Accent",
                "Selection color is derived from the accent.",
                |ui| {
                    let mut color = terminator_core::appearance::rgb(&self.theme_draft.accent)
                        .unwrap_or([56, 113, 225]);
                    if ui.color_edit_button_srgb(&mut color).changed() {
                        self.theme_draft.apply_accent(format!(
                            "#{:02X}{:02X}{:02X}",
                            color[0], color[1], color[2]
                        ));
                    }
                },
            );
        }
        if self.field_visible("Density", "compact comfortable spacing") {
            settings_controls::settings_row(
                ui,
                "Density",
                "Compact shortens sidebar rows and control height.",
                |ui| {
                    settings_controls::segmented(
                        ui,
                        &mut self.theme_draft.density,
                        &[
                            (
                                "Comfortable",
                                terminator_core::appearance::Density::Comfortable,
                            ),
                            ("Compact", terminator_core::appearance::Density::Compact),
                        ],
                    );
                },
            );
        }
        if ui.button("Reset appearance").clicked() {
            self.theme_draft = AppearanceConfig::default();
        }
        ui.add_space(8.0);
        for (group, title, keywords) in [
            (0, "Interface", "window surface hover border text"),
            (1, "Terminal", "terminal background foreground"),
            (2, "Git status", "git added modified deleted"),
            (3, "Agent status", "running waiting failed"),
        ] {
            if !self.field_visible(title, keywords) && !self.settings_search.is_empty() {
                continue;
            }
            egui::CollapsingHeader::new(title)
                .id_salt(("appearance-group", group))
                .default_open(false)
                .show(ui, |ui| {
                    egui::Grid::new(("appearance-fields", group))
                        .num_columns(2)
                        .min_col_width(155.0)
                        .spacing([20.0, 10.0])
                        .show(ui, |ui| {
                            for (name, value) in self.theme_draft.colors_mut() {
                                let category = if name.starts_with("terminal_") {
                                    1
                                } else if name.starts_with("git_") {
                                    2
                                } else if name.starts_with("status_") {
                                    3
                                } else {
                                    0
                                };
                                if category != group {
                                    continue;
                                }
                                let label = match name {
                                    "window" => "Workspace background".into(),
                                    "surface" => "Sidebar background".into(),
                                    "hover" => "Row highlight".into(),
                                    "text" => "Primary text".into(),
                                    "secondary" => "Secondary text".into(),
                                    _ => {
                                        let label = name.replace('_', " ");
                                        let mut chars = label.chars();
                                        chars.next().unwrap().to_uppercase().to_string()
                                            + chars.as_str()
                                    }
                                };
                                ui.label(label);
                                ui.horizontal(|ui| {
                                    let mut color = terminator_core::appearance::rgb(value)
                                        .unwrap_or([0, 0, 0]);
                                    if ui.color_edit_button_srgb(&mut color).changed() {
                                        *value = format!(
                                            "#{:02X}{:02X}{:02X}",
                                            color[0], color[1], color[2]
                                        );
                                    }
                                    ui.add_sized(
                                        [108.0, 28.0],
                                        egui::TextEdit::singleline(value)
                                            .font(egui::TextStyle::Monospace),
                                    );
                                    if terminator_core::appearance::rgb(value).is_err() {
                                        ui.colored_label(
                                            appearance::color(&self.theme.status_failed),
                                            "#RRGGBB",
                                        );
                                    }
                                });
                                ui.end_row();
                            }
                            if group == 0 {
                                ui.label("Border width");
                                ui.add(
                                    egui::DragValue::new(&mut self.theme_draft.border_width)
                                        .range(0.0..=8.0)
                                        .speed(0.1)
                                        .suffix(" pt"),
                                );
                                ui.end_row();
                                ui.label("Pane divider width");
                                ui.add(
                                    egui::DragValue::new(&mut self.theme_draft.pane_divider_width)
                                        .range(1.0..=24.0)
                                        .suffix(" pt"),
                                );
                                ui.end_row();
                            }
                        });
                });
        }
    }

    fn terminal_settings(
        &mut self,
        ui: &mut egui::Ui,
        grant_folder: &mut bool,
    ) -> Option<BrowseTarget> {
        ui.heading("Terminal & Editor");
        let mut browse = None;
        let shells = settings_controls::shell_options();
        let editors = settings_controls::editor_options();
        self.custom_shell |= shells
            .iter()
            .all(|option| option.value != self.settings_draft.shell);
        self.custom_editor |= editors
            .iter()
            .all(|option| option.value != self.settings_draft.editor_program);
        if self.field_visible("Shell override", "zsh bash fish sh") {
            settings_controls::settings_row(
                ui,
                "Shell override",
                "Used for new terminals. Leave Automatic unless you need a specific binary.",
                |ui| {
                    settings_controls::combo_or_custom(
                        ui,
                        "shell-override",
                        &mut self.settings_draft.shell,
                        &shells,
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
                        browse = Some(BrowseTarget::Shell);
                    }
                    if !self.settings_draft.shell.is_empty()
                        && terminator_core::find_executable(&self.settings_draft.shell).is_none()
                    {
                        ui.colored_label(
                            appearance::color(&self.theme.status_failed),
                            "Shell not found",
                        );
                    }
                },
            );
        }
        if self.field_visible("Pull requests", "github gh metadata") {
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
                        egui::Checkbox::new(
                            &mut self.settings_draft.pr_metadata,
                            "Fetch PR metadata",
                        ),
                    );
                },
            );
        }
        if self.field_visible("Font size", "type terminal") {
            settings_controls::settings_row(
                ui,
                "Font size",
                "Applies to terminal surfaces.",
                |ui| {
                    ui.add(egui::Slider::new(
                        &mut self.settings_draft.font_size,
                        9.0..=32.0,
                    ));
                },
            );
        }
        if self.field_visible("Editor mode", "embedded neovim terminal external") {
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
                        ],
                    );
                },
            );
        }
        if self.field_visible("Diff viewer", "native neovim review") {
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
        if self.field_visible("Editor executable", "nvim neovim")
            && self.settings_draft.editor_mode != EditorMode::External
        {
            settings_controls::settings_row(
                ui,
                "Editor executable",
                "Embedded and terminal editors launch this Neovim.",
                |ui| {
                    settings_controls::combo_or_custom(
                        ui,
                        "editor-program",
                        &mut self.settings_draft.editor_program,
                        &editors,
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
                        browse = Some(BrowseTarget::Editor);
                    }
                    match terminator_core::find_executable(&self.settings_draft.editor_program) {
                        Some(path) => ui.weak(path.display().to_string()),
                        None => ui.colored_label(
                            appearance::color(&self.theme.status_failed),
                            "nvim was not found on PATH",
                        ),
                    };
                },
            );
        }
        if self.field_visible("External editor", "vscode cursor zed rustrover") {
            settings_controls::settings_row(
                ui,
                "External editor",
                "Used when opening files outside Terminator.",
                |ui| {
                    egui::ComboBox::from_id_salt("external-preset")
                        .width(ui.available_width().clamp(180.0, 320.0))
                        .selected_text(external_editor::PRESETS[self.editor_preset])
                        .show_ui(ui, |ui| {
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
                        });
                },
            );
        }
        if self.editor_preset == external_editor::CUSTOM
            && self.field_visible("Executable", "custom external binary")
        {
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
                        browse = Some(BrowseTarget::External);
                    }
                    ui.weak("{file} is appended automatically.");
                    ui.vertical(|ui| {
                        let mut remove = None;
                        for (index, arg) in self.settings_draft.external_args.iter_mut().enumerate()
                        {
                            ui.horizontal(|ui| {
                                ui.text_edit_singleline(arg);
                                if ui.small_button("×").clicked() {
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
                    });
                },
            );
        }
        if self.field_visible("Test draft", "launch editor") {
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
        if cfg!(target_os = "macos") && self.field_visible("Folder access", "privacy files folders")
        {
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
        browse
    }

    fn notification_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Notifications");
        if self.field_visible("Events", "waiting permission completed failed") {
            egui::Grid::new("notification-settings")
                .num_columns(3)
                .show(ui, |ui| {
                    for state in [
                        AgentState::WaitingInput,
                        AgentState::WaitingPermission,
                        AgentState::Completed,
                        AgentState::Failed,
                    ] {
                        ui.label(state.label());
                        for (events, label) in [
                            (&mut self.settings_draft.events, "In app"),
                            (&mut self.settings_draft.os_events, "OS when unfocused"),
                        ] {
                            let mut enabled = events.contains(&state);
                            if ui.checkbox(&mut enabled, label).changed() {
                                if enabled {
                                    events.insert(state);
                                } else {
                                    events.remove(&state);
                                }
                            }
                        }
                        ui.end_row();
                    }
                });
        }
        if self.field_visible("Dismiss notifications", "focus resolve manual") {
            settings_controls::settings_row(
                ui,
                "Dismiss notifications",
                "When in-app attention is cleared.",
                |ui| {
                    settings_controls::segmented(
                        ui,
                        &mut self.settings_draft.dismissal,
                        &[
                            ("When opening terminal", Dismissal::OnFocus),
                            ("When request resolves", Dismissal::OnResolve),
                            ("Manually", Dismissal::Manual),
                        ],
                    );
                },
            );
        }
        ui.checkbox(
            &mut self.settings_draft.notifications_side,
            "Place notifications at the side",
        );
        ui.add_enabled_ui(
            self.state
                .capabilities
                .iter()
                .any(|c| c == TERMINAL_NOTICES_CAPABILITY),
            |ui| {
                ui.checkbox(
                    &mut self.settings_draft.terminal_notifications,
                    "Show terminal OSC notifications",
                );
                ui.checkbox(
                    &mut self.settings_draft.terminal_notifications_os,
                    "Also send terminal notifications to the desktop",
                );
            },
        );
    }

    fn history_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("History");
        if self.field_visible("Days", "age retain") {
            settings_controls::settings_row(ui, "Days", "How long saved output is kept.", |ui| {
                ui.add(egui::DragValue::new(&mut self.settings_draft.history_days).range(1..=3650));
            });
        }
        if self.field_visible("MiB per session", "disk") {
            settings_controls::settings_row(
                ui,
                "MiB per session",
                "Cap for one session's saved output.",
                |ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.settings_draft.session_mib).range(1..=4096),
                    );
                },
            );
        }
        if self.field_visible("MiB total", "disk") {
            settings_controls::settings_row(ui, "MiB total", "Cap across all sessions.", |ui| {
                ui.add(egui::DragValue::new(&mut self.settings_draft.total_mib).range(1..=65536));
            });
        }
        ui.weak("Limits apply to saved output; records and resume commands remain.");
    }

    fn shortcut_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Shortcuts");
        ui.weak("Click a row, then press the keys. Esc cancels capture.");
        if ui.button("Reset to defaults").clicked() {
            self.settings_draft.keybindings = Settings::default().keybindings;
            self.shortcut_capture = None;
        }
        ui.add_space(8.0);
        for (action, label) in shortcuts::ACTIONS {
            if !self.field_visible(label, action) {
                continue;
            }
            let binding = self
                .settings_draft
                .keybindings
                .entry((*action).into())
                .or_default()
                .clone();
            let capturing = self.shortcut_capture.as_deref() == Some(*action);
            let shown = if capturing {
                "Press keys…".into()
            } else if binding.is_empty() {
                "Unbound".into()
            } else {
                shortcuts::display(&binding)
            };
            settings_controls::settings_row(ui, label, action, |ui| {
                let button = ui.add_sized([180.0, 28.0], egui::Button::new(shown));
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), &format!("shortcut:{action}"), button.rect);
                if button.clicked() {
                    self.shortcut_capture = Some((*action).into());
                }
            });
        }
        if let Some(action) = self.shortcut_capture.clone() {
            let mut captured = None;
            let mut cancel = false;
            ui.input(|input| {
                for event in &input.events {
                    if let egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } = event
                    {
                        if *key == egui::Key::Escape && !modifiers.any() {
                            cancel = true;
                            continue;
                        }
                        captured = shortcuts::from_input(*modifiers, *key);
                    }
                }
            });
            if cancel {
                self.shortcut_capture = None;
            } else if let Some(value) = captured {
                self.settings_draft.keybindings.insert(action, value);
                self.shortcut_capture = None;
            }
        }
    }

    fn hook_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Agent Hooks");
        ui.weak("Agents are launched manually. Install hooks so Terminator can show waiting and done states.");
        egui::Grid::new("hook-settings")
            .num_columns(5)
            .show(ui, |ui| {
                for kind in terminator_integrations::AGENTS {
                    let installed = self.hook_status.get(kind).copied();
                    let binary = settings_controls::agent_binary(kind);
                    ui.label(kind);
                    match binary {
                        Some(path) => settings_controls::status_badge(
                            ui,
                            &format!("Detected · {}", path.display()),
                            true,
                        ),
                        None => settings_controls::status_badge(ui, "Not on PATH", false),
                    }
                    ui.weak(match installed {
                        Some(true) => "Configured",
                        Some(false) => "Not configured",
                        None => "Checking…",
                    })
                    .on_hover_text(
                        terminator_integrations::config_path(
                            Path::new(&std::env::var("HOME").unwrap_or_default()),
                            kind,
                        )
                        .map(|path| path.display().to_string())
                        .unwrap_or_default(),
                    );
                    if ui
                        .button(if installed == Some(true) {
                            "Repair"
                        } else {
                            "Install"
                        })
                        .clicked()
                    {
                        let _ = self.jobs.send(Job::Install(kind.to_string(), false));
                        let _ = self.jobs.send(Job::HookStatus);
                    }
                    if ui
                        .add_enabled(installed == Some(true), egui::Button::new("Remove"))
                        .clicked()
                    {
                        let _ = self.jobs.send(Job::Install(kind.to_string(), true));
                        let _ = self.jobs.send(Job::HookStatus);
                    }
                    ui.end_row();
                }
            });
        ui.weak("See docs/INTEGRATIONS.md for event limitations.");
    }

    fn settings_validation(&self) -> Result<(), String> {
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

    pub(super) fn preview_appearance(&mut self, ctx: &egui::Context) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_titles_match_fixture_targets() {
        assert_eq!(SettingsSection::Terminal.title(), "Terminal & Editor");
        assert_eq!(SettingsSection::Updates.title(), "Updates");
        assert_eq!(SettingsSection::ALL.len(), 7);
    }
}
