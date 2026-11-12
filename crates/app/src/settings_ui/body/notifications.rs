use crate::*;

/// Why a control gated on the session service is disabled.
const OLDER_DAEMON_HINT: &str = settings_controls::NEEDS_SERVICE;

impl App {
    pub(super) fn notification_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Notifications");
        self.notification_events_grid(ui);
        self.ntfy_settings(ui);
        self.dismissal_row(ui);
        if self.field_visible("Place notifications at the side", "position side corner") {
            settings_controls::settings_row(settings_controls::SettingsRowInput {
                ui,
                label: "Position",
                description: "Where in-app agent alerts show.",
                add_control: |ui| {
                    ui.checkbox(
                        &mut self.settings_draft.notifications_side,
                        "Place notifications at the side",
                    );
                },
            });
        }
        self.notification_sound_row(ui);
        self.terminal_notice_toggles(ui);
    }

    fn notification_events_grid(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("Events", "waiting permission completed failed") {
            return;
        }
        egui::Grid::new("notification-settings")
            .num_columns(3)
            .show(ui, |ui| {
                for state in [
                    AgentState::WaitingInput,
                    AgentState::WaitingPermission,
                    AgentState::Completed,
                    AgentState::Failed,
                ] {
                    self.notification_event_row(ui, state);
                }
            });
    }

    fn notification_event_row(&mut self, ui: &mut egui::Ui, state: AgentState) {
        ui.label(state.label());
        for (events, label) in [
            (&mut self.settings_draft.events, "In app"),
            (&mut self.settings_draft.os_events, "OS when unfocused"),
        ] {
            let mut enabled = events.contains(&state);
            if !ui.checkbox(&mut enabled, label).changed() {
                continue;
            }
            if enabled {
                events.insert(state);
            } else {
                events.remove(&state);
            }
        }
        ui.end_row();
    }

    fn ntfy_settings(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible(
            "ntfy",
            "channel machine phone push server token click notice text",
        ) {
            return;
        }
        // Clearing the channel is the removal gesture: an enabled toggle
        // with no channel can never validate, so switch the toggle off
        // instead of trapping Apply behind an error.
        if self.settings_draft.ntfy_channel.is_empty() {
            self.settings_draft.ntfy_enabled = false;
        }
        self.ntfy_fields(ui);
        self.ntfy_server_fields(ui);
        let ntfy = self.state.capabilities.iter().any(|c| c == NTFY_CAPABILITY);
        self.service_caption(ui, ntfy);
        ui.horizontal(|ui| self.ntfy_test_buttons(ui));
        ui.small("Test posts directly to the server with this channel and no token; no Apply needed. Real alerts need the toggle on plus Apply, and no hook reinstall.");
    }

    /// Self-hosted server, click action, notice text and access token
    /// (`ntfy-server-v1`). The token goes to the daemon, which keeps it in
    /// the OS credential store; it is never part of the settings.
    fn ntfy_server_fields(&mut self, ui: &mut egui::Ui) {
        let supported = self
            .state
            .capabilities
            .iter()
            .any(|c| c == terminator_core::NTFY_SERVER_CAPABILITY);
        let token_id = egui::Id::unique("ntfy-token-draft");
        let mut token = ui.data(|data| data.get_temp::<String>(token_id).unwrap_or_default());
        let mut save = None;
        ui.add_enabled_ui(supported, |ui| {
            let options = &mut self.settings_draft.ntfy_options;
            ui.horizontal(|ui| {
                ui.label("Server");
                ui.add(
                    egui::TextEdit::singleline(&mut options.server).hint_text("https://ntfy.sh"),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Click opens");
                ui.add(egui::TextEdit::singleline(&mut options.click).hint_text("Optional URL"));
            });
            ui.checkbox(
                &mut options.include_text,
                "Include the notice text (it can contain the agent's question)",
            );
            ui.horizontal(|ui| {
                ui.label("Access token");
                ui.add(egui::TextEdit::singleline(&mut token).password(true));
                if ui
                    .add_enabled(!token.is_empty(), egui::Button::new("Save token"))
                    .clicked()
                {
                    save = Some(std::mem::take(&mut token));
                }
                if ui.button("Remove token").clicked() {
                    save = Some(String::new());
                }
            });
        })
        .response
        .on_disabled_hover_text(OLDER_DAEMON_HINT);
        ui.data_mut(|data| data.insert_temp(token_id, token));
        if let Some(token) = save {
            self.info = Some(if token.is_empty() {
                "ntfy token removed.".into()
            } else {
                "ntfy token saved in the OS credential store.".into()
            });
            self.send(Request::NtfyToken { token });
        }
    }

    fn ntfy_fields(&mut self, ui: &mut egui::Ui) {
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
    }

    fn ntfy_test_buttons(&mut self, ui: &mut egui::Ui) {
        let channel = self.settings_draft.ntfy_channel.clone();
        let machine = self.settings_draft.ntfy_machine.clone();
        let ready = !channel.is_empty();
        let test = ui.add_enabled(ready, egui::Button::new("Send test"));
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "ntfy-send-test", test.rect);
        if test.clicked()
            && let Err(error) = self.send_ntfy_test(channel.clone(), machine.clone())
        {
            self.error.set(error);
        }
        let copy = ui.add_enabled(ready, egui::Button::new("Copy test command"));
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "ntfy-copy-test", copy.rect);
        if copy.clicked() {
            let endpoint = self.settings_draft.ntfy_endpoint();
            ui.ctx()
                .copy_text(notify_test::curl_command(&endpoint, &channel, &machine));
            self.info = Some("ntfy test command copied.".into());
        }
    }

    fn dismissal_row(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("Dismiss notifications", "focus resolve manual") {
            return;
        }
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Dismiss notifications",
            description: "When in-app attention is cleared.",
            add_control: |ui| {
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
        });
    }

    fn notification_sound_row(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("Play sound with desktop notifications", "sound alert") {
            return;
        }
        let enabled = self
            .state
            .capabilities
            .iter()
            .any(|c| c == NOTIFICATION_SOUND_CAPABILITY);
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Sound",
            description: "The in-app Agents inbox stays silent.",
            add_control: |ui| {
                ui.add_enabled(
                    enabled,
                    egui::Checkbox::new(
                        &mut self.settings_draft.notification_sound,
                        "Play sound with desktop notifications",
                    ),
                )
                .on_disabled_hover_text(OLDER_DAEMON_HINT);
            },
        });
    }

    fn terminal_notice_toggles(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("Terminal notifications", "osc terminal desktop") {
            return;
        }
        let enabled = self
            .state
            .capabilities
            .iter()
            .any(|c| c == TERMINAL_NOTICES_CAPABILITY);
        let draft = &mut self.settings_draft;
        settings_controls::settings_row(settings_controls::SettingsRowInput {
            ui,
            label: "Terminal notifications",
            description: "Notifications that programs in a terminal send with OSC escape codes.",
            add_control: |ui| {
                ui.add_enabled(
                    enabled,
                    egui::Checkbox::new(
                        &mut draft.terminal_notifications,
                        "Show terminal OSC notifications",
                    ),
                )
                .on_disabled_hover_text(OLDER_DAEMON_HINT);
                ui.add_enabled(
                    enabled,
                    egui::Checkbox::new(
                        &mut draft.terminal_notifications_os,
                        "Also send terminal notifications to the desktop",
                    ),
                )
                .on_disabled_hover_text(OLDER_DAEMON_HINT);
            },
        });
    }

    fn send_ntfy_test(&mut self, channel: String, machine: String) -> Result<(), String> {
        notify_test::validate(&channel, &machine)?;
        let endpoint = self.settings_draft.ntfy_endpoint();
        let _ = self.jobs.send(Job::TestNtfy {
            channel,
            machine,
            endpoint,
        });
        self.info = Some("Sending ntfy test…".into());
        Ok(())
    }
}
