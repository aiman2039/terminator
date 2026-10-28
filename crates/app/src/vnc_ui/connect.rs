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

impl App {
    /// The direct connection form shown while the session is not live.
    /// Saved hosts live in the VNC sidebar; repeating them here duplicated
    /// every row and its two controls, so this pane only carries the form.
    pub(super) fn vnc_connect_form(
        &mut self,
        ui: &mut egui::Ui,
        state_key: &str,
        features: bool,
    ) -> Option<PendingVncConnect> {
        let error_color = appearance::color(&self.theme.status_failed);
        let view = self.vnc_views.entry(state_key.to_owned()).or_default();
        if let Some(error) = &view.error {
            ui.colored_label(error_color, error);
            ui.add_space(4.0);
        }
        ui.vertical(|ui| {
            ui.strong("Connect to VNC Server");
            ui.add_space(4.0);
            let request = ui
                .horizontal_wrapped(|ui| VncConnectForm::endpoint_row(ui, view))
                .inner;
            ui.horizontal_wrapped(|ui| VncConnectForm::label_row(ui, view));
            settings::options(ui, &mut view.draft_options, features);
            request
        })
        .inner
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
        if request.options != terminator_core::vnc::VncOptions::default() && !features {
            self.error = Some("Restart the session service to use advanced VNC options".into());
            return;
        }
        host.clone_from(&request.host);
        *port = request.port;
        if let Some(l) = &request.label {
            label.clone_from(l);
        }
        let effective_profile_id = if request.save_profile && request.profile_id.is_none() {
            Some(self.save_connect_profile(&request, label))
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
    fn save_connect_profile(&mut self, request: &PendingVncConnect, name: &str) -> String {
        let pid = id();
        let profile = VncProfile {
            options: request.options.clone(),
            credential_saved: false,
            id: pid.clone(),
            name: name.to_owned(),
            host: request.host.clone(),
            port: request.port,
            username: request.username.clone(),
            password: String::new(),
            shared: request.shared,
            view_only: request.view_only,
            last_connected: 0,
        };
        self.send(Request::VncProfileSave {
            profile,
            forget_password: false,
        });
        pid
    }
}

/// Rows of the direct connection form.
struct VncConnectForm;

impl VncConnectForm {
    fn endpoint_row(ui: &mut egui::Ui, view: &mut VncViewState) -> Option<PendingVncConnect> {
        ui.label("Host:");
        let host_resp = ui.add(
            appearance::singleline(&mut view.draft_host)
                .hint_text("Host name or IP address")
                .desired_width(200.0),
        );
        let endpoint_lost_focus =
            port_display_fields(ui, &mut view.draft_port, &mut view.draft_display);
        ui.label("Username:");
        ui.add(
            appearance::singleline(&mut view.draft_username)
                .hint_text("Optional")
                .desired_width(130.0),
        );
        ui.label("Password:");
        let pass_resp = ui.add(
            appearance::singleline(&mut view.draft_password)
                .password(true)
                .hint_text("VNC password")
                .desired_width(150.0),
        );
        let enter = (host_resp.lost_focus() || endpoint_lost_focus || pass_resp.lost_focus())
            && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if !(ui.button("Connect").clicked() || enter) {
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
            save_profile: view.save_profile,
            options: view.draft_options.clone(),
        })
    }

    fn label_row(ui: &mut egui::Ui, view: &mut VncViewState) {
        ui.label("Label:");
        ui.add(
            appearance::singleline(&mut view.draft_name)
                .hint_text("Optional name")
                .desired_width(160.0),
        );
        ui.checkbox(&mut view.draft_shared, "Shared");
        ui.checkbox(&mut view.draft_view_only, "View only");
        ui.checkbox(&mut view.save_profile, "Save to VNC sidebar");
    }
}
