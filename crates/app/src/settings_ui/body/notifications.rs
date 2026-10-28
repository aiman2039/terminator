use crate::*;

impl App {
    pub(super) fn notification_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Notifications");
        self.notification_events_grid(ui);
        self.ntfy_settings(ui);
        self.dismissal_row(ui);
        ui.checkbox(
            &mut self.settings_draft.notifications_side,
            "Place notifications at the side",
        );
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
        if !self.field_visible("ntfy", "channel machine phone push") {
            return;
        }
        // Clearing the channel is the removal gesture: an enabled toggle
        // with no channel can never validate, so switch the toggle off
        // instead of trapping Apply behind an error.
        if self.settings_draft.ntfy_channel.is_empty() {
            self.settings_draft.ntfy_enabled = false;
        }
        self.ntfy_fields(ui);
        if !self.state.capabilities.iter().any(|c| c == NTFY_CAPABILITY) {
            ui.small("Activate an updated daemon to enable ntfy. Older sessions need a supporting daemon.");
        }
        ui.horizontal(|ui| self.ntfy_test_buttons(ui));
        ui.small("Test posts directly to ntfy.sh with this channel; no Apply needed. Real alerts need the toggle on plus Apply, and no hook reinstall.");
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

    fn terminal_notice_toggles(&mut self, ui: &mut egui::Ui) {
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
}
