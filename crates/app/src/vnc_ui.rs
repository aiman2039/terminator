mod clipboard;
mod display;
mod frame;
mod keyboard;
mod profiles;
mod settings;
mod sidebar;
mod state;
mod transfer;
use clipboard::remote_clipboard_events;
use display::vnc_screen;
pub(crate) use frame::{VncDecodedFrame, decode_frame};
use keyboard::{
    MODIFIER_KEYSYMS, accelerator_keysym, egui_key_to_x11, modifier_bits, modifier_transitions,
};
pub(crate) use state::{VncSidebarState, VncViewState};
use std::{
    io::Read,
    time::{Duration, Instant},
};

use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
#[cfg(test)]
use eframe::egui::Color32;
use eframe::egui::{self, ColorImage, RichText, TextureHandle};
use flate2::read::ZlibDecoder;
use terminator_core::{
    Request, VNC_CAPABILITY, id,
    vnc::{VncFrame, VncInputEvent, VncProfile, VncRegion, VncSession, VncSessionStatus},
};

use crate::preferences::VncPasteShortcut;
use crate::{After, App, Job, Tab, appearance, icons};

// New forms start at display 66; saved profiles retain their explicit port.
const DEFAULT_CONNECTION_PORT: u16 = 5966;

fn display_for_port(port: &str) -> String {
    port.trim()
        .parse::<u16>()
        .ok()
        .and_then(|port| port.checked_sub(terminator_core::vnc::DEFAULT_PORT))
        .map(|display| display.to_string())
        .unwrap_or_default()
}

fn port_for_display(display: &str) -> String {
    display
        .trim()
        .parse::<u16>()
        .ok()
        .and_then(|display| display.checked_add(terminator_core::vnc::DEFAULT_PORT))
        .map(|port| port.to_string())
        .unwrap_or_default()
}

fn port_display_fields(ui: &mut egui::Ui, port: &mut String, display: &mut String) -> bool {
    let port_response = ui
        .vertical(|ui| {
            ui.set_width(68.0);
            ui.label("Port");
            ui.add(appearance::singleline(port).desired_width(68.0))
                .on_hover_text("TCP port: 1–65535")
        })
        .inner;
    if port_response.changed() {
        *display = display_for_port(port);
    }
    let display_response = ui
        .vertical(|ui| {
            ui.set_width(68.0);
            ui.label("Display");
            ui.add(appearance::singleline(display).desired_width(68.0))
                .on_hover_text("Port = 5900 + display. Valid display: 0–59635")
        })
        .inner;
    if display_response.changed() {
        *port = port_for_display(display);
    }
    port_response.lost_focus() || display_response.lost_focus()
}

