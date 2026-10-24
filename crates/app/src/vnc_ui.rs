use std::{
    io::Read,
    time::{Duration, Instant},
};

use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use eframe::egui::{self, Color32, ColorImage, RichText, TextureHandle};
use flate2::read::ZlibDecoder;
use terminator_core::{
    Request, VNC_CAPABILITY, id,
    vnc::{
        DEFAULT_PORT, VncFrame, VncInputEvent, VncProfile, VncSession, VncSessionStatus,
        parse_host_port,
    },
};

use crate::{After, App, Job, Tab, appearance, icons};

pub(crate) struct VncDecodedFrame {
    pub(crate) session: String,
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) revision: u64,
    pub(crate) status: VncSessionStatus,
    pub(crate) error: Option<String>,
    pub(crate) image: Option<ColorImage>,
    pub(crate) clipboard: Option<String>,
}

pub(crate) fn decode_frame(frame: VncFrame) -> Result<VncDecodedFrame> {
    let image = if frame.width > 0 && frame.height > 0 && !frame.zlib_rgba.is_empty() {
        let compressed = B64.decode(&frame.zlib_rgba)?;
        let expected = usize::from(frame.width)
            .saturating_mul(usize::from(frame.height))
            .saturating_mul(4);
        ensure!(
            expected <= 128 * 1024 * 1024,
            "VNC frame exceeds 128 MiB limit"
        );
        let mut rgba = Vec::with_capacity(expected);
        ZlibDecoder::new(compressed.as_slice())
            .take(u64::try_from(expected).unwrap_or(u64::MAX))
            .read_to_end(&mut rgba)?;
        if rgba.len() == expected {
            Some(ColorImage::from_rgba_unmultiplied(
                [usize::from(frame.width), usize::from(frame.height)],
                &rgba,
            ))
        } else {
            None
        }
    } else {
        None
    };
    Ok(VncDecodedFrame {
        session: frame.session,
        width: frame.width,
        height: frame.height,
        revision: frame.revision,
        status: frame.status,
        error: frame.error,
        image,
        clipboard: frame.clipboard,
    })
}

pub(crate) struct VncViewState {
    pub(crate) texture: Option<TextureHandle>,
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) revision: u64,
    pub(crate) status: VncSessionStatus,
    pub(crate) error: Option<String>,
    pub(crate) fetching: bool,
    pub(crate) last_poll: Option<Instant>,
    pub(crate) fit: bool,
    pub(crate) last_pointer: Option<(u16, u16, u8)>,
    pub(crate) draft_name: String,
    pub(crate) draft_host: String,
    pub(crate) draft_port: String,
    pub(crate) draft_username: String,
    pub(crate) draft_password: String,
    pub(crate) draft_shared: bool,
    pub(crate) draft_view_only: bool,
    pub(crate) save_profile: bool,
}

impl Default for VncViewState {
    fn default() -> Self {
        Self {
            texture: None,
            width: 0,
            height: 0,
            revision: 0,
            status: VncSessionStatus::Disconnected,
            error: None,
            fetching: false,
            last_poll: None,
            fit: true,
            last_pointer: None,
            draft_name: String::new(),
            draft_host: "127.0.0.1".into(),
            draft_port: DEFAULT_PORT.to_string(),
            draft_username: String::new(),
            draft_password: String::new(),
            draft_shared: true,
            draft_view_only: false,
            save_profile: true,
        }
    }
}

#[derive(Default)]
pub(crate) struct VncSidebarState {
    pub(crate) filter: String,
    pub(crate) editing_id: Option<String>,
    pub(crate) form_open: bool,
    pub(crate) name: String,
    pub(crate) host: String,
    pub(crate) port: String,
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) show_password: bool,
    pub(crate) shared: bool,
    pub(crate) view_only: bool,
}

impl VncSidebarState {
    pub(crate) fn begin_new(&mut self) {
        self.editing_id = None;
        self.form_open = true;
        self.name.clear();
        self.host = "127.0.0.1".into();
        self.port = DEFAULT_PORT.to_string();
        self.username.clear();
        self.password.clear();
        self.show_password = false;
        self.shared = true;
        self.view_only = false;
    }

