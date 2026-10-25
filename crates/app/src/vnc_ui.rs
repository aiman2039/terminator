mod clipboard;
mod display;
mod frame;
mod keyboard;
mod profiles;
mod settings;
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
            cursor: None,
            monitors: Vec::new(),
            resize_supported: false,
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
    #[test]
    fn a_selected_monitor_maps_pointer_input_to_that_monitor() {
        let ctx = egui::Context::default();
        let mut view = VncViewState {
            width: 800,
            height: 600,
            selected_monitor: Some(2),
            monitors: vec![
                terminator_core::vnc::VncMonitor {
                    id: 1,
                    x: 0,
                    y: 0,
                    width: 400,
                    height: 600,
                },
                terminator_core::vnc::VncMonitor {
                    id: 2,
                    x: 400,
                    y: 0,
                    width: 400,
                    height: 600,
                },
            ],
            ..Default::default()
        };
        let texture = ctx.load_texture(
            "monitors",
            ColorImage::filled([800, 600], Color32::BLACK),
            egui::TextureOptions::NEAREST,
        );
        let mut events = Vec::new();
        for pressed in [false, true] {
            let pos = egui::pos2(400.0, 300.0);
            let input = if pressed {
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            } else {
                Vec::new()
            };
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    focused: true,
                    events: input,
                    ..Default::default()
                },
                |ui| {
                    events = vnc_screen(
                        ui,
                        &mut view,
                        &texture,
                        false,
                        VncPasteShortcut::ShiftInsert,
                    )
                    .events;
                },
            );
            output.textures_delta.clear();
        }
        assert!(events.iter().any(|e|matches!(e,VncInputEvent::Pointer{x,y,buttons:1} if (590..=610).contains(x)&&(290..=310).contains(y))));
    }
}
