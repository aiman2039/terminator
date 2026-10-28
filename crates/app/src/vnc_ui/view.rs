use super::*;

/// The tab fields the VNC view reads while it lays out the toolbar.
pub(super) struct VncTabFields<'a> {
    pub(super) tab_id: &'a str,
    pub(super) session_id: &'a str,
    pub(super) profile_id: &'a Option<String>,
    pub(super) host: &'a str,
    pub(super) port: u16,
    pub(super) label: &'a str,
}

/// What the toolbar asked for this frame; applied after it is laid out.
#[derive(Default)]
pub(super) struct VncToolbarActions {
    pub(super) connect: Option<PendingVncConnect>,
    pub(super) special_keys: Vec<VncInputEvent>,
    pub(super) disconnect: bool,
    pub(super) refresh: bool,
    pub(super) ctrl_alt_del: bool,
    pub(super) save_to_sidebar: bool,
    pub(super) open_sidebar: bool,
}

struct VncToolbarRequest<'a> {
    state_key: &'a str,
    fields: &'a VncTabFields<'a>,
    session: Option<&'a VncSession>,
    features: bool,
}

struct VncActionsInput<'a> {
    actions: VncToolbarActions,
    session_id: &'a str,
    session: Option<&'a VncSession>,
    save_host: VncSaveHost<'a>,
}

struct VncSaveHost<'a> {
    state_key: &'a str,
    profile_id: &'a mut Option<String>,
    host: &'a str,
    port: u16,
    label: &'a str,
}

pub(super) struct VncLiveInput<'a> {
    pub(super) session_id: &'a str,
    pub(super) view_only: bool,
    pub(super) features: bool,
}

impl App {
    pub(crate) fn vnc_view(&mut self, ui: &mut egui::Ui, input: VncViewInput<'_>) {
        let VncViewInput {
            tab_id,
            session_id,
            profile_id,
            host,
            port,
            label,
        } = input;
        let daemon_session = self.vnc_daemon_session(session_id);
        let state_key = Self::vnc_state_key(tab_id, session_id);
        self.visible_vnc.insert(state_key.clone());
        let features = self.session_features(session_id);
        let fields = VncTabFields {
            tab_id,
            session_id,
            profile_id,
            host,
            port: *port,
            label,
        };
        let mut actions = self.vnc_toolbar(
            ui,
            VncToolbarRequest {
                state_key: &state_key,
                fields: &fields,
                session: daemon_session.as_ref(),
                features,
            },
        );
        let connect = actions.connect.take();
        self.apply_vnc_toolbar_actions(VncActionsInput {
            actions,
            session_id,
            session: daemon_session.as_ref(),
            save_host: VncSaveHost {
                state_key: &state_key,
                profile_id,
                host,
                port: *port,
                label,
            },
        });
        self.vnc_status_lines(ui, &state_key, daemon_session.as_ref());
        ui.separator();

        let is_live = daemon_session.as_ref().is_some_and(|s| s.status.live());
        let form_connect = (!is_live)
            .then(|| self.vnc_connect_form(ui, &state_key, features))
            .flatten();
        if let Some(request) = form_connect.or(connect)
            && let Some(project) = self.selected.clone()
        {
            self.start_vnc_connect(StartVncConnect {
                request,
                project,
                features,
                tab: VncConnectTab {
                    tab_id,
                    profile_id,
                    host,
                    port,
                    label,
                },
            });
            return;
        }
        if !is_live {
            return;
        }
        self.vnc_live_screen(
            ui,
            VncLiveInput {
                session_id,
                view_only: daemon_session.as_ref().is_some_and(|s| s.view_only),
                features,
            },
        );
    }

    fn vnc_daemon_session(&self, session_id: &str) -> Option<VncSession> {
        self.state
            .vnc_sessions
            .iter()
            .find(|s| s.id == session_id)
            .cloned()
    }

    /// View state is keyed by session, or by the tab while it is still a
    /// draft: the key doubles as the "seen this frame" marker that keeps
    /// the pruner from dropping live view state.
    fn vnc_state_key(tab_id: &str, session_id: &str) -> String {
        if session_id.is_empty() {
            format!("draft:{tab_id}")
        } else {
            session_id.to_owned()
        }
    }

