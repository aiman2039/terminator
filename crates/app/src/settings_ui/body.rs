use super::state::{BrowseTarget, SettingsFrame, SettingsSection};
use crate::*;

impl App {
    pub(super) fn paint_settings_body(
        &mut self,
        ui: &mut egui::Ui,
        height: f32,
        frame: &mut SettingsFrame,
    ) {
        ui.vertical(|ui| {
            ui.set_min_width(ui.available_width().max(240.0));
            egui::ScrollArea::vertical()
                .id_salt(("settings-section", self.settings_section as u8))
                .max_height(height)
                .auto_shrink([false, false])
                .show(ui, |ui| match self.settings_section {
                    SettingsSection::Appearance => self.appearance_settings(ui),
                    SettingsSection::Terminal => {
                        let mut grant = false;
                        frame.browse = self.terminal_settings(ui, &mut grant);
                        frame.grant_folder |= grant;
                    }
                    SettingsSection::Notifications => self.notification_settings(ui),
                    SettingsSection::History => self.history_settings(ui),
                    SettingsSection::Shortcuts => self.shortcut_settings(ui),
                    SettingsSection::AgentHooks => self.hook_settings(ui),
                    SettingsSection::Updates => {
                        self.installation_settings(ui);
                        ui.add_space(12.0);
                        ui.separator();
                        #[cfg(any(windows, test))]
                        self.windows_updates
                            .settings(ui, &mut self.settings_draft.automatic_update_checks);
                        #[cfg(not(windows))]
                        self.updater.settings(ui);
                    }
                });
        });
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
                                        match chars.next() {
                                            Some(first) => {
                                                first.to_uppercase().to_string() + chars.as_str()
                                            }
                                            None => label,
                                        }
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
                                        appearance::singleline(value)
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
                            ("Native editor", EditorMode::Native),
                        ],
                    );
                },
            );
        }
        if self.field_visible("Vim keybindings", "vim modal native")
            && self.settings_draft.editor_mode == EditorMode::Native
        {
            settings_controls::settings_row(
                ui,
                "Vim keybindings",
                "Native editor starts in Normal mode with the minimal vim grammar.",
                |ui| {
                    ui.checkbox(&mut self.settings_draft.native_vim, "Enable vim mode");
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
        self.diff_close_settings(ui);
        if self.field_visible("Editor executable", "nvim neovim")
            && !matches!(
                self.settings_draft.editor_mode,
                EditorMode::External | EditorMode::Native
            )
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
                        .selected_text(
                            external_editor::PRESETS
                                .get(self.editor_preset)
                                .copied()
                                .unwrap_or("Custom"),
                        )
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
        if self.field_visible("ntfy", "channel machine phone push") {
            // Clearing the channel is the removal gesture: an enabled toggle
            // with no channel can never validate, so switch the toggle off
            // instead of trapping Apply behind an error.
            if self.settings_draft.ntfy_channel.is_empty() {
                self.settings_draft.ntfy_enabled = false;
            }
            ui.add_enabled_ui(
                self.state.capabilities.iter().any(|c| c == NTFY_CAPABILITY),
                |ui| {
                    ui.checkbox(&mut self.settings_draft.ntfy_enabled, "Send agent notifications to ntfy");
                    ui.horizontal(|ui| {
                        ui.label("Channel (ntfy.sh)");
                        ui.text_edit_singleline(&mut self.settings_draft.ntfy_channel);
                    });
                    ui.horizontal(|ui| {
                        ui.label("Machine name");
                        ui.text_edit_singleline(&mut self.settings_draft.ntfy_machine);
                    });
                    ui.small("Sends the In app events above, even while focused or the GUI is closed. Subscribe to this channel in ntfy. Sends only agent and status, not prompt text.");
                },
            );
            if !self.state.capabilities.iter().any(|c| c == NTFY_CAPABILITY) {
                ui.small("Activate an updated daemon to enable ntfy. Older sessions need a supporting daemon.");
            }
            ui.horizontal(|ui| {
                let channel = self.settings_draft.ntfy_channel.clone();
                let machine = self.settings_draft.ntfy_machine.clone();
                let ready = !channel.is_empty();
                let test = ui.add_enabled(ready, egui::Button::new("Send test"));
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "ntfy-send-test", test.rect);
                if test.clicked()
                    && let Err(error) = self.send_ntfy_test(channel.clone(), machine.clone())
                {
                    self.error = Some(error);
                }
                let copy = ui.add_enabled(ready, egui::Button::new("Copy test command"));
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "ntfy-copy-test", copy.rect);
                if copy.clicked() {
                    ui.ctx()
                        .copy_text(notify_test::curl_command(&channel, &machine));
                    self.info = Some("ntfy test command copied.".into());
                }
            });
            ui.small("Test posts directly to ntfy.sh with this channel; no Apply needed. Real alerts need the toggle on plus Apply, and no hook reinstall.");
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
        if self.field_visible("Play sound with desktop notifications", "sound alert") {
            ui.add_enabled_ui(
                self.state
                    .capabilities
                    .iter()
                    .any(|c| c == NOTIFICATION_SOUND_CAPABILITY),
                |ui| {
                    ui.checkbox(
                        &mut self.settings_draft.notification_sound,
                        "Play sound with desktop notifications",
                    );
                },
            );
        }
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

    fn send_ntfy_test(&mut self, channel: String, machine: String) -> Result<(), String> {
        notify_test::validate(&channel, &machine)?;
        let _ = self.jobs.send(Job::TestNtfy { channel, machine });
        self.info = Some("Sending ntfy test…".into());
        Ok(())
    }

    fn send_hello_test(&mut self) {
        let live = |id: &str| {
            self.state
                .sessions
                .iter()
                .any(|s| s.id == id && s.lifecycle.live())
        };
        let sid = self
            .active_session
            .clone()
            .filter(|id| live(id))
            .or_else(|| {
                self.state
                    .sessions
                    .iter()
                    .find(|s| s.lifecycle.live())
                    .map(|s| s.id.clone())
            });
        let Some(sid) = sid else {
            self.error = Some("Open a terminal first, then send the hello test.".into());
            return;
        };
        let label = self
            .state
            .sessions
            .iter()
            .find(|s| s.id == sid)
            .map(|s| s.label.clone())
            .unwrap_or_default();
        for event in notify_test::hello_events(&sid) {
            self.send(Request::Hook(event));
        }
        let mut note = format!(
            "Sent hello from {} agents to '{label}'. Check the Agents inbox.",
            terminator_integrations::AGENTS.len()
        );
        if !self
            .state
            .settings
            .events
            .contains(&AgentState::WaitingInput)
        {
            note.push_str(
                " Waiting input is off in saved Notifications → Events, so enable it and Apply.",
            );
        } else if !(self.state.settings.ntfy_enabled
            && !self.state.settings.ntfy_channel.is_empty())
        {
            note.push_str(" ntfy is off in saved settings, so phones stay silent.");
        }
        self.info = Some(note);
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
                        terminator_integrations::settings_path(
                            Path::new(&std::env::var("HOME").unwrap_or_default()),
                            kind,
                        )
                        .display()
                        .to_string(),
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
        ui.add_space(8.0);
        if self.field_visible("hello test", "hello test verify notification") {
            ui.weak("Send a hello from every agent to a live terminal. Uses saved settings; Apply notification changes first.");
            ui.horizontal(|ui| {
                let hello = ui.button("Send hello from all agents");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "hook-send-hello", hello.rect);
                if hello.clicked() {
                    self.send_hello_test();
                }
                let copy = ui.button("Copy hello command");
                #[cfg(feature = "test-support")]
                diagnostics::record(ui.ctx(), "hook-copy-hello", copy.rect);
                if copy.clicked() {
                    let helper = std::env::current_exe()
                        .map(|exe| exe.with_file_name("terminator-hook").display().to_string())
                        .unwrap_or_else(|_| "terminator-hook".into());
                    ui.ctx().copy_text(notify_test::hello_command(&helper));
                    self.info =
                        Some("Hello command copied. Paste it inside a Terminator terminal.".into());
                }
            });
        }
    }
}
