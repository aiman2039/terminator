use super::*;

/// The tab fields a started connection rewrites.
pub(super) struct VncConnectTab<'a> {
    pub(super) tab_id: &'a str,
    pub(super) profile_id: &'a mut Option<String>,
    pub(super) host: &'a mut String,
    pub(super) port: &'a mut u16,
    pub(super) label: &'a mut String,
}

pub(super) struct StartVncConnect<'a> {
    pub(super) request: PendingVncConnect,
    pub(super) project: String,
    pub(super) features: bool,
    pub(super) tab: VncConnectTab<'a>,
}

/// What the not-yet-connected pane needs to draw itself.
pub(super) struct VncConnectPage<'a> {
    pub(super) state_key: &'a str,
    pub(super) features: bool,
    pub(super) profile_id: Option<&'a str>,
}

impl App {
    /// The pane while no session is live: the address book, or a password
    /// prompt when a saved host without a stored password was opened.
    pub(super) fn vnc_connect_page(
        &mut self,
        ui: &mut egui::Ui,
        page: VncConnectPage<'_>,
    ) -> Option<PendingVncConnect> {
        let VncConnectPage {
            state_key,
            features,
            profile_id,
        } = page;
        let prompt = self
            .vnc_views
            .get(state_key)
            .is_some_and(|view| !view.save_profile);
        if !prompt {
            self.vnc_address_book(ui, state_key);
            return None;
        }
        let profile =
            profile_id.and_then(|id| self.state.vnc_profiles.iter().find(|p| p.id == id).cloned());
        let view = self.vnc_views.entry(state_key.to_owned()).or_default();
        let mut request = form::card(ui, |ui| PasswordPrompt::show(ui, view))?;
        request.profile_id = profile_id.map(str::to_owned);
        if let Some(mut profile) = profile
            && view.draft_remember
            && features
            && !request.password.is_empty()
        {
            profile.password.clone_from(&request.password);
            self.send(Request::VncProfileSave {
                profile,
                forget_password: false,
            });
        }
        Some(request)
    }

    pub(super) fn start_vnc_connect(&mut self, input: StartVncConnect<'_>) {
        let StartVncConnect {
            request,
            project,
            features,
            tab,
        } = input;
        let VncConnectTab {
            tab_id,
            profile_id,
            host,
            port,
            label,
        } = tab;
        if let Some(error) = self.vnc_options_unsupported(&request.options, features) {
            self.error = Some(error.into());
            return;
        }
        host.clone_from(&request.host);
        *port = request.port;
        if let Some(l) = &request.label {
            label.clone_from(l);
        }
        let effective_profile_id = if request.save_profile && request.profile_id.is_none() {
            Some(self.save_connect_profile(&request, label, features))
        } else {
            request.profile_id.clone()
        };
        profile_id.clone_from(&effective_profile_id);
        let PendingVncConnect {
            profile_id: _,
            host,
            port,
            username,
            password,
            shared,
            view_only,
            label,
            save_profile: _,
            options,
        } = request;
        let _ = self.jobs.send(Job::VncConnectTab {
            tab_id: tab_id.to_owned(),
            request: Request::VncConnect {
                options,
                project,
                profile_id: effective_profile_id,
                host,
                port,
                username,
                password,
                shared,
                view_only,
                label,
            },
        });
    }

    /// Save the host being connected to the sidebar; returns the new id.
    fn save_connect_profile(
        &mut self,
        request: &PendingVncConnect,
        name: &str,
        features: bool,
    ) -> String {
        let (profile, forget_password) = connect_profile(request, name, features);
        let pid = profile.id.clone();
        self.send(Request::VncProfileSave {
            profile,
            forget_password,
        });
        pid
    }
}

/// The sidebar profile for a form connection, and whether to forget its
/// password. The password goes to the OS keychain only when the daemon
/// stores it securely (`features`); older daemons keep it in plaintext.
pub(super) fn connect_profile(
    request: &PendingVncConnect,
    name: &str,
    features: bool,
) -> (VncProfile, bool) {
    let save_password = features && !request.password.is_empty();
    let profile = VncProfile {
        options: request.options.clone(),
        credential_saved: false,
        labels: Vec::new(),
        id: id(),
        name: name.to_owned(),
        host: request.host.clone(),
        port: request.port,
        username: request.username.clone(),
        password: if save_password {
            request.password.clone()
        } else {
            String::new()
        },
        shared: request.shared,
        view_only: request.view_only,
        last_connected: 0,
    };
    (profile, !save_password)
}

/// Password entry for a saved host that has no stored password.
struct PasswordPrompt;

impl PasswordPrompt {
    fn show(ui: &mut egui::Ui, view: &mut VncViewState) -> Option<PendingVncConnect> {
        let title = if view.draft_name.trim().is_empty() {
            view.draft_host.clone()
        } else {
            view.draft_name.clone()
        };
        ui.label(
            RichText::new(format!("Connect to {title}"))
                .size(16.0)
                .strong(),
        );
        form::hint(ui, &format!("{}:{}", view.draft_host, view.draft_port), "");
        form::section(ui, "Sign in");
        form::text(ui, "Username", &mut view.draft_username, "Optional");
        form::label(ui, "Password");
        let field = form::password(
            ui,
            &mut view.draft_password,
            &mut view.draft_show_password,
            false,
        );
        if !field.has_focus() && view.draft_password.is_empty() {
            field.request_focus();
        }
        let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.checkbox(&mut view.draft_remember, "Remember password")
            .on_hover_text("Saved in the OS keychain");
        ui.add_space(8.0);
        ui.separator();
        let (connect, back) = ui
            .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let connect = form::primary_button(ui, "Connect").clicked();
                let back = ui.button("All hosts").clicked();
                (connect, back)
            })
            .inner;
        if back {
            view.save_profile = true;
            view.draft_password.clear();
            return None;
        }
        if !(connect || enter) {
            return None;
        }
        Self::draft_request(view)
    }

    fn draft_request(view: &mut VncViewState) -> Option<PendingVncConnect> {
        let (parsed_host, parsed_port) =
            match connection_endpoint(&view.draft_host, &view.draft_port) {
                Ok(endpoint) => endpoint,
                Err(error) => {
                    view.error = Some(error.to_string());
                    return None;
                }
            };
        let title = if view.draft_name.trim().is_empty() {
            format!("{parsed_host}:{parsed_port}")
        } else {
            view.draft_name.trim().to_owned()
        };
        Some(PendingVncConnect {
            profile_id: None,
            host: parsed_host,
            port: parsed_port,
            username: view.draft_username.clone(),
            password: view.draft_password.clone(),
            shared: view.draft_shared,
            view_only: view.draft_view_only,
            label: Some(title),
            save_profile: false,
            options: view.draft_options.clone(),
        })
    }
}