    pub(crate) fn begin_edit(&mut self, profile: &VncProfile) {
        self.editing_id = Some(profile.id.clone());
        self.form_open = true;
        self.name.clone_from(&profile.name);
        self.host.clone_from(&profile.host);
        self.port = profile.port.to_string();
        self.username.clone_from(&profile.username);
        self.password.clone_from(&profile.password);
        self.show_password = false;
        self.shared = profile.shared;
        self.view_only = profile.view_only;
    }
}

struct PendingVncConnect {
    profile_id: Option<String>,
    host: String,
    port: u16,
    username: String,
    password: String,
    shared: bool,
    view_only: bool,
    label: Option<String>,
    save_profile: bool,
}

impl App {
    pub(crate) fn vnc_supported(&self) -> bool {
        !self.state_loaded
            || self
                .state
                .capabilities
                .iter()
                .any(|cap| cap == VNC_CAPABILITY)
    }

    pub(crate) fn open_new_browser_tab(&mut self, split: Option<&str>) {
        let Some(project) = self.selected.clone() else {
            return;
        };
        let _ = self.open_browser_url(&project, "https://example.com", split);
    }

    pub(crate) fn open_new_vnc_tab(&mut self, split: Option<&str>) {
        self.hide_center_overlay();
        let Some(project) = self.selected.clone() else {
            return;
        };
        let origin = self
            .active_session
            .as_ref()
            .map(|sid| Tab::Terminal(sid.clone()));
        let after = self.editor_target(&project, origin.as_ref(), split);
        let tab = Tab::Vnc {
            id: id(),
            session: String::new(),
            profile_id: None,
            host: String::new(),
            port: DEFAULT_PORT,
            label: "VNC Viewer".into(),
        };
        self.place_gui_tab(project, tab, after);
    }

    pub(crate) fn connect_vnc_profile(&mut self, profile: &VncProfile, split: Option<&str>) {
        self.hide_center_overlay();
        let Some(project) = self.selected.clone() else {
            return;
        };
        let origin = self
            .active_session
            .as_ref()
            .map(|sid| Tab::Terminal(sid.clone()));
        let after = self.editor_target(&project, origin.as_ref(), split);
        let _ = self.jobs.send(Job::rpc(
            Request::VncConnect {
                project,
                profile_id: Some(profile.id.clone()),
                host: profile.host.clone(),
                port: profile.port,
                username: profile.username.clone(),
                password: profile.password.clone(),
                shared: profile.shared,
                view_only: profile.view_only,
                label: Some(profile.display_name()),
            },
            after,
        ));
    }

