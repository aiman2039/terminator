use super::*;
impl App {
    pub(super) fn vnc_profile_form(&mut self, ui: &mut egui::Ui) {
        let features = self.vnc_features_supported();
        if !features {
            self.vnc_sidebar.save_password = false;
        }
        ui.group(|ui| {
            ui.strong(if self.vnc_sidebar.editing_id.is_some() {
                "Edit VNC Host"
            } else {
                "New VNC Host"
            });
            ui.add_space(4.0);
            if let Some(error) = &self.vnc_sidebar.error {
                ui.colored_label(appearance::color(&self.theme.status_failed), error);
            }
            ui.label("Name");
            ui.add(
                appearance::singleline(&mut self.vnc_sidebar.name)
                    .hint_text("Build Mac / Linux VM")
                    .desired_width(f32::INFINITY),
            );
            ui.label("Host");
            ui.add(
                appearance::singleline(&mut self.vnc_sidebar.host)
                    .hint_text("127.0.0.1")
                    .desired_width(f32::INFINITY),
            );
            ui.horizontal(|ui| {
                port_display_fields(
                    ui,
                    &mut self.vnc_sidebar.port,
                    &mut self.vnc_sidebar.display,
                    true,
                );
            });
            ui.label("Username (optional)");
            ui.add(
                appearance::singleline(&mut self.vnc_sidebar.username).desired_width(f32::INFINITY),
            );
            ui.label("Password");
            ui.horizontal(|ui| {
                let show = self.vnc_sidebar.show_password;
                ui.add(
                    appearance::singleline(&mut self.vnc_sidebar.password)
                        .password(!show)
                        .desired_width((ui.available_width() - 32.0).max(60.0)),
                );
                let eye = if show { "EyeOff" } else { "Eye" };
                if appearance::sidebar_action(ui, eye, "Toggle password visibility").clicked() {
                    self.vnc_sidebar.show_password = !self.vnc_sidebar.show_password;
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.vnc_sidebar.shared, "Shared session");
                ui.checkbox(&mut self.vnc_sidebar.view_only, "View only");
            });
            ui.add_enabled(
                features,
                egui::Checkbox::new(
                    &mut self.vnc_sidebar.save_password,
                    "Save password in the OS keychain",
                ),
            )
            .on_hover_text("Off: the password is only used for this session");
            settings::options(ui, &mut self.vnc_sidebar.options, features);
            ui.horizontal_wrapped(|ui| {
                let save = ui.button("Save").clicked();
                let save_connect = ui.button("Save & Connect").clicked();
                if ui.button("Cancel").clicked() {
                    self.vnc_sidebar.password.clear();
                    self.vnc_sidebar.error = None;
                    self.vnc_sidebar.form_open = false;
                }
                if save || save_connect {
                    self.save_vnc_profile_form(save_connect);
                }
            });
        });
    }

    pub(crate) fn save_vnc_profile_form(&mut self, connect: bool) {
        let result = (|| -> Result<VncProfile> {
            let (host, port) = connection_endpoint(&self.vnc_sidebar.host, &self.vnc_sidebar.port)?;
            let profile = VncProfile {
                options: self.vnc_sidebar.options.clone(),
                credential_saved: false,
                id: self.vnc_sidebar.editing_id.clone().unwrap_or_else(id),
                name: self.vnc_sidebar.name.trim().to_owned(),
                host,
                port,
                username: self.vnc_sidebar.username.trim().to_owned(),
                password: self.vnc_sidebar.password.clone(),
                shared: self.vnc_sidebar.shared,
                view_only: self.vnc_sidebar.view_only,
                last_connected: 0,
            };
            profile.validate()?;
            if !self.vnc_features_supported() {
                ensure!(
                    !self
                        .state
                        .vnc_profiles
                        .iter()
                        .any(|p| p.id == profile.id && !p.password.is_empty()),
                    "Restart the session service before editing profiles that contain legacy saved passwords"
                );
            }
            ensure!(
                self.vnc_features_supported()
                    || profile.options == terminator_core::vnc::VncOptions::default(),
                "Restart the session service to use advanced VNC options"
            );
            ensure!(
                self.vnc_features_supported() || !self.vnc_sidebar.save_password,
                "Restart the session service to save passwords securely"
            );
            Ok(profile)
        })();
        match result {
            Ok(profile) => {
                let mut saved = profile.clone();
                if !self.vnc_sidebar.save_password {
                    saved.password.clear();
                }
                self.send(Request::VncProfileSave {
                    profile: saved,
                    forget_password: !self.vnc_sidebar.save_password,
                });
                if connect {
                    self.connect_vnc_profile(&profile, None);
                }
                self.vnc_sidebar.password.clear();
                self.vnc_sidebar.error = None;
                self.vnc_sidebar.form_open = false;
            }
            Err(error) => self.vnc_sidebar.error = Some(error.to_string()),
        }
    }
}