fn connection_endpoint(host: &str, port: &str) -> Result<(String, u16)> {
    let port = port
        .trim()
        .parse::<u16>()
        .map_err(|_| anyhow::anyhow!("Port must be between 1 and 65535"))?;
    ensure!(port > 0, "Port must be between 1 and 65535");
    let host = host.trim();
    let host = host
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(host);
    ensure!(
        !host.contains(':') || host.parse::<std::net::Ipv6Addr>().is_ok(),
        "Enter only the host name or IP address in Host; use Port or Display for the connection number"
    );
    terminator_core::vnc::validate_host(host)?;
    ensure!(
        !host.trim().is_empty() && host.len() <= 255,
        "VNC host must be 1–255 characters"
    );
    Ok((host.to_owned(), port))
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
    options: terminator_core::vnc::VncOptions,
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

    pub(crate) fn vnc_features_supported(&self) -> bool {
        self.state
            .capabilities
            .iter()
            .any(|cap| cap == terminator_core::vnc::FEATURES_CAPABILITY)
    }
    fn session_features(&self, sid: &str) -> bool {
        let Some(session) = self.state.vnc_sessions.iter().find(|s| s.id == sid) else {
            return self.vnc_features_supported();
        };
        if session.generation == self.state.generation {
            return self.vnc_features_supported();
        }
        self.state.generations.iter().any(|g| {
            g.owner.id == session.generation
                && g.capabilities
                    .iter()
                    .any(|c| c == terminator_core::vnc::FEATURES_CAPABILITY)
        })
    }
    pub(crate) fn vnc_captures_shortcuts(&self, ctx: &egui::Context) -> bool {
        self.vnc_views.values().any(|view| {
            view.focused
                && view.capture_shortcuts
                && view.visible_last_frame
                && view
                    .focus_id
                    .is_some_and(|id| ctx.memory(|m| m.has_focus(id)))
        })
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
            port: DEFAULT_CONNECTION_PORT,
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
        if profile.options != terminator_core::vnc::VncOptions::default()
            && !self.vnc_features_supported()
        {
            self.error = Some("Restart the session service to use advanced VNC options".into());
            return;
        }
        if profile.password.is_empty() && !profile.credential_saved {
            let tab_id = id();
            let view = self.vnc_views.entry(format!("draft:{tab_id}")).or_default();
            view.draft_host.clone_from(&profile.host);
            view.draft_port = profile.port.to_string();
            view.draft_display = display_for_port(&view.draft_port);
            view.draft_username.clone_from(&profile.username);
            view.draft_name = profile.display_name();
            view.draft_options = profile.options.clone();
            view.draft_shared = profile.shared;
            view.draft_view_only = profile.view_only;
            view.save_profile = false;
            let tab = Tab::Vnc {
                id: tab_id,
                session: String::new(),
                profile_id: Some(profile.id.clone()),
                host: profile.host.clone(),
                port: profile.port,
                label: profile.display_name(),
            };
            self.place_gui_tab(project, tab, after);
            return;
        }
        let _ = self.jobs.send(Job::rpc(
            Request::VncConnect {
                options: profile.options.clone(),
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
        entry.monitors = decoded.monitors;
        entry.resize_supported = decoded.resize_supported;
        if let Some((cursor, image)) = decoded.cursor
            && (entry.cursor_signature.as_ref() != Some(&cursor.rgba)
                || entry.cursor_hotspot != [cursor.x, cursor.y]
                || image.as_ref().map(|i| i.size)
                    != entry.cursor_texture.as_ref().map(TextureHandle::size))
        {
            entry.cursor_signature = Some(cursor.rgba);
            entry.cursor_hotspot = [cursor.x, cursor.y];
            entry.cursor_texture = image.map(|image| {
                ctx.load_texture(
                    format!("vnc-cursor:{}", decoded.session),
                    image,
                    egui::TextureOptions::NEAREST,
                )
            });
        }
        if decoded.status != VncSessionStatus::Connected {
            entry.held_keys.clear();
            entry.modifier_bits = 0;
            entry.focused = false;
        }

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
        let features = self.session_features(session_id);
        let mut connect_request: Option<PendingVncConnect> = None;
        let mut special_keys = Vec::new();
        let mut disconnect_clicked = false;
        let mut refresh_clicked = false;
        let mut cad_clicked = false;
        let mut save_to_sidebar = false;
        let mut open_sidebar = false;

        {
            let view = self.vnc_views.entry(state_key.clone()).or_default();
            if !view.options_initialized
                && let Some(session) = &daemon_session
            {
                view.draft_options = session.options.clone();
                view.options_initialized = true;
            }
            if !host.is_empty() && view.draft_host == "127.0.0.1" && *host != "127.0.0.1" {
                view.draft_host.clone_from(host);
                view.draft_port = port.to_string();
                view.draft_display = display_for_port(&view.draft_port);
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
                    .tint(appearance::icon_color())
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
                        options: view.draft_options.clone(),
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
                    ui.add_enabled(
                        features,
                        egui::Checkbox::new(
                            &mut view.paste_as_keystrokes,
                            "Paste as keystrokes (up to 4096 characters)",
                        ),
                    );
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
                if appearance::sidebar_action(ui, "Maximize", "Toggle remote fullscreen (F11)")
                    .clicked()
                {
                    if self.vnc_fullscreen.is_some() {
                        self.vnc_fullscreen = None;
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                    } else {
                        self.vnc_fullscreen = Some(Tab::Vnc {
                            id: tab_id.into(),
                            session: session_id.clone(),
                            profile_id: profile_id.clone(),
                            host: host.clone(),
                            port: *port,
                            label: label.clone(),
                        });
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
                    }
                }
                appearance::text_menu_button(ui, "Keys", |ui| {
                    ui.checkbox(
                        &mut view.capture_shortcuts,
                        "Capture app shortcuts (F8 to release)",
                    );
                    for (name, key, modifier) in [
                        ("Alt+Tab", 0xFF09, 0xFFE9),
                        ("Command+Tab", 0xFF09, 0xFFEB),
                        ("Super key", 0xFFEB, 0),
                        ("Escape", 0xFF1B, 0),
                    ] {
                        if ui.button(name).clicked() {
                            let mut events = Vec::new();
                            if modifier != 0 {
                                events.push(VncInputEvent::Key {
                                    keycode: modifier,
                                    down: true,
                                });
                            }
                            events.extend([
                                VncInputEvent::Key {
                                    keycode: key,
                                    down: true,
                                },
                                VncInputEvent::Key {
                                    keycode: key,
                                    down: false,
                                },
                            ]);
                            if modifier != 0 {
                                events.push(VncInputEvent::Key {
                                    keycode: modifier,
                                    down: false,
                                });
                            }
                            special_keys.extend(events);
                            ui.close();
                        }
                    }
                });
                if view.monitors.len() > 1 {
                    appearance::text_menu_button(ui, "Monitor", |ui| {
                        ui.selectable_value(&mut view.selected_monitor, None, "All monitors");
                        for monitor in &view.monitors {
                            ui.selectable_value(
                                &mut view.selected_monitor,
                                Some(monitor.id),
                                format!(
                                    "Monitor {} ({} × {})",
                                    monitor.id, monitor.width, monitor.height
                                ),
                            );
                        }
                    });
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
            });
        }

        if !special_keys.is_empty() && !daemon_session.as_ref().is_some_and(|s| s.view_only) {
            self.send(Request::VncInput {
                session: session_id.clone(),
                events: special_keys,
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
                options: self
                    .vnc_views
                    .get(&state_key)
                    .map_or_else(Default::default, |v| v.draft_options.clone()),
                credential_saved: false,
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
            self.send(Request::VncProfileSave {
                profile,
                forget_password: false,
            });
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

        if let Some(record) = &daemon_session {
            let security = match record.options.transport {
                terminator_core::vnc::Transport::Direct => "Direct VNC · unencrypted transport",
                terminator_core::vnc::Transport::Tls => "TLS · certificate verification required",
                terminator_core::vnc::Transport::Ssh => "SSH · known host key required",
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
            .get(&state_key)
            .and_then(|v| v.error.as_deref())
        {
            ui.colored_label(appearance::color(&self.theme.status_failed), error);
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
                    let enter =
                        (host_resp.lost_focus() || endpoint_lost_focus || pass_resp.lost_focus())
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
                                    options: view.draft_options.clone(),
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
                settings::options(ui, &mut view.draft_options, features);
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
            options: req_options,
        }) = connect_request
            && let Some(project) = self.selected.clone()
        {
            if req_options != terminator_core::vnc::VncOptions::default() && !features {
                self.error = Some("Restart the session service to use advanced VNC options".into());
                return;
            }
            host.clone_from(&req_host);
            *port = req_port;
            if let Some(l) = &req_label {
                label.clone_from(l);
            }
            let effective_profile_id = if should_save && req_profile.is_none() {
                let pid = id();
                let profile = VncProfile {
                    options: req_options.clone(),
                    credential_saved: false,
                    id: pid.clone(),
                    name: label.clone(),
                    host: req_host.clone(),
                    port: req_port,
                    username: req_user.clone(),
                    password: String::new(),
                    shared: req_shared,
                    view_only: req_view_only,
                    last_connected: 0,
                };
                self.send(Request::VncProfileSave {
                    profile,
                    forget_password: false,
                });
                Some(pid)
            } else {
                req_profile
            };
            profile_id.clone_from(&effective_profile_id);
            let _ = self.jobs.send(Job::VncConnectTab {
                tab_id: tab_id.to_owned(),
                request: Request::VncConnect {
                    options: req_options,
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
        if features {
            transfer::panel(self, ui, &sid, view_only);
        }
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
pub(super) struct VncScreenOutcome {
    events: Vec<VncInputEvent>,
    activated: bool,
}

use egui::Widget;

#[cfg(test)]
mod tests;