    pub(crate) fn apply_vnc_frame(&mut self, ctx: &egui::Context, decoded: VncDecodedFrame) {
        let entry = self.vnc_views.entry(decoded.session.clone()).or_default();
        entry.fetching = false;
        entry.width = decoded.width;
        entry.height = decoded.height;
        entry.revision = decoded.revision;
        entry.status = decoded.status;
        entry.error = decoded.error;
        if let Some(image) = decoded.image {
            entry.texture = Some(ctx.load_texture(
                format!("vnc:{}", decoded.session),
                image,
                egui::TextureOptions::LINEAR,
            ));
        }
        if let Some(clipboard) = decoded.clipboard
            && !clipboard.is_empty()
        {
            ctx.copy_text(clipboard);
        }
    }

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
            self.vnc_profile_form(ui);
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
                && ui.small_button("✕").on_hover_text("Clear filter").clicked()
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
                            if appearance::sidebar_action(ui, "X", "Remove host").clicked() {
                                self.send(Request::VncProfileRemove {
                                    id: profile.id.clone(),
                                });
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

    fn vnc_profile_form(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.strong(if self.vnc_sidebar.editing_id.is_some() {
                "Edit VNC Host"
            } else {
                "New VNC Host"
            });
            ui.add_space(4.0);
            ui.label("Name");
            ui.add(
                appearance::singleline(&mut self.vnc_sidebar.name)
                    .hint_text("Build Mac / Linux VM")
                    .desired_width(f32::INFINITY),
            );
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label("Host");
                    ui.add(
                        appearance::singleline(&mut self.vnc_sidebar.host)
                            .hint_text("127.0.0.1")
                            .desired_width((ui.available_width() - 80.0).max(80.0)),
                    );
                });
                ui.vertical(|ui| {
                    ui.label("Port");
                    ui.add(
                        appearance::singleline(&mut self.vnc_sidebar.port)
                            .hint_text("5900")
                            .desired_width(68.0),
                    );
                });
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
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.vnc_sidebar.shared, "Shared session");
                ui.checkbox(&mut self.vnc_sidebar.view_only, "View only");
            });
            ui.horizontal(|ui| {
                let save = ui.button("Save").clicked();
                let save_connect = ui.button("Save & Connect").clicked();
                if ui.button("Cancel").clicked() {
                    self.vnc_sidebar.form_open = false;
                }
                if save || save_connect {
                    let (host, parsed_port) = parse_host_port(
                        &self.vnc_sidebar.host,
                        self.vnc_sidebar
                            .port
                            .trim()
                            .parse::<u16>()
                            .unwrap_or(DEFAULT_PORT),
                    );
                    if !host.is_empty() {
                        let profile = VncProfile {
                            id: self.vnc_sidebar.editing_id.clone().unwrap_or_else(id),
                            name: self.vnc_sidebar.name.trim().to_owned(),
                            host,
                            port: parsed_port,
                            username: self.vnc_sidebar.username.trim().to_owned(),
                            password: self.vnc_sidebar.password.clone(),
                            shared: self.vnc_sidebar.shared,
                            view_only: self.vnc_sidebar.view_only,
                            last_connected: 0,
                        };
                        self.send(Request::VncProfileSave {
                            profile: profile.clone(),
                        });
                        if save_connect {
                            self.connect_vnc_profile(&profile, None);
                        }
                        self.vnc_sidebar.form_open = false;
                    }
                }
            });
        });
    }

    pub(crate) fn focus_or_open_vnc_session(&mut self, session: &VncSession) {
        let project = if !session.project_id.is_empty() {
            session.project_id.clone()
        } else if let Some(selected) = &self.selected {
            selected.clone()
        } else {
            return;
        };
        if let Some(workspace) = self.layouts.get_mut(&project) {
            let existing = workspace
                .tabs
                .iter()
                .flat_map(|group| group.layout.iter_all_tabs())
                .map(|(_, tab)| tab.clone())
                .find(|tab| matches!(tab, Tab::Vnc { session: sid, .. } if sid == &session.id));
            if let Some(tab) = existing {
                workspace.activate_containing(&tab);
                if let Some(path) = workspace.find_tab(&tab) {
                    let _ = workspace.set_active_tab(path);
                    workspace.set_focused_node_and_surface(path.node_path());
                }
                self.select_project(project);
                self.active_session = None;
                self.non_terminal_selected = true;
                self.focus_tab = Some(tab);
                return;
            }
        }
        let tab = Tab::Vnc {
            id: id(),
            session: session.id.clone(),
            profile_id: session.profile_id.clone(),
            host: session.host.clone(),
            port: session.port,
            label: session.label.clone(),
        };
        self.select_project(project.clone());
        self.place_gui_tab(project, tab, After::Workspace(id(), Vec::new()));
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn vnc_view(
        &mut self,
        ui: &mut egui::Ui,
        tab_id: &str,
        session_id: &mut String,
        profile_id: &mut Option<String>,
        host: &mut String,
        port: &mut u16,
        label: &mut String,
    ) {
        self.visible_vnc.insert(session_id.clone());
        let daemon_session = self
            .state
            .vnc_sessions
            .iter()
            .find(|s| s.id == *session_id)
            .cloned();

        let state_key = if session_id.is_empty() {
            format!("draft:{tab_id}")
        } else {
            session_id.clone()
        };
        let mut connect_request: Option<PendingVncConnect> = None;
        let mut disconnect_clicked = false;
        let mut refresh_clicked = false;
        let mut cad_clicked = false;
        let mut save_to_sidebar = false;
        let mut open_sidebar = false;

        {
            let view = self.vnc_views.entry(state_key.clone()).or_default();
            if !host.is_empty() && view.draft_host == "127.0.0.1" && *host != "127.0.0.1" {
                view.draft_host.clone_from(host);
                view.draft_port = port.to_string();
            }
            if let Some(ds) = &daemon_session {
                view.status = ds.status;
                if ds.error.is_some() {
                    view.error.clone_from(&ds.error);
                }
                if ds.width > 0 && ds.height > 0 {
                    view.width = ds.width;
                    view.height = ds.height;
                }
            }

            ui.horizontal_wrapped(|ui| {
                egui::Image::new(icons::source("Monitor"))
                    .tint(appearance::ICON_COLOR)
                    .fit_to_exact_size(egui::vec2(16.0, 16.0))
                    .ui(ui);
                ui.strong(if label.is_empty() {
                    "VNC Viewer"
                } else {
                    label.as_str()
                });
                if !host.is_empty() {
                    ui.weak(format!("{host}:{port}"));
                }
                if let Some(ds) = &daemon_session {
                    let color = match ds.status {
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
                    ui.colored_label(color, format!("● {}", ds.status.label()));
                }
                if view.width > 0 && view.height > 0 {
                    ui.weak(format!("{} × {}", view.width, view.height));
                }
                if ui
                    .selectable_label(view.fit, "Fit")
                    .on_hover_text("Scale remote screen to pane")
                    .clicked()
                {
                    view.fit = true;
                }
                if ui
                    .selectable_label(!view.fit, "100%")
                    .on_hover_text("Actual pixel size")
                    .clicked()
                {
                    view.fit = false;
                }
                if daemon_session
                    .as_ref()
                    .is_some_and(|s| s.status == VncSessionStatus::Connected)
                {
                    refresh_clicked = ui.button("Refresh").clicked();
                    cad_clicked = ui
                        .button("Ctrl+Alt+Del")
                        .on_hover_text("Send Ctrl+Alt+Del to remote host")
                        .clicked();
                    disconnect_clicked = ui.button("Disconnect").clicked();
                } else if !host.is_empty() && ui.button("Reconnect").clicked() {
                    connect_request = Some(PendingVncConnect {
                        profile_id: profile_id.clone(),
                        host: host.clone(),
                        port: *port,
                        username: view.draft_username.clone(),
                        password: view.draft_password.clone(),
                        shared: view.draft_shared,
                        view_only: view.draft_view_only,
                        label: Some(label.clone()),
                        save_profile: false,
                    });
                }
                if !host.is_empty() && profile_id.is_none() && ui.button("Save host").clicked() {
                    save_to_sidebar = true;
                }
                if ui.button("VNC Sidebar").clicked() {
                    open_sidebar = true;
                }
            });
        }

        if open_sidebar {
            self.preferences.right_projects = false;
            self.preferences.visible = true;
            self.preferences.tool = crate::SidebarTool::Vnc;
        }
        if save_to_sidebar && !host.is_empty() {
            let new_id = id();
            let profile = VncProfile {
                id: new_id.clone(),
                name: label.clone(),
                host: host.clone(),
                port: *port,
                username: String::new(),
                password: String::new(),
                shared: true,
                view_only: false,
                last_connected: 0,
            };
            *profile_id = Some(new_id);
            self.send(Request::VncProfileSave { profile });
        }
        if disconnect_clicked && !session_id.is_empty() {
            self.send(Request::VncDisconnect {
                session: session_id.clone(),
            });
        }
        if refresh_clicked && !session_id.is_empty() {
            self.send(Request::VncInput {
                session: session_id.clone(),
                events: vec![VncInputEvent::Refresh { full: true }],
            });
        }
        if cad_clicked && !session_id.is_empty() {
            self.send(Request::VncInput {
                session: session_id.clone(),
                events: vec![
                    VncInputEvent::Key {
                        keycode: 0xFFE3,
                        down: true,
                    },
                    VncInputEvent::Key {
                        keycode: 0xFFE9,
                        down: true,
                    },
                    VncInputEvent::Key {
                        keycode: 0xFFFF,
                        down: true,
                    },
                    VncInputEvent::Key {
                        keycode: 0xFFFF,
                        down: false,
                    },
                    VncInputEvent::Key {
                        keycode: 0xFFE9,
                        down: false,
                    },
                    VncInputEvent::Key {
                        keycode: 0xFFE3,
                        down: false,
                    },
                ],
            });
        }

        ui.separator();

        let is_live = daemon_session.as_ref().is_some_and(|s| s.status.live());
        if !is_live {
            let profiles = self.state.vnc_profiles.clone();
            let view = self.vnc_views.entry(state_key.clone()).or_default();
            if let Some(error) = &view.error {
                ui.colored_label(appearance::color(&self.theme.status_failed), error);
                ui.add_space(4.0);
            }
            ui.vertical(|ui| {
                if !profiles.is_empty() {
                    ui.strong("Saved Connections (One-Click Connect)");
                    ui.horizontal_wrapped(|ui| {
                        for prof in &profiles {
                            if ui
                                .button(format!("▶ {} ({})", prof.display_name(), prof.endpoint()))
                                .clicked()
                            {
                                connect_request = Some(PendingVncConnect {
                                    profile_id: Some(prof.id.clone()),
                                    host: prof.host.clone(),
                                    port: prof.port,
                                    username: prof.username.clone(),
                                    password: prof.password.clone(),
                                    shared: prof.shared,
                                    view_only: prof.view_only,
                                    label: Some(prof.display_name()),
                                    save_profile: false,
                                });
                            }
                        }
                    });
                    ui.add_space(8.0);
                    ui.separator();
                }

                ui.strong("Connect to VNC Server");
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Host:");
                    let host_resp = ui.add(
                        appearance::singleline(&mut view.draft_host)
                            .hint_text("127.0.0.1 or host:5900")
                            .desired_width(220.0),
                    );
                    ui.label("Port:");
                    let port_resp = ui.add(
                        appearance::singleline(&mut view.draft_port)
                            .hint_text("5900")
                            .desired_width(70.0),
                    );
                    ui.label("Password:");
                    let pass_resp = ui.add(
                        appearance::singleline(&mut view.draft_password)
                            .password(true)
                            .hint_text("VNC password")
                            .desired_width(160.0),
                    );
                    let enter = (host_resp.lost_focus()
                        || port_resp.lost_focus()
                        || pass_resp.lost_focus())
                        && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.button("Connect").clicked() || enter {
                        let fallback = view
                            .draft_port
                            .trim()
                            .parse::<u16>()
                            .unwrap_or(DEFAULT_PORT);
                        let (parsed_host, parsed_port) =
                            parse_host_port(&view.draft_host, fallback);
                        if !parsed_host.is_empty() {
                            let title = if view.draft_name.trim().is_empty() {
                                format!("{parsed_host}:{parsed_port}")
                            } else {
                                view.draft_name.trim().to_owned()
                            };
                            connect_request = Some(PendingVncConnect {
                                profile_id: None,
                                host: parsed_host,
                                port: parsed_port,
                                username: view.draft_username.clone(),
                                password: view.draft_password.clone(),
                                shared: view.draft_shared,
                                view_only: view.draft_view_only,
                                label: Some(title),
                                save_profile: view.save_profile,
                            });
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Label:");
                    ui.add(
                        appearance::singleline(&mut view.draft_name)
                            .hint_text("Optional name")
                            .desired_width(160.0),
                    );
                    ui.checkbox(&mut view.draft_shared, "Shared");
                    ui.checkbox(&mut view.draft_view_only, "View only");
                    ui.checkbox(&mut view.save_profile, "Save to VNC sidebar");
                });
            });
        }

        if let Some(PendingVncConnect {
            profile_id: req_profile,
            host: req_host,
            port: req_port,
            username: req_user,
            password: req_pass,
            shared: req_shared,
            view_only: req_view_only,
            label: req_label,
            save_profile: should_save,
        }) = connect_request
            && let Some(project) = self.selected.clone()
        {
            host.clone_from(&req_host);
            *port = req_port;
            if let Some(l) = &req_label {
                label.clone_from(l);
            }
            let effective_profile_id = if should_save && req_profile.is_none() {
                let pid = id();
                let profile = VncProfile {
                    id: pid.clone(),
                    name: label.clone(),
                    host: req_host.clone(),
                    port: req_port,
                    username: req_user.clone(),
                    password: req_pass.clone(),
                    shared: req_shared,
                    view_only: req_view_only,
                    last_connected: 0,
                };
                self.send(Request::VncProfileSave { profile });
                Some(pid)
            } else {
                req_profile
            };
            profile_id.clone_from(&effective_profile_id);
            let _ = self.jobs.send(Job::VncConnectTab {
                tab_id: tab_id.to_owned(),
                request: Request::VncConnect {
                    project,
                    profile_id: effective_profile_id,
                    host: req_host,
                    port: req_port,
                    username: req_user,
                    password: req_pass,
                    shared: req_shared,
                    view_only: req_view_only,
                    label: req_label,
                },
            });
            return;
        }

        if !is_live {
            return;
        }

        // Poll for new framebuffer data when visible.
        let sid = session_id.clone();
        let should_poll = {
            let view = self.vnc_views.entry(sid.clone()).or_default();
            !view.fetching
                && view
                    .last_poll
                    .is_none_or(|t| t.elapsed() >= Duration::from_millis(33))
        };
        if should_poll {
            let since = self.vnc_views.get(&sid).map_or(0, |v| v.revision);
            if let Some(view) = self.vnc_views.get_mut(&sid) {
                view.fetching = true;
                view.last_poll = Some(Instant::now());
            }
            let _ = self.jobs.send(Job::VncFrame {
                session: sid.clone(),
                since,
            });
        }
        ui.ctx().request_repaint_after(Duration::from_millis(33));

        let mut outgoing = Vec::new();
        {
            let view = self.vnc_views.entry(sid.clone()).or_default();
            let Some(texture) = view.texture.as_ref() else {
                ui.centered_and_justified(|ui| {
                    ui.label("Connecting to VNC framebuffer…");
                });
                return;
            };
            let tex_size = texture.size_vec2();
            let available = ui.available_size();
            let display_size = if view.fit && tex_size.x > 0.0 && tex_size.y > 0.0 {
                let scale = (available.x / tex_size.x)
                    .min(available.y / tex_size.y)
                    .max(0.05);
                egui::vec2(tex_size.x * scale, tex_size.y * scale)
            } else {
                tex_size
            };

            let (rect, response) = ui.allocate_exact_size(
                available.max(egui::Vec2::ZERO),
                egui::Sense::click_and_drag(),
            );
            ui.painter()
                .rect_filled(rect, 0.0, Color32::from_rgb(16, 16, 20));
            let image_rect = egui::Rect::from_center_size(rect.center(), display_size);
            egui::Image::new(texture).paint_at(ui, image_rect);

            if response.clicked() || response.drag_started() {
                response.request_focus();
                self.active_session = None;
                self.non_terminal_selected = true;
            }

            let view_only = daemon_session.as_ref().is_some_and(|s| s.view_only);
            if !view_only && image_rect.width() > 1.0 && image_rect.height() > 1.0 {
                if let Some(pos) = response.hover_pos().or_else(|| {
                    response
                        .dragged()
                        .then(|| ui.input(|i| i.pointer.interact_pos()))
                        .flatten()
                }) && image_rect.contains(pos)
                {
                    let rel_x = ((pos.x - image_rect.left()) / image_rect.width()).clamp(0.0, 1.0);
                    let rel_y = ((pos.y - image_rect.top()) / image_rect.height()).clamp(0.0, 1.0);
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let x = (rel_x * f32::from(view.width.saturating_sub(1))) as u16;
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let y = (rel_y * f32::from(view.height.saturating_sub(1))) as u16;
                    let buttons = ui.input(|i| {
                        let mut mask = 0u8;
                        if i.pointer.primary_down() {
                            mask |= 1;
                        }
                        if i.pointer.middle_down() {
                            mask |= 2;
                        }
                        if i.pointer.secondary_down() {
                            mask |= 4;
                        }
                        mask
                    });
                    let next_ptr = (x, y, buttons);
                    if view.last_pointer != Some(next_ptr) {
                        view.last_pointer = Some(next_ptr);
                        outgoing.push(VncInputEvent::Pointer { x, y, buttons });
                    }
                    let scroll = ui.input(|i| i.smooth_scroll_delta);
                    if scroll.y.abs() > 1.0 {
                        let wheel_bit = if scroll.y > 0.0 { 8u8 } else { 16u8 };
                        outgoing.push(VncInputEvent::Pointer {
                            x,
                            y,
                            buttons: buttons | wheel_bit,
                        });
                        outgoing.push(VncInputEvent::Pointer { x, y, buttons });
                    }
                }

                if response.has_focus() {
                    ui.input(|input| {
                        for event in &input.events {
                            match event {
                                egui::Event::Key { key, pressed, .. } => {
                                    if let Some(keysym) = egui_key_to_x11(*key) {
                                        outgoing.push(VncInputEvent::Key {
                                            keycode: keysym,
                                            down: *pressed,
                                        });
                                    }
                                }
                                egui::Event::Text(text) => {
                                    for ch in text.chars() {
                                        let code = u32::from(ch);
                                        if (0x20..=0x7E).contains(&code) {
                                            outgoing.push(VncInputEvent::Key {
                                                keycode: code,
                                                down: true,
                                            });
                                            outgoing.push(VncInputEvent::Key {
                                                keycode: code,
                                                down: false,
                                            });
                                        }
                                    }
                                }
                                egui::Event::Paste(text) => {
                                    outgoing.push(VncInputEvent::CopyText(text.clone()));
                                }
                                _ => {}
                            }
                        }
                    });
                }
            }
        }

        if !outgoing.is_empty() {
            self.send(Request::VncInput {
                session: sid,
                events: outgoing,
            });
        }
    }
}

use egui::Widget;

fn egui_key_to_x11(key: egui::Key) -> Option<u32> {
    use egui::Key;
    match key {
        Key::Backspace => Some(0xFF08),
        Key::Tab => Some(0xFF09),
        Key::Enter => Some(0xFF0D),
        Key::Escape => Some(0xFF1B),
        Key::Insert => Some(0xFF63),
        Key::Delete => Some(0xFFFF),
        Key::Home => Some(0xFF50),
        Key::ArrowLeft => Some(0xFF51),
        Key::ArrowUp => Some(0xFF52),
        Key::ArrowRight => Some(0xFF53),
        Key::ArrowDown => Some(0xFF54),
        Key::PageUp => Some(0xFF55),
        Key::PageDown => Some(0xFF56),
        Key::End => Some(0xFF57),
        Key::F1 => Some(0xFFBE),
        Key::F2 => Some(0xFFBF),
        Key::F3 => Some(0xFFC0),
        Key::F4 => Some(0xFFC1),
        Key::F5 => Some(0xFFC2),
        Key::F6 => Some(0xFFC3),
        Key::F7 => Some(0xFFC4),
        Key::F8 => Some(0xFFC5),
        Key::F9 => Some(0xFFC6),
        Key::F10 => Some(0xFFC7),
        Key::F11 => Some(0xFFC8),
        Key::F12 => Some(0xFFC9),
        _ => None,
    }
}
