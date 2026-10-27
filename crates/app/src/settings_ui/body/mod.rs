mod hooks;
mod notifications;
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
        for action in Action::ALL {
            let (action, label) = (action.id(), action.label());
            if !self.field_visible(label, action) {
                continue;
            }
            let binding = self
                .settings_draft
                .keybindings
                .entry(action.into())
                .or_default()
                .clone();
            let capturing = self.shortcut_capture.as_deref() == Some(action);
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
                    self.shortcut_capture = Some(action.into());
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
}
