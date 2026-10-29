use super::*;
impl App {
    /// Host Properties in the main VNC page: General, Security and
    /// Options pages, then Cancel / Save / Save & Connect.
    pub(super) fn vnc_properties(&mut self, ui: &mut egui::Ui) {
        let features = self.vnc_features_supported();
        if !features {
            self.vnc_sidebar.save_password = false;
        }
        let failed = appearance::color(&self.theme.status_failed);
        let accent = appearance::color(&self.theme.accent);
        form::card(ui, |ui| {
            let editor = &mut self.vnc_sidebar;
            let title = if editor.editing_id.is_some() {
                format!("{} – Properties", display_title(editor))
            } else {
                "New host".to_owned()
            };
            ui.label(RichText::new(title).size(16.0).strong());
            ui.add_space(6.0);
            form::segmented(
                ui,
                &mut editor.tab,
                &[
                    (PropertiesTab::General, "General"),
                    (PropertiesTab::Security, "Security"),
                    (PropertiesTab::Options, "Options"),
                ],
            );
            if let Some(notice) = editor.ssh_notice.clone() {
                Self::vnc_ssh_notice(ui, editor, &notice, accent);
            }
            match editor.tab {
                PropertiesTab::General => self.vnc_properties_general(ui, features),
                PropertiesTab::Security => {
                    settings::security(ui, &mut self.vnc_sidebar.options, features);
                }
                PropertiesTab::Options => {
                    settings::options(ui, &mut self.vnc_sidebar.options, features);
                }
            }
            if let Some(error) = &self.vnc_sidebar.error {
                ui.add_space(6.0);
                ui.colored_label(failed, error);
            }
            ui.add_space(8.0);
            ui.separator();
            self.vnc_properties_footer(ui);
        });
    }

    fn vnc_ssh_notice(
        ui: &mut egui::Ui,
        editor: &mut VncSidebarState,
        notice: &str,
        accent: egui::Color32,
    ) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.colored_label(accent, RichText::new(notice).size(11.0));
            if appearance::sidebar_action(ui, "X", "Dismiss").clicked() {
                editor.ssh_notice = None;
            }
        });
    }

    fn vnc_properties_general(&mut self, ui: &mut egui::Ui, features: bool) {
        let editor = &mut self.vnc_sidebar;
        form::section(ui, "Server");
        form::endpoint(ui, &mut editor.host, &mut editor.port, &mut editor.display);
        form::text(
            ui,
            "Name",
            &mut editor.name,
            "Optional, for example Build VM",
        );
        form::section(ui, "Sign in");
        form::text(ui, "Username", &mut editor.username, "Optional");
        form::label(ui, "Password");
        let saved = editor.credential_saved && !editor.forget_password;
        form::password(ui, &mut editor.password, &mut editor.show_password, saved);
        ui.horizontal(|ui| {
            ui.add_enabled(
                features,
                egui::Checkbox::new(&mut editor.save_password, "Remember password"),
            )
            .on_hover_text("Saved in the OS keychain. Off: used for this session only.");
            if saved && ui.small_button("Forget password").clicked() {
                editor.forget_password = true;
                editor.password.clear();
            }
        });
        form::section(ui, "Session");
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut editor.shared, "Shared")
                .on_hover_text("Keep other viewers connected");
            ui.checkbox(&mut editor.view_only, "View only");
        });
        self.vnc_apply_ssh_probe(ui.ctx());
    }

    /// For a new host, fill in an SSH tunnel when `~/.ssh/config`
    /// configures the typed address and the user has not chosen a transport.
    fn vnc_apply_ssh_probe(&mut self, ctx: &egui::Context) {
        let editor = &mut self.vnc_sidebar;
        if editor.editing_id.is_some() {
            return;
        }
        let found = editor.ssh_probe.poll(&editor.host);
        if editor.ssh_probe.busy() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        let Some(found) = found else {
            return;
        };
        let options = &mut editor.options;
        if options.transport == terminator_core::vnc::Transport::Direct
            && options.ssh.host.is_empty()
        {
            options.transport = terminator_core::vnc::Transport::Ssh;
            settings::enable_ssh_defaults(options);
            editor.ssh_notice = Some(found.summary());
        }
    }

    fn vnc_properties_footer(&mut self, ui: &mut egui::Ui) {
        let (save, save_connect, cancel) = ui
            .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let save_connect = form::primary_button(ui, "Save & Connect").clicked();
                let save = ui.button("Save").clicked();
                let cancel = ui.button("Cancel").clicked();
                (save, save_connect, cancel)
            })
            .inner;
        if cancel {
            self.close_vnc_properties();
        }
        if save || save_connect {
            self.save_vnc_profile_form(save_connect);
        }
    }

    pub(super) fn close_vnc_properties(&mut self) {
        self.vnc_sidebar.password.clear();
        self.vnc_sidebar.error = None;
        self.vnc_sidebar.form_open = false;
    }

    pub(crate) fn save_vnc_profile_form(&mut self, connect: bool) {
        let result = (|| -> Result<VncProfile> {
            let (host, port) = connection_endpoint(&self.vnc_sidebar.host, &self.vnc_sidebar.port)?;
            let mut options = self.vnc_sidebar.options.clone();
            // A blank SSH host tunnels to the address itself.
            if options.transport == terminator_core::vnc::Transport::Ssh
                && options.ssh.host.trim().is_empty()
            {
                options.ssh.host.clone_from(&host);
            }
            let profile = VncProfile {
                options,
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
            if let Some(error) =
                self.vnc_options_unsupported(&profile.options, self.vnc_features_supported())
            {
                anyhow::bail!(error);
            }
            ensure!(
                self.vnc_features_supported() || !self.vnc_sidebar.save_password,
                "Restart the session service to save passwords securely"
            );
            Ok(profile)
        })();
        match result {
            Ok(mut profile) => {
                let editor = &self.vnc_sidebar;
                let forget = !editor.save_password || editor.forget_password;
                let mut saved = profile.clone();
                if forget {
                    saved.password.clear();
                }
                self.send(Request::VncProfileSave {
                    profile: saved,
                    forget_password: forget,
                });
                // An empty field keeps the keychain password, so connecting
                // must not ask for it again.
                profile.credential_saved =
                    !forget && (editor.credential_saved || !profile.password.is_empty());
                if connect {
                    self.connect_vnc_profile(&profile, None);
                }
                self.close_vnc_properties();
            }
            Err(error) => self.vnc_sidebar.error = Some(error.to_string()),
        }
    }
}

/// Name shown in the Properties title: the name, else the address.
fn display_title(editor: &VncSidebarState) -> String {
    let name = editor.name.trim();
    if name.is_empty() {
        editor.host.trim().to_owned()
    } else {
        name.to_owned()
    }
}
