use crate::*;
use settings_controls::{FIELD_WIDTH, SettingsRowInput, settings_row};

/// Why a control gated on the session service is disabled.
const OLDER_DAEMON_HINT: &str = settings_controls::NEEDS_SERVICE;

impl App {
    pub(super) fn notification_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Notifications");
        let events = self.field_visible("Events", "waiting permission completed failed");
        let sound = self.field_visible("Play sound with desktop notifications", "sound alert");
        let dismiss = self.field_visible("Dismiss notifications", "focus resolve manual");
        let side = self.field_visible("Place notifications at the side", "position side corner");
        let ntfy = self.field_visible(
            "ntfy",
            "channel machine phone push server token click notice text",
        );
        let terminal = self.field_visible("Terminal notifications", "osc terminal desktop");
        if events || sound {
            settings_controls::sub_heading(ui, "Agent alerts");
            if events {
                self.notification_event_rows(ui);
            }
            if sound {
                self.notification_sound_row(ui);
            }
        }
        if dismiss || side {
            settings_controls::sub_heading(ui, "In-app");
            if dismiss {
                self.dismissal_row(ui);
            }
            if side {
                self.notification_side_row(ui);
            }
        }
        if ntfy {
            settings_controls::sub_heading(ui, "ntfy push");
            self.ntfy_settings(ui);
        }
        if terminal {
            settings_controls::sub_heading(ui, "Terminal");
            self.terminal_notice_toggles(ui);
        }
    }

    /// One row per agent event, with the In app and OS checkboxes in the
    /// control column so both columns line up across rows.
    fn notification_event_rows(&mut self, ui: &mut egui::Ui) {
        for state in [
            AgentState::WaitingInput,
            AgentState::WaitingPermission,
            AgentState::Completed,
            AgentState::Failed,
        ] {
            let draft = &mut self.settings_draft;
            settings_row(SettingsRowInput {
                ui,
                label: state.label(),
                description: "",
                add_control: |ui| {
                    ui.horizontal(|ui| {
                        for (events, label) in [
                            (&mut draft.events, "In app"),
                            (&mut draft.os_events, "OS when unfocused"),
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
                    });
                },
            });
        }
    }

    fn notification_side_row(&mut self, ui: &mut egui::Ui) {
        settings_row(SettingsRowInput {
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

    fn ntfy_settings(&mut self, ui: &mut egui::Ui) {
        // Clearing the channel is the removal gesture: an enabled toggle
        // with no channel can never validate, so switch the toggle off
        // instead of trapping Apply behind an error.
        if self.settings_draft.ntfy_channel.is_empty() {
            self.settings_draft.ntfy_enabled = false;
        }
        let ntfy = self.state.capabilities.iter().any(|c| c == NTFY_CAPABILITY);
        ui.add_enabled_ui(ntfy, |ui| self.ntfy_fields(ui))
            .response
            .on_disabled_hover_text(OLDER_DAEMON_HINT);
        self.ntfy_server_fields(ui);
        if !ntfy {
            settings_controls::control_column(ui, |ui| self.service_caption(ui, ntfy));
        }
        settings_row(SettingsRowInput {
            ui,
            label: "Test",
            description: "Test posts directly to the server with this channel and no token; no Apply needed. Real alerts need the toggle on plus Apply, and no hook reinstall.",
            add_control: |ui| {
                ui.horizontal(|ui| self.ntfy_test_buttons(ui));
            },
        });
    }

    fn ntfy_fields(&mut self, ui: &mut egui::Ui) {
        let draft = &mut self.settings_draft;
        settings_row(SettingsRowInput {
            ui,
            label: "Send to ntfy",
            description: "Sends the In app events above, even while focused or the GUI is closed. Sends only agent and status, not prompt text.",
            add_control: |ui| {
                ui.checkbox(&mut draft.ntfy_enabled, "Send agent notifications to ntfy");
            },
        });
        settings_row(SettingsRowInput {
            ui,
            label: "Channel (ntfy.sh)",
            description: "Subscribe to this channel in ntfy.",
            add_control: |ui| {
                ui.add(appearance::singleline(&mut draft.ntfy_channel).desired_width(FIELD_WIDTH));
            },
        });
        settings_row(SettingsRowInput {
            ui,
            label: "Machine name",
            description: "",
            add_control: |ui| {
                ui.add(appearance::singleline(&mut draft.ntfy_machine).desired_width(FIELD_WIDTH));
            },
        });
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
            self.ntfy_server_rows(ui);
            ntfy_token_row(ui, &mut token, &mut save);
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

    fn ntfy_server_rows(&mut self, ui: &mut egui::Ui) {
        let options = &mut self.settings_draft.ntfy_options;
        settings_row(SettingsRowInput {
            ui,
            label: "Server",
            description: "Blank uses ntfy.sh.",
            add_control: |ui| {
                ui.add(
                    appearance::singleline(&mut options.server)
                        .hint_text("https://ntfy.sh")
                        .desired_width(FIELD_WIDTH),
                );
            },
        });
        settings_row(SettingsRowInput {
            ui,
            label: "Click opens",
            description: "",
            add_control: |ui| {
                ui.add(
                    appearance::singleline(&mut options.click)
                        .hint_text("Optional URL")
                        .desired_width(FIELD_WIDTH),
                );
            },
        });
        settings_row(SettingsRowInput {
            ui,
            label: "Notice text",
            description: "The notice text can contain the agent's question.",
            add_control: |ui| {
                ui.checkbox(&mut options.include_text, "Include the notice text");
            },
        });
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
        settings_row(SettingsRowInput {
            ui,
            label: "Dismiss notifications",
            description: "Opening the terminal clears the alert, or it stays until the agent continues, or until you dismiss it.",
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
        let enabled = self
            .state
            .capabilities
            .iter()
            .any(|c| c == NOTIFICATION_SOUND_CAPABILITY);
        settings_row(SettingsRowInput {
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
        let enabled = self
            .state
            .capabilities
            .iter()
            .any(|c| c == TERMINAL_NOTICES_CAPABILITY);
        let draft = &mut self.settings_draft;
        settings_row(SettingsRowInput {
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

/// The access token field with Save token and Remove token. A saved or
/// removed token lands in `save`; an empty string removes it.
fn ntfy_token_row(ui: &mut egui::Ui, token: &mut String, save: &mut Option<String>) {
    settings_row(SettingsRowInput {
        ui,
        label: "Access token",
        description: "Kept in the OS credential store, not in the settings file.",
        add_control: |ui| {
            // The field gives up width so both buttons stay on its line.
            let buttons = settings_controls::button_width(ui, "Save token")
                + settings_controls::button_width(ui, "Remove token")
                + 2.0 * ui.spacing().item_spacing.x;
            let field = (ui.available_width() - buttons).clamp(120.0, FIELD_WIDTH);
            ui.horizontal_wrapped(|ui| {
                ui.add(
                    appearance::singleline(token)
                        .password(true)
                        .desired_width(field),
                );
                if ui
                    .add_enabled(!token.is_empty(), egui::Button::new("Save token"))
                    .clicked()
                {
                    *save = Some(std::mem::take(token));
                }
                if ui.button("Remove token").clicked() {
                    *save = Some(String::new());
                }
            });
        },
    });
}