    fn sync_vnc_view(
        &mut self,
        state_key: &str,
        fields: &VncTabFields<'_>,
        daemon_session: Option<&VncSession>,
    ) {
        let view = self.vnc_views.entry(state_key.to_owned()).or_default();
        if !view.options_initialized
            && let Some(session) = daemon_session
        {
            view.draft_options = session.options.clone();
            view.options_initialized = true;
        }
        let host = fields.host;
        if !host.is_empty() && view.draft_host == "127.0.0.1" && host != "127.0.0.1" {
            host.clone_into(&mut view.draft_host);
            view.draft_port = fields.port.to_string();
            view.draft_display = display_for_port(&view.draft_port);
        }
        let Some(ds) = daemon_session else {
            return;
        };
        view.status = ds.status;
        if ds.error.is_some() {
            view.error.clone_from(&ds.error);
        }
        if ds.width > 0 && ds.height > 0 {
            view.width = ds.width;
            view.height = ds.height;
        }
    }

    fn vnc_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        request: VncToolbarRequest<'_>,
    ) -> VncToolbarActions {
        let VncToolbarRequest {
            state_key,
            fields,
            session,
            features,
        } = request;
        self.sync_vnc_view(state_key, fields, session);
        let status_color = session.map(|ds| self.vnc_status_color(ds.status));
        let view = self.vnc_views.entry(state_key.to_owned()).or_default();
        VncToolbar {
            fields,
            session,
            features,
            status_color,
            view,
            paste_shortcut: &mut self.preferences.vnc_paste_shortcut,
            fullscreen: &mut self.vnc_fullscreen,
            actions: VncToolbarActions::default(),
        }
        .show(ui)
    }

    fn vnc_status_color(&self, status: VncSessionStatus) -> egui::Color32 {
        match status {
            VncSessionStatus::Connected => appearance::color(&self.theme.status_running),
            VncSessionStatus::Connecting => appearance::color(&self.theme.status_waiting),
            VncSessionStatus::Disconnected | VncSessionStatus::Failed => {
                appearance::color(&self.theme.status_failed)
            }
        }
    }

    fn apply_vnc_toolbar_actions(&mut self, input: VncActionsInput<'_>) {
        let VncActionsInput {
            actions,
            session_id,
            session,
            save_host,
        } = input;
        let VncToolbarActions {
            connect: _,
            special_keys,
            disconnect,
            refresh,
            ctrl_alt_del,
            save_to_sidebar,
            open_sidebar,
        } = actions;
        if !special_keys.is_empty() && !session.is_some_and(|s| s.view_only) {
            self.send(Request::VncInput {
                session: session_id.to_owned(),
                events: special_keys,
            });
        }
        if open_sidebar {
            self.preferences.right_projects = false;
            self.preferences.visible = true;
            self.preferences.tool = crate::SidebarTool::Vnc;
        }
        if save_to_sidebar && !save_host.host.is_empty() {
            self.save_vnc_host(save_host);
        }
        if session_id.is_empty() {
            return;
        }
        if disconnect {
            self.send(Request::VncDisconnect {
                session: session_id.to_owned(),
            });
        }
        if refresh {
            self.send(Request::VncInput {
                session: session_id.to_owned(),
                events: vec![VncInputEvent::Refresh { full: true }],
            });
        }
        if ctrl_alt_del {
            self.send(Request::VncInput {
                session: session_id.to_owned(),
                events: VncToolbar::ctrl_alt_del_events(),
            });
        }
    }

    fn save_vnc_host(&mut self, input: VncSaveHost<'_>) {
        let VncSaveHost {
            state_key,
            profile_id,
            host,
            port,
            label,
        } = input;
        let new_id = id();
        let profile = VncProfile {
            options: self
                .vnc_views
                .get(state_key)
                .map_or_else(Default::default, |v| v.draft_options.clone()),
            credential_saved: false,
            id: new_id.clone(),
            name: label.to_owned(),
            host: host.to_owned(),
            port,
            username: String::new(),
            password: String::new(),
            shared: true,
            view_only: false,
            last_connected: 0,
        };
        *profile_id = Some(new_id);
        self.send(Request::VncProfileSave {
            profile,
            forget_password: false,
        });
    }

    fn vnc_status_lines(
        &self,
        ui: &mut egui::Ui,
        state_key: &str,
        daemon_session: Option<&VncSession>,
    ) {
        if let Some(record) = daemon_session {
            let security = match record.options.transport {
                terminator_core::vnc::Transport::Direct => "Direct VNC · unencrypted transport",
                terminator_core::vnc::Transport::Tls => "TLS · certificate verification required",
                terminator_core::vnc::Transport::Ssh => "SSH · trusted host key required",
            };
            ui.weak(security);
            if record.retry_attempt > 0 {
                ui.label(format!(
                    "Reconnect attempt {} · retry in {}s",
                    record.retry_attempt, record.retry_in_seconds
                ));
            }
        }
        if let Some(error) = self
            .vnc_views
            .get(state_key)
            .and_then(|v| v.error.as_deref())
        {
            ui.colored_label(appearance::color(&self.theme.status_failed), error);
        }
    }
}
