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
        DEFAULT_PORT, VncFrame, VncInputEvent, VncProfile, VncRegion, VncSession, VncSessionStatus,
        parse_host_port,
    },
};

use crate::preferences::VncPasteShortcut;
use crate::{After, App, Job, Tab, appearance, icons};

fn connection_endpoint(host: &str, port: &str) -> Result<(String, u16)> {
    let port = port
        .trim()
        .parse::<u16>()
        .map_err(|_| anyhow::anyhow!("Port must be between 1 and 65535"))?;
    ensure!(port > 0, "Port must be between 1 and 65535");
    let (host, port) = parse_host_port(host, port);
    ensure!(
        !host.trim().is_empty() && host.len() <= 255,
        "VNC host must be 1–255 characters"
    );
    Ok((host, port))
}

pub(crate) struct VncDecodedFrame {
    pub(crate) session: String,
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) revision: u64,
    pub(crate) status: VncSessionStatus,
    pub(crate) error: Option<String>,
    pub(crate) image: Option<ColorImage>,
    /// Set when `image` covers only this rectangle of the frame.
    pub(crate) region: Option<VncRegion>,
    pub(crate) clipboard: Option<String>,
}

pub(crate) fn decode_frame(frame: VncFrame) -> Result<VncDecodedFrame> {
    // A partial frame carries the changed rectangle only; the record's
    // width/height still describe the whole remote screen.
    let (pixels_w, pixels_h) = match frame.region {
        Some(region) => (region.width, region.height),
        None => (frame.width, frame.height),
    };
    let image = if pixels_w > 0 && pixels_h > 0 && !frame.zlib_rgba.is_empty() {
        let compressed = B64.decode(&frame.zlib_rgba)?;
        let expected = usize::from(pixels_w)
            .saturating_mul(usize::from(pixels_h))
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
                [usize::from(pixels_w), usize::from(pixels_h)],
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
        region: frame.region,
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
    /// The last received clipboard text. Frames repeat it until it changes.
    pub(crate) remote_clipboard: Option<String>,
    pub(crate) fetching: bool,
    pub(crate) last_poll: Option<Instant>,
    pub(crate) fit: bool,
    pub(crate) last_pointer: Option<(u16, u16, u8)>,
    /// Ask the next frame request for the whole screen: set when the
    /// texture cannot take an incremental update (missing, wrong size, or
    /// a decode that produced nothing).
    pub(crate) force_full: bool,
    /// Keyboard focus on the remote screen last frame: a fall from true to
    /// false releases every held key and modifier.
    pub(crate) focused: bool,
    /// Vergen modifier state (shift/ctrl/alt/meta) held for the remote:
    /// the viewer presses and releases the remote modifier keys itself so
    /// Shift+click and Ctrl+C land as the remote expects.
    pub(crate) modifier_bits: u8,
    /// Keysyms currently held down for the remote, released when this
    /// view loses focus so nothing sticks.
    pub(crate) held_keys: Vec<u32>,
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
            remote_clipboard: None,
            last_poll: None,
            fit: true,
            last_pointer: None,
            force_full: false,
            focused: false,
            modifier_bits: 0,
            held_keys: Vec::new(),
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

pub(crate) struct VncSidebarState {
    pub(crate) filter: String,
    pub(crate) error: Option<String>,
    pub(crate) editing_id: Option<String>,
    pub(crate) form_open: bool,
    pub(crate) name: String,
    pub(crate) host: String,
    pub(crate) port: String,
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) show_password: bool,
    pub(crate) save_password: bool,
    pub(crate) shared: bool,
    pub(crate) view_only: bool,
}

impl Default for VncSidebarState {
    fn default() -> Self {
        // The same starting point as [`Self::begin_new`]: a form that opens
        // without an explicit reset still shows a usable host and port.
        Self {
            filter: String::new(),
            error: None,
            editing_id: None,
            form_open: false,
            name: String::new(),
            host: "127.0.0.1".into(),
            port: DEFAULT_PORT.to_string(),
            username: String::new(),
            password: String::new(),
            show_password: false,
            save_password: true,
            shared: true,
            view_only: false,
        }
    }
}

impl VncSidebarState {
    pub(crate) fn begin_new(&mut self) {
        self.error = None;
        self.editing_id = None;
        self.form_open = true;
        self.name.clear();
        self.host = "127.0.0.1".into();
        self.port = DEFAULT_PORT.to_string();
        self.username.clear();
        self.password.clear();
        self.show_password = false;
        self.save_password = true;
        self.shared = true;
        self.view_only = false;
    }

    pub(crate) fn begin_edit(&mut self, profile: &VncProfile) {
        self.error = None;
        self.editing_id = Some(profile.id.clone());
        self.form_open = true;
        self.name.clone_from(&profile.name);
        self.host.clone_from(&profile.host);
        self.port = profile.port.to_string();
        self.username.clone_from(&profile.username);
        self.password.clone_from(&profile.password);
        self.show_password = false;
        self.save_password = true;
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
        let frame_size = [usize::from(decoded.width), usize::from(decoded.height)];
        let mut applied = false;
        if let Some(image) = decoded.image {
            // A partial frame is valid only on top of the texture the daemon
            // cut it from. Anything else (no texture yet, a resize, a
            // dropped frame) falls through to a full request.
            if let Some(region) = decoded.region {
                let in_bounds = usize::from(region.x).saturating_add(usize::from(region.width))
                    <= frame_size[0]
                    && usize::from(region.y).saturating_add(usize::from(region.height))
                        <= frame_size[1];
                if in_bounds
                    && let Some(texture) = entry.texture.as_mut()
                    && texture.size() == frame_size
                {
                    texture.set_partial(
                        [usize::from(region.x), usize::from(region.y)],
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    applied = true;
                }
            } else {
                entry.texture = Some(ctx.load_texture(
                    format!("vnc:{}", decoded.session),
                    image,
                    egui::TextureOptions::LINEAR,
                ));
                applied = true;
            }
        }
        if !applied {
            entry.force_full = true;
        }
        if let Some(clipboard) = decoded.clipboard
            && entry.remote_clipboard.as_ref() != Some(&clipboard)
        {
            entry.remote_clipboard = Some(clipboard.clone());
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
            if let Some(error) = &self.vnc_sidebar.error {
                ui.colored_label(appearance::color(&self.theme.status_failed), error);
            }
            ui.label("Name");
            ui.add(
                appearance::singleline(&mut self.vnc_sidebar.name)
                    .hint_text("Build Mac / Linux VM")
                    .desired_width(f32::INFINITY),
            );
            ui.horizontal(|ui| {
                let host_width =
                    (ui.available_width() - 68.0 - ui.spacing().item_spacing.x).max(40.0);
                ui.vertical(|ui| {
                    ui.set_width(host_width);
                    ui.label("Host");
                    ui.add(
                        appearance::singleline(&mut self.vnc_sidebar.host)
                            .hint_text("127.0.0.1")
                            .desired_width(host_width),
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
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.vnc_sidebar.shared, "Shared session");
                ui.checkbox(&mut self.vnc_sidebar.view_only, "View only");
            });
            ui.checkbox(
                &mut self.vnc_sidebar.save_password,
                "Save password with this host",
            )
            .on_hover_text("Off: the password is only used for this session");
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
            Ok(profile)
        })();
        match result {
            Ok(profile) => {
                let mut saved = profile.clone();
                if !self.vnc_sidebar.save_password {
                    saved.password.clear();
                }
                self.send(Request::VncProfileSave { profile: saved });
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

    /// Drop view state for sessions and drafts that no open tab uses. Runs
    /// after the frame is laid out, so a tab that closed this frame loses
    /// its texture instead of keeping it alive forever.
    pub(super) fn prune_vnc_views(&mut self) {
        let mut live: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut collect = |tab: &Tab| {
            if let Tab::Vnc { id, session, .. } = tab {
                if session.is_empty() {
                    live.insert(format!("draft:{id}"));
                } else {
                    live.insert(session.clone());
                }
            }
        };
        for workspace in self.layouts.values() {
            for tab in &workspace.tabs {
                for (_, tab) in tab.layout.iter_all_tabs() {
                    collect(tab);
                }
            }
        }
        for window in &self.floating {
            let Some(dock) = window.dock.as_ref() else {
                continue;
            };
            for (_, tab) in dock.iter_all_tabs() {
                collect(tab);
            }
        }
        // Views painted this frame are kept even if their tab is mid-move:
        // a transient dock check-out must not drop a live texture.
        live.extend(self.visible_vnc.iter().cloned());
        self.vnc_views.retain(|key, _| live.contains(key));
    }

    /// True when any open tab (docked or floating) still shows `session`.
    pub(super) fn vnc_session_has_tab(&self, session: &str) -> bool {
        let docked = self.layouts.values().any(|workspace| {
            workspace.tabs.iter().any(|tab| {
                tab.layout
                    .iter_all_tabs()
                    .any(|(_, tab)| matches!(tab, Tab::Vnc { session: sid, .. } if sid == session))
            })
        });
        if docked {
            return true;
        }
        self.floating.iter().any(|window| {
            window.dock.as_ref().is_some_and(|dock| {
                dock.iter_all_tabs()
                    .any(|(_, tab)| matches!(tab, Tab::Vnc { session: sid, .. } if sid == session))
            })
        })
    }

    /// Send the deferred disconnect for a closed VNC tab, unless another
    /// tab still shows the session. Called after the dock is checked back
    /// in: counting tabs while the workspace is checked out missed the
    /// remaining copies and dropped a session still on screen.
    pub(super) fn drain_pending_vnc_disconnect(&mut self) {
        let Some(session) = self.pending_vnc_disconnect.take() else {
            return;
        };
        if session.is_empty() || self.vnc_session_has_tab(&session) {
            return;
        }
        self.send(Request::VncDisconnect { session });
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
        let daemon_session = self
            .state
            .vnc_sessions
            .iter()
            .find(|s| s.id == *session_id)
            .cloned();

        // View state is keyed by session, or by the tab while it is still a
        // draft: the key doubles as the "seen this frame" marker that keeps
        // the pruner from dropping live view state.
        let state_key = if session_id.is_empty() {
            format!("draft:{tab_id}")
        } else {
            session_id.clone()
        };
        self.visible_vnc.insert(state_key.clone());
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
                // Actions stay one row of same-size icon buttons; the two
                // view-mode toggles trail them so the icon row does not mix
                // in differently sized widgets.
                if daemon_session
                    .as_ref()
                    .is_some_and(|s| s.status == VncSessionStatus::Connected)
                {
                    refresh_clicked =
                        appearance::sidebar_action(ui, "RefreshCw", "Refresh remote screen")
                            .clicked();
                    cad_clicked = appearance::sidebar_action(
                        ui,
                        "Keyboard",
                        "Send Ctrl+Alt+Del to remote host",
                    )
                    .clicked();
                    disconnect_clicked =
                        appearance::sidebar_action(ui, "CircleStop", "Disconnect").clicked();
                } else if !host.is_empty()
                    && appearance::sidebar_action(ui, "Play", "Reconnect").clicked()
                {
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
                if !host.is_empty()
                    && profile_id.is_none()
                    && appearance::sidebar_action(ui, "Save", "Save host to the VNC sidebar")
                        .clicked()
                {
                    save_to_sidebar = true;
                }
                if appearance::sidebar_action(ui, "PanelRight", "Open the VNC sidebar").clicked() {
                    open_sidebar = true;
                }
                ui.separator();
                appearance::text_menu_button(ui, "Clipboard", |ui| {
                    ui.label("Remote paste shortcut (copy and cut follow it)");
                    for (shortcut, label) in [
                        (VncPasteShortcut::ShiftInsert, "Shift+Insert"),
                        (VncPasteShortcut::ControlV, "Ctrl+V"),
                        (VncPasteShortcut::ControlShiftV, "Ctrl+Shift+V"),
                        (VncPasteShortcut::CommandV, "Command+V (macOS)"),
                    ] {
                        if ui
                            .selectable_value(
                                &mut self.preferences.vnc_paste_shortcut,
                                shortcut,
                                label,
                            )
                            .clicked()
                        {
                            ui.close();
                        }
                    }
                });
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
            let view = self.vnc_views.entry(state_key.clone()).or_default();
            if let Some(error) = &view.error {
                ui.colored_label(appearance::color(&self.theme.status_failed), error);
                ui.add_space(4.0);
            }
            // The saved hosts live in the VNC sidebar; repeating them here
            // duplicated every row and its two controls, so this pane only
            // carries the direct connection form.
            ui.vertical(|ui| {
                ui.strong("Connect to VNC Server");
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label("Host:");
                    let host_resp = ui.add(
                        appearance::singleline(&mut view.draft_host)
                            .hint_text("127.0.0.1 or host:5900")
                            .desired_width(200.0),
                    );
                    ui.label("Port:");
                    let port_resp = ui.add(
                        appearance::singleline(&mut view.draft_port)
                            .hint_text("5900")
                            .desired_width(70.0),
                    );
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
                    let enter = (host_resp.lost_focus()
                        || port_resp.lost_focus()
                        || pass_resp.lost_focus())
                        && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.button("Connect").clicked() || enter {
                        match connection_endpoint(&view.draft_host, &view.draft_port) {
                            Ok((parsed_host, parsed_port)) => {
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
                            Err(error) => view.error = Some(error.to_string()),
                        }
                    }
                });
                ui.horizontal_wrapped(|ui| {
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
            // A view whose texture cannot take the incremental update (no
            // texture yet, wrong size, dropped decode) asks for the whole
            // screen once; the daemon answers anything else with the
            // changed rectangle only.
            let (since, regions) = self
                .vnc_views
                .get(&sid)
                .map_or((0, false), |view| (view.revision, !view.force_full));
            if let Some(view) = self.vnc_views.get_mut(&sid) {
                view.fetching = true;
                view.last_poll = Some(Instant::now());
                view.force_full = false;
            }
            let _ = self.jobs.send(Job::VncFrame {
                session: sid.clone(),
                since,
                regions,
            });
        }
        ui.ctx().request_repaint_after(Duration::from_millis(33));

        let texture = {
            let view = self.vnc_views.entry(sid.clone()).or_default();
            let Some(texture) = view.texture.clone() else {
                ui.centered_and_justified(|ui| {
                    ui.label("Connecting to VNC framebuffer…");
                });
                return;
            };
            texture
        };
        let view_only = daemon_session.as_ref().is_some_and(|s| s.view_only);
        let outcome = {
            let view = self.vnc_views.entry(sid.clone()).or_default();
            vnc_screen(
                ui,
                view,
                &texture,
                view_only,
                self.preferences.vnc_paste_shortcut,
            )
        };
        if outcome.activated {
            self.active_session = None;
            self.non_terminal_selected = true;
        }
        if !outcome.events.is_empty() {
            self.send(Request::VncInput {
                session: sid,
                events: outcome.events,
            });
        }
    }
}

#[derive(Default)]
struct VncScreenOutcome {
    events: Vec<VncInputEvent>,
    activated: bool,
}

/// Remote modifier keysyms: one X11 keysym per bit of
/// [`VncViewState::modifier_bits`].
const MODIFIER_KEYSYMS: [(u8, u32); 4] = [
    (1, 0xFFE1), // Shift_L
    (2, 0xFFE3), // Control_L
    (4, 0xFFE9), // Alt_L
    (8, 0xFFEB), // Meta_L (Command on macOS)
];

/// Local modifier state reduced to the remote's four keys.
fn modifier_bits(modifiers: egui::Modifiers) -> u8 {
    let mut bits = 0u8;
    if modifiers.shift {
        bits |= 1;
    }
    if modifiers.ctrl {
        bits |= 2;
    }
    if modifiers.alt {
        bits |= 4;
    }
    // `command` mirrors Ctrl away from macOS; on macOS it is Command.
    if modifiers.command && !modifiers.ctrl {
        bits |= 8;
    }
    bits
}

/// Remote key transitions between two modifier snapshots: the remote sees
/// exactly the modifiers that changed, never a stale held key.
fn modifier_transitions(before: u8, after: u8) -> Vec<(u32, bool)> {
    let mut transitions = Vec::new();
    for (bit, keysym) in MODIFIER_KEYSYMS {
        let was_down = before & bit != 0;
        let is_down = after & bit != 0;
        if was_down != is_down {
            transitions.push((keysym, is_down));
        }
    }
    transitions
}

/// Letter and digit keysyms for accelerator chords: egui suppresses the
/// text event while Ctrl/Command is held, so the letter has to travel as a
/// key event or the remote shortcut never fires.
fn accelerator_keysym(key: egui::Key) -> Option<u32> {
    use egui::Key;
    let letter = match key {
        Key::A => 'a',
        Key::B => 'b',
        Key::C => 'c',
        Key::D => 'd',
        Key::E => 'e',
        Key::F => 'f',
        Key::G => 'g',
        Key::H => 'h',
        Key::I => 'i',
        Key::J => 'j',
        Key::K => 'k',
        Key::L => 'l',
        Key::M => 'm',
        Key::N => 'n',
        Key::O => 'o',
        Key::P => 'p',
        Key::Q => 'q',
        Key::R => 'r',
        Key::S => 's',
        Key::T => 't',
        Key::U => 'u',
        Key::V => 'v',
        Key::W => 'w',
        Key::X => 'x',
        Key::Y => 'y',
        Key::Z => 'z',
        _ => return None,
    };
    Some(u32::from(letter))
}

/// Translate local clipboard commands into the remote application's shortcuts.
/// Clipboard transfer alone does not insert text. Local modifiers must not
/// change the remote chord (for example Command must not modify Shift+Insert).
fn remote_clipboard_events(
    event: &egui::Event,
    shortcut: VncPasteShortcut,
    held_modifiers: u8,
) -> Vec<VncInputEvent> {
    let (modifiers, key) = match (shortcut, event) {
        (VncPasteShortcut::ShiftInsert, egui::Event::Paste(_)) => (1, 0xFF63),
        (VncPasteShortcut::ShiftInsert, egui::Event::Copy) => (2, 0xFF63),
        (VncPasteShortcut::ShiftInsert, egui::Event::Cut) => (1, 0xFFFF),
        (shortcut, egui::Event::Paste(_)) => (clipboard_modifiers(shortcut), u32::from('v')),
        (shortcut, egui::Event::Copy) => (clipboard_modifiers(shortcut), u32::from('c')),
        (shortcut, egui::Event::Cut) => (clipboard_modifiers(shortcut), u32::from('x')),
        _ => return Vec::new(),
    };
    let mut events = Vec::new();
    for (keycode, down) in modifier_transitions(held_modifiers, 0) {
        events.push(VncInputEvent::Key { keycode, down });
    }
    if let egui::Event::Paste(text) = event {
        events.push(VncInputEvent::CopyText(text.to_owned()));
    }
    for (keycode, down) in modifier_transitions(0, modifiers) {
        events.push(VncInputEvent::Key { keycode, down });
    }
    events.push(VncInputEvent::Key {
        keycode: key,
        down: true,
    });
    events.push(VncInputEvent::Key {
        keycode: key,
        down: false,
    });
    for (keycode, down) in modifier_transitions(modifiers, 0)
        .into_iter()
        .chain(modifier_transitions(0, held_modifiers))
    {
        events.push(VncInputEvent::Key { keycode, down });
    }
    events
}

fn clipboard_modifiers(shortcut: VncPasteShortcut) -> u8 {
    match shortcut {
        VncPasteShortcut::ControlV => 2,
        VncPasteShortcut::ControlShiftV => 3,
        VncPasteShortcut::CommandV => 8,
        VncPasteShortcut::ShiftInsert => 1,
    }
}

/// One remote screen inside its pane: painting, pointer mapping, wheel,
/// keyboard events, and the pointer/key release bookkeeping that stops a
/// key or button from sticking when focus moves away mid-gesture.
fn vnc_screen(
    ui: &mut egui::Ui,
    view: &mut VncViewState,
    texture: &TextureHandle,
    view_only: bool,
    paste_shortcut: VncPasteShortcut,
) -> VncScreenOutcome {
    let mut outcome = VncScreenOutcome::default();
    let tex_size = texture.size_vec2();
    let available = ui.available_size();
    // Output area: scaled to the pane ("Fit"), or shown at actual pixel
    // size inside scroll bars ("100%").
    let scroll = egui::ScrollArea::both()
        .id_salt("vnc-screen")
        .auto_shrink([false, false]);
    let mut image_rect = egui::Rect::NOTHING;
    let mut pointer_events: Vec<VncInputEvent> = Vec::new();
    let scroll_output = scroll.show(ui, |ui| {
        let display_size = if view.fit && tex_size.x > 0.0 && tex_size.y > 0.0 {
            let scale = (available.x / tex_size.x)
                .min(available.y / tex_size.y)
                .max(0.05);
            egui::vec2(tex_size.x * scale, tex_size.y * scale)
        } else {
            tex_size
        };
        let size = if view.fit {
            available.max(egui::Vec2::ZERO)
        } else {
            display_size
        };
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        ui.painter()
            .rect_filled(rect, 0.0, Color32::from_rgb(16, 16, 20));
        image_rect = egui::Rect::from_center_size(rect.center(), display_size);
        egui::Image::new(texture).paint_at(ui, image_rect);

        if response.clicked() || response.drag_started() {
            response.request_focus();
            outcome.activated = true;
        }

        // Pointer mapping: window point to remote pixel, clamped to the
        // screen edge so a drag that leaves the pane keeps tracking.
        let remote_at = |pos: egui::Pos2| -> Option<(u16, u16)> {
            if view.width == 0 || view.height == 0 || !image_rect.is_positive() {
                return None;
            }
            let rel_x = ((pos.x - image_rect.left()) / image_rect.width()).clamp(0.0, 1.0);
            let rel_y = ((pos.y - image_rect.top()) / image_rect.height()).clamp(0.0, 1.0);
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let x = (rel_x * f32::from(view.width.saturating_sub(1))) as u16;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let y = (rel_y * f32::from(view.height.saturating_sub(1))) as u16;
            Some((x, y))
        };
        let buttons = ui.input(|input| {
            let mut mask = 0u8;
            if input.pointer.primary_down() {
                mask |= 1;
            }
            if input.pointer.middle_down() {
                mask |= 2;
            }
            if input.pointer.secondary_down() {
                mask |= 4;
            }
            mask
        });
        let pointer_pos = response.hover_pos().or_else(|| {
            response
                .dragged()
                .then(|| ui.input(|input| input.pointer.interact_pos()))
                .flatten()
        });
        let pos = pointer_pos.filter(|pos| image_rect.contains(*pos) || response.dragged());
        if let Some((x, y)) = pos.and_then(&remote_at) {
            let next = (x, y, buttons);
            let changed = view.last_pointer != Some(next);
            if changed {
                view.last_pointer = Some(next);
                pointer_events.push(VncInputEvent::Pointer { x, y, buttons });
            }
            // Wheel: forwarded only when the remote fills the pane; at
            // 100% the local scroll bars own the wheel.
            let scroll_delta = ui.input(|input| input.smooth_scroll_delta);
            if view.fit && scroll_delta.y.abs() > 1.0 {
                let wheel_bit = if scroll_delta.y > 0.0 { 8u8 } else { 16u8 };
                pointer_events.push(VncInputEvent::Pointer {
                    x,
                    y,
                    buttons: buttons | wheel_bit,
                });
                pointer_events.push(VncInputEvent::Pointer { x, y, buttons });
            }
        } else if let Some((x, y, held)) = view.last_pointer
            && held != 0
        {
            // The press happened here but the release landed outside the
            // pane: release the button, or the remote keeps it down.
            view.last_pointer = Some((x, y, 0));
            pointer_events.push(VncInputEvent::Pointer { x, y, buttons: 0 });
        }
        response
    });

    let modifiers = ui.input(|input| modifier_bits(input.modifiers));
    let window_focused = ui.ctx().input(|input| input.focused);
    let focused = scroll_output.inner.has_focus() && window_focused;
    let mut keyboard_events: Vec<VncInputEvent> = Vec::new();
    if focused || scroll_output.inner.hovered() || scroll_output.inner.dragged() {
        // Modifiers reach the remote as their own key presses, so
        // Shift+click selects and Ctrl+C interrupts there too.
        if view.modifier_bits != modifiers {
            for (keysym, down) in modifier_transitions(view.modifier_bits, modifiers) {
                keyboard_events.push(VncInputEvent::Key {
                    keycode: keysym,
                    down,
                });
            }
            view.modifier_bits = modifiers;
        }
    }
    if focused {
        ui.input(|input| {
            for event in &input.events {
                match event {
                    egui::Event::Key {
                        key,
                        pressed,
                        modifiers,
                        ..
                    } => {
                        let keysym = egui_key_to_x11(*key).or_else(|| {
                            (modifiers.ctrl || modifiers.command)
                                .then(|| accelerator_keysym(*key))
                                .flatten()
                        });
                        let Some(keysym) = keysym else {
                            continue;
                        };
                        if *pressed {
                            if !view.held_keys.contains(&keysym) {
                                view.held_keys.push(keysym);
                            }
                        } else {
                            view.held_keys.retain(|held| *held != keysym);
                        }
                        keyboard_events.push(VncInputEvent::Key {
                            keycode: keysym,
                            down: *pressed,
                        });
                    }
                    egui::Event::Text(text) => {
                        for ch in text.chars() {
                            let code = u32::from(ch);
                            if (0x20..=0x7E).contains(&code) {
                                keyboard_events.push(VncInputEvent::Key {
                                    keycode: code,
                                    down: true,
                                });
                                keyboard_events.push(VncInputEvent::Key {
                                    keycode: code,
                                    down: false,
                                });
                            }
                        }
                    }
                    egui::Event::Paste(_) | egui::Event::Copy | egui::Event::Cut => {
                        keyboard_events.extend(remote_clipboard_events(
                            event,
                            paste_shortcut,
                            view.modifier_bits,
                        ));
                    }
                    _ => {}
                }
            }
        });
    }
    // Leaving the pane (or the window) mid-chord must not leave keys or
    // modifiers stuck down on the remote.
    if view.focused && !focused {
        for (bit, keysym) in MODIFIER_KEYSYMS {
            if view.modifier_bits & bit != 0 {
                keyboard_events.push(VncInputEvent::Key {
                    keycode: keysym,
                    down: false,
                });
            }
        }
        view.modifier_bits = 0;
        for keysym in view.held_keys.drain(..) {
            keyboard_events.push(VncInputEvent::Key {
                keycode: keysym,
                down: false,
            });
        }
    }
    view.focused = focused;

    if !view_only {
        outcome.events.extend(pointer_events);
        outcome.events.extend(keyboard_events);
    }
    outcome
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

#[cfg(test)]
mod tests {
    use super::*;

    fn compressed(pixels: &[u8]) -> String {
        use flate2::{Compression, write::ZlibEncoder};
        use std::io::Write;
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(pixels).expect("compress pixels");
        B64.encode(encoder.finish().expect("finish compression"))
    }

    fn frame(region: Option<VncRegion>, pixels: &[u8]) -> VncFrame {
        VncFrame {
            session: "s".into(),
            width: 2,
            height: 2,
            revision: 3,
            status: VncSessionStatus::Connected,
            error: None,
            zlib_rgba: compressed(pixels),
            region,
            clipboard: None,
        }
    }

    #[test]
    fn a_partial_frame_decodes_to_region_pixels_only() {
        let region = VncRegion {
            x: 1,
            y: 0,
            width: 1,
            height: 2,
        };
        let pixels = [0u8, 255, 0, 255, 0, 0, 255, 255];
        let decoded = decode_frame(frame(Some(region), &pixels)).expect("region frame decodes");
        assert_eq!(decoded.region, Some(region));
        assert_eq!(decoded.width, 2, "the record keeps the remote size");
        assert_eq!(decoded.height, 2);
        let image = decoded.image.expect("region pixels decode");
        assert_eq!(image.size, [1, 2], "the image covers the region only");
        assert_eq!(
            image.pixels[0],
            Color32::from_rgba_unmultiplied(0, 255, 0, 255)
        );
        assert_eq!(
            image.pixels[1],
            Color32::from_rgba_unmultiplied(0, 0, 255, 255)
        );
    }

    #[test]
    fn a_frame_without_a_region_decodes_at_the_remote_size() {
        let pixels = [1u8, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255];
        let decoded = decode_frame(frame(None, &pixels)).expect("full frame decodes");
        assert_eq!(decoded.region, None);
        let image = decoded.image.expect("full pixels decode");
        assert_eq!(image.size, [2, 2]);
    }

    #[test]
    fn modifier_transitions_report_each_changed_key_once() {
        assert_eq!(
            modifier_transitions(0, 0b0011),
            vec![(0xFFE1, true), (0xFFE3, true)],
            "newly pressed modifiers go down in keysym order"
        );
        assert_eq!(
            modifier_transitions(0b0101, 0),
            vec![(0xFFE1, false), (0xFFE9, false)],
            "every released modifier goes up"
        );
        assert!(modifier_transitions(0b1010, 0b1010).is_empty());
        assert_eq!(
            modifier_transitions(0b0001, 0b1000),
            vec![(0xFFE1, false), (0xFFEB, true)],
            "a swap releases the old key before pressing the new one"
        );
    }

    #[test]
    fn modifier_bits_map_command_to_meta_without_double_counting_ctrl() {
        let none = egui::Modifiers::default();
        assert_eq!(modifier_bits(none), 0);
        let shift_alt = egui::Modifiers {
            shift: true,
            alt: true,
            ..none
        };
        assert_eq!(modifier_bits(shift_alt), 1 | 4);
        // macOS Command is `command` without `ctrl`.
        let command = egui::Modifiers {
            command: true,
            mac_cmd: true,
            ..none
        };
        assert_eq!(modifier_bits(command), 8);
        // Elsewhere `command` mirrors Ctrl: one Control key, not two.
        let ctrl = egui::Modifiers {
            ctrl: true,
            command: true,
            ..none
        };
        assert_eq!(modifier_bits(ctrl), 2);
    }

    #[test]
    fn accelerator_keys_fall_back_to_keycodes_for_letters() {
        assert_eq!(accelerator_keysym(egui::Key::C), Some(u32::from('c')));
        assert_eq!(accelerator_keysym(egui::Key::V), Some(u32::from('v')));
        assert_eq!(accelerator_keysym(egui::Key::F5), None);
    }
    #[test]
    fn focused_vnc_clipboard_commands_send_remote_shortcuts() {
        let ctx = egui::Context::default();
        let mut view = VncViewState {
            width: 800,
            height: 600,
            ..Default::default()
        };
        let texture = ctx.load_texture(
            "clipboard-test",
            ColorImage::filled([800, 600], Color32::BLACK),
            egui::TextureOptions::NEAREST,
        );
        let mut paint = |mut events: Vec<egui::Event>, modifiers: egui::Modifiers, view_only| {
            events.insert(0, egui::Event::ModifiersChanged(modifiers));
            let mut result = None;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    focused: true,
                    events,
                    ..Default::default()
                },
                |ui| {
                    result = Some(vnc_screen(
                        ui,
                        &mut view,
                        &texture,
                        view_only,
                        VncPasteShortcut::ShiftInsert,
                    ));
                },
            );
            output.textures_delta.clear();
            result.unwrap().events
        };
        let none = egui::Modifiers::NONE;
        paint(vec![], none, false);
        let pos = egui::pos2(120.0, 170.0);
        paint(
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: none,
                },
            ],
            none,
            false,
        );
        paint(
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: none,
            }],
            none,
            false,
        );
        let command = egui::Modifiers {
            command: true,
            mac_cmd: true,
            ..none
        };
        let mut paste = paint(
            vec![egui::Event::Paste("gui-vnc-paste-ok".into())],
            command,
            false,
        );
        paste.extend(paint(vec![], none, false));
        assert!(paste.iter().any(
            |event| matches!(event,VncInputEvent::CopyText(text) if text=="gui-vnc-paste-ok")
        ));
        assert!(paste.iter().any(|event| matches!(
            event,
            VncInputEvent::Key {
                keycode: 0xFF63,
                down: true
            }
        )));
        assert!(matches!(
            paste.last(),
            Some(VncInputEvent::Key {
                keycode: 0xFFEB,
                down: false
            })
        ));
        let mut copy = paint(vec![egui::Event::Copy], command, false);
        copy.extend(paint(vec![], none, false));
        assert!(
            !copy
                .iter()
                .any(|event| matches!(event, VncInputEvent::CopyText(_)))
        );
        assert!(copy.iter().any(|event| matches!(
            event,
            VncInputEvent::Key {
                keycode: 0xFF63,
                down: true
            }
        )));
        let blocked = paint(
            vec![
                egui::Event::Paste("blocked".into()),
                egui::Event::Copy,
                egui::Event::Cut,
            ],
            none,
            true,
        );
        assert!(blocked.is_empty());
        println!(
            "GUI_CLIPBOARD_EVENTS={}",
            serde_json::json!({"paste":paste,"copy":copy})
        );
    }

    #[test]
    fn clipboard_shortcuts_release_local_modifiers_and_use_the_selected_remote_chord() {
        for (shortcut, key, modifier) in [
            (VncPasteShortcut::ShiftInsert, 0xFF63, 0xFFE1),
            (VncPasteShortcut::ControlV, u32::from('v'), 0xFFE3),
            (VncPasteShortcut::ControlShiftV, u32::from('v'), 0xFFE3),
            (VncPasteShortcut::CommandV, u32::from('v'), 0xFFEB),
        ] {
            let events = remote_clipboard_events(&egui::Event::Paste("text".into()), shortcut, 8);
            assert!(matches!(
                events.first(),
                Some(VncInputEvent::Key {
                    keycode: 0xFFEB,
                    down: false
                })
            ));
            let key_index = events
                .iter()
                .position(|e| matches!(e,VncInputEvent::Key {keycode,down:true} if *keycode==key))
                .unwrap();
            let transfer = events
                .iter()
                .position(|e| matches!(e,VncInputEvent::CopyText(text) if text=="text"))
                .unwrap();
            let modifier_index = events
                .iter()
                .position(
                    |e| matches!(e,VncInputEvent::Key {keycode,down:true} if *keycode==modifier),
                )
                .unwrap();
            assert!(transfer < modifier_index && modifier_index < key_index);
            assert!(matches!(
                events.last(),
                Some(VncInputEvent::Key {
                    keycode: 0xFFEB,
                    down: true
                })
            ));
        }
    }
}
