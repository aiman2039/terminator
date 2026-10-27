use super::*;
impl App {
    pub(crate) fn vnc_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("VNC Connections");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if appearance::sidebar_action(ui, "Plus", "New VNC connection").clicked() {
                    self.vnc_sidebar.begin_new();
                }
                if appearance::sidebar_action(ui, "Monitor", "Open VNC tab").clicked() {
                    self.open_new_vnc_tab(None);
                }
            });
        });
        ui.add_space(4.0);

        if !self.vnc_supported() {
            ui.colored_label(
                appearance::color(&self.theme.status_waiting),
                "Restart the session service to enable daemon VNC sessions.",
            );
            self.restart_session_button(ui, true);
            return;
        }

        if self.vnc_sidebar.form_open {
            egui::ScrollArea::vertical()
                .id_salt("vnc-profile-form")
                .max_height((ui.available_height() * 0.75).max(40.0))
                .show(ui, |ui| self.vnc_profile_form(ui));
            ui.separator();
        }

        ui.horizontal(|ui| {
            let response = ui.add(
                appearance::singleline(&mut self.vnc_sidebar.filter)
                    .hint_text("Filter connections…")
                    .desired_width((ui.available_width() - 28.0).max(60.0)),
            );
            let _ = response;
            if !self.vnc_sidebar.filter.is_empty()
                && appearance::sidebar_action(ui, "X", "Clear filter").clicked()
            {
                self.vnc_sidebar.filter.clear();
            }
        });
        ui.add_space(6.0);

        let filter = self.vnc_sidebar.filter.trim().to_lowercase();
        let profiles: Vec<VncProfile> = self
            .state
            .vnc_profiles
            .iter()
            .filter(|p| {
                filter.is_empty()
                    || p.name.to_lowercase().contains(&filter)
                    || p.host.to_lowercase().contains(&filter)
            })
            .cloned()
            .collect();

        let live_sessions: Vec<VncSession> = self.state.vnc_sessions.clone();

        appearance::sidebar_scroll("vnc-sidebar").show(ui, |ui| {
            if !live_sessions.is_empty() {
                ui.weak("Active sessions");
                for session in &live_sessions {
                    ui.horizontal(|ui| {
                        let status_color = match session.status {
                            VncSessionStatus::Connected => {
                                appearance::color(&self.theme.status_running)
                            }
                            VncSessionStatus::Connecting => {
                                appearance::color(&self.theme.status_waiting)
                            }
                            VncSessionStatus::Disconnected | VncSessionStatus::Failed => {
                                appearance::color(&self.theme.status_failed)
                            }
                        };
                        ui.colored_label(status_color, "●");
                        if ui
                            .selectable_label(false, &session.label)
                            .on_hover_text(format!(
                                "{}:{} ({})",
                                session.host,
                                session.port,
                                session.status.label()
                            ))
                            .clicked()
                        {
                            self.focus_or_open_vnc_session(session);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if appearance::sidebar_action(ui, "CircleStop", "Disconnect").clicked()
                            {
                                self.send(Request::VncDisconnect {
                                    session: session.id.clone(),
                                });
                            }
                        });
                    });
                }
                ui.add_space(8.0);
            }

            ui.weak("Saved hosts");
            if profiles.is_empty() {
                ui.weak("No saved VNC connections. Click + to add a host.");
            }
            for profile in &profiles {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        let connect_click = ui
                            .add(
                                egui::Button::new(RichText::new(profile.display_name()).strong())
                                    .frame(false),
                            )
                            .on_hover_text("Click to connect");
                        if connect_click.clicked() {
                            self.connect_vnc_profile(profile, None);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if appearance::sidebar_action(ui, "Trash", "Remove host").clicked() {
                                self.vnc_sidebar.confirm_remove = Some(profile.id.clone());
                            }
                            if appearance::sidebar_action(ui, "Pencil", "Edit host").clicked() {
                                self.vnc_sidebar.begin_edit(profile);
                            }
                            if appearance::sidebar_action(ui, "Play", "One-click connect").clicked()
                            {
                                self.connect_vnc_profile(profile, None);
                            }
                        });
                    });
                    if self.vnc_sidebar.confirm_remove.as_deref() == Some(profile.id.as_str()) {
                        ui.horizontal(|ui| {
                            ui.label("Remove this host?");
                            if ui.button("Remove").clicked() {
                                self.vnc_sidebar.confirm_remove = None;
                                self.send(Request::VncProfileRemove {
                                    id: profile.id.clone(),
                                });
                            }
                            if ui.button("Cancel").clicked() {
                                self.vnc_sidebar.confirm_remove = None;
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        ui.weak(profile.endpoint());
                        if profile.view_only {
                            ui.weak("· View only");
                        }
                        if !profile.password.is_empty() {
                            ui.weak("· Key saved");
                        }
                    });
                });
                ui.add_space(4.0);
            }
        });
    }
}
