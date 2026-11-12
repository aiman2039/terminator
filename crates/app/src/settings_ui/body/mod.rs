mod agents;
mod browser;
mod editor;
mod fonts;
mod hooks;
mod languages;
mod notifications;
mod ssh;
mod terminal;
mod theme;

use super::state::{SettingsFrame, SettingsSection};
use crate::*;

impl App {
    pub(super) fn paint_settings_body(
        &mut self,
        ui: &mut egui::Ui,
        height: f32,
        frame: &mut SettingsFrame,
    ) {
        let section = self.settings_section.canonical();
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            if self.settings_search_empty() {
                self.paint_settings_no_match(ui);
                return;
            }
            egui::ScrollArea::vertical()
                .id_salt(("settings-section", section as u8))
                .max_height(height)
                .auto_shrink([false, false])
                .show(ui, |ui| match section {
                    SettingsSection::Appearance => self.appearance_settings(ui),
                    SettingsSection::Terminal => {
                        self.terminal_settings(ui, &mut frame.grant_folder, &mut frame.browse);
                    }
                    SettingsSection::Editor => self.editor_settings(ui, &mut frame.browse),
                    SettingsSection::Languages => self.language_settings(ui, &mut frame.browse),
                    SettingsSection::Git => self.git_settings(ui),
                    SettingsSection::Browser => {
                        ui.heading("Browser");
                        self.browser_settings_rows(ui);
                    }
                    SettingsSection::Keymap => self.keymap_settings(ui),
                    SettingsSection::Notifications => self.notification_settings(ui),
                    SettingsSection::History => self.history_settings(ui),
                    SettingsSection::Agents | SettingsSection::AgentHooks => {
                        self.agent_settings(ui);
                        ui.add_space(12.0);
                        self.hook_settings(ui);
                    }
                    SettingsSection::Remote => {
                        self.ssh_settings(ui);
                        settings_controls::sub_heading(ui, "VNC");
                        self.vnc_detach_setting(ui);
                    }
                    SettingsSection::Updates => {
                        self.installation_settings(ui);
                        ui.add_space(12.0);
                        ui.separator();
                        #[cfg(any(not(target_os = "macos"), test))]
                        self.windows_updates.settings(
                            ui,
                            &mut self.settings_draft.automatic_update_checks,
                            &mut self.preferences.skipped_update_version,
                        );
                        #[cfg(target_os = "macos")]
                        self.updater.settings(ui);
                    }
                });
        });
    }

    fn history_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("History");
        if self.field_visible("Days", "age retain") {
            settings_controls::settings_row(settings_controls::SettingsRowInput {
                ui,
                label: "Days",
                description: "How long saved output is kept.",
                add_control: |ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.settings_draft.history_days).range(1..=3650),
                    );
                },
            });
        }
        if self.field_visible("MiB per session", "disk") {
            settings_controls::settings_row(settings_controls::SettingsRowInput {
                ui,
                label: "MiB per session",
                description: "Cap for one session's saved output.",
                add_control: |ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.settings_draft.session_mib).range(1..=4096),
                    );
                },
            });
        }
        if self.field_visible("MiB total", "disk") {
            settings_controls::settings_row(settings_controls::SettingsRowInput {
                ui,
                label: "MiB total",
                description: "Cap across all sessions.",
                add_control: |ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.settings_draft.total_mib).range(1..=65536),
                    );
                },
            });
        }
        settings_controls::description_line(
            ui,
            "Limits apply to saved output; records and resume commands remain.",
        );
        ui.add_space(10.0);
        self.local_history_setting(ui);
    }

    /// How long Local History keeps file snapshots (`local-history-v1`).
    fn local_history_setting(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("Local History days", "snapshots files revert age") {
            return;
        }
        let supported = self.local_history_supported();
        ui.add_enabled_ui(supported, |ui| {
            settings_controls::settings_row(settings_controls::SettingsRowInput {
                ui,
                label: "Local History days",
                description: "How long file snapshots are kept. 0 stops recording.",
                add_control: |ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.settings_draft.local_history_days)
                            .range(0..=3650),
                    );
                },
            });
        })
        .response
        .on_disabled_hover_text("Requires an updated daemon.");
    }

    /// How long a VNC session with no open tab stays connected.
    fn vnc_detach_setting(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("Detached VNC sessions", "vnc timeout minutes disconnect") {
            return;
        }
        let supported = self
            .state
            .capabilities
            .iter()
            .any(|c| c == terminator_core::vnc::DETACH_TIMEOUT_CAPABILITY);
        ui.add_enabled_ui(supported, |ui| {
            settings_controls::settings_row(settings_controls::SettingsRowInput {
                ui,
                label: "Detached VNC sessions",
                description: "Disconnect after this many minutes without a viewer. 0 keeps them connected.",
                add_control: |ui| {
                    ui.add(
                        egui::DragValue::new(&mut self.settings_draft.vnc_detached_timeout_minutes)
                            .range(0..=1440)
                            .suffix(" min"),
                    );
                },
            });
        })
        .response
        .on_disabled_hover_text("Requires an updated daemon; older ones disconnect after 30 minutes.");
    }

    /// The JetBrains keymap switch, then every shortcut.
    fn keymap_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Keymap");
        self.editor_keymap_setting(ui);
        settings_controls::sub_heading(ui, "Shortcuts");
        self.shortcut_settings(ui);
    }

    fn shortcut_settings(&mut self, ui: &mut egui::Ui) {
        ui.weak("Click a row, then press the keys. Esc cancels capture.");
        if ui.button("Reset to defaults").clicked() {
            self.settings_draft.keybindings = Settings::default().keybindings;
            self.shortcut_capture = None;
        }
        ui.add_space(8.0);
        for action in Action::ALL {
            let (action, label) = (action.id(), action.label());
            if !self.field_visible(label, action) {
                continue;
            }
            // Read only: an unbound action without a default (the Git
            // ones) must not add a key, or the draft looks changed.
            let binding = self
                .settings_draft
                .keybindings
                .get(action)
                .cloned()
                .unwrap_or_default();
            let capturing = self.shortcut_capture.as_deref() == Some(action);
            let shown = if capturing {
                "Press keys…".into()
            } else if binding.is_empty() {
                "Unbound".into()
            } else {
                shortcuts::display(&binding)
            };
            settings_controls::settings_row(settings_controls::SettingsRowInput {
                ui,
                label,
                description: action,
                add_control: |ui| {
                    let button = ui.add_sized([180.0, 28.0], egui::Button::new(shown));
                    #[cfg(feature = "test-support")]
                    diagnostics::record(ui.ctx(), &format!("shortcut:{action}"), button.rect);
                    if button.clicked() {
                        self.shortcut_capture = Some(action.into());
                    }
                },
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
}
