use super::*;
use view::{VncTabFields, VncToolbarActions};

/// One frame of the VNC toolbar row: title, session actions, and the
/// clipboard, keys, monitor, and scaling controls.
pub(super) struct VncToolbar<'a> {
    pub(super) fields: &'a VncTabFields<'a>,
    pub(super) session: Option<&'a VncSession>,
    pub(super) features: bool,
    pub(super) status_color: Option<egui::Color32>,
    pub(super) view: &'a mut VncViewState,
    pub(super) paste_shortcut: &'a mut VncPasteShortcut,
    pub(super) fullscreen: &'a mut Option<Tab>,
    pub(super) actions: VncToolbarActions,
}

impl VncToolbar<'_> {
    pub(super) fn show(mut self, ui: &mut egui::Ui) -> VncToolbarActions {
        ui.horizontal_wrapped(|ui| {
            self.title(ui);
            // Actions stay one row of same-size icon buttons; the two
            // view-mode toggles trail them so the icon row does not mix
            // in differently sized widgets.
            self.session_buttons(ui);
            self.sidebar_buttons(ui);
            ui.separator();
            self.clipboard_menu(ui);
            self.fullscreen_toggle(ui);
            self.keys_menu(ui);
            self.monitor_menu(ui);
            self.scale_menu(ui);
            self.speed_chip(ui);
            self.capture_chip(ui);
        });
        self.ctrl_alt_del_bar(ui);
        self.actions
    }

    fn title(&self, ui: &mut egui::Ui) {
        let fields = self.fields;
        egui::Image::new(icons::source("Monitor"))
            .tint(appearance::icon_color())
            .fit_to_exact_size(egui::vec2(16.0, 16.0))
            .ui(ui);
        ui.strong(if fields.label.is_empty() {
            "VNC Viewer"
        } else {
            fields.label
        });
        if !fields.host.is_empty() {
            ui.weak(format!("{}:{}", fields.host, fields.port));
        }
        if let (Some(ds), Some(color)) = (self.session, self.status_color) {
            ui.colored_label(color, format!("● {}", ds.status.label()));
        }
        if self.view.width > 0 && self.view.height > 0 {
            ui.weak(format!("{} × {}", self.view.width, self.view.height));
        }
    }

    fn session_buttons(&mut self, ui: &mut egui::Ui) {
        if self
            .session
            .is_some_and(|s| s.status == VncSessionStatus::Connected)
        {
            self.actions.refresh =
                appearance::sidebar_action(ui, "RefreshCw", "Refresh remote screen").clicked();
            if appearance::sidebar_action(ui, "Keyboard", "Send Ctrl+Alt+Del to remote host")
                .clicked()
            {
                self.view.confirm_ctrl_alt_del = true;
            }
            self.actions.disconnect =
                appearance::sidebar_action(ui, "CircleStop", "Disconnect").clicked();
        } else if self
            .session
            .is_some_and(|s| s.status == VncSessionStatus::Connecting)
        {
            // Connecting or waiting to retry: stopping must stay possible.
            self.actions.disconnect =
                appearance::sidebar_action(ui, "CircleStop", "Disconnect").clicked();
        } else if !self.fields.host.is_empty()
            && appearance::sidebar_action(ui, "Play", "Reconnect").clicked()
        {
            self.actions.connect = Some(self.reconnect_request());
        }
    }

    fn reconnect_request(&self) -> PendingVncConnect {
        let view = &*self.view;
        PendingVncConnect {
            profile_id: self.fields.profile_id.clone(),
            host: self.fields.host.to_owned(),
            port: self.fields.port,
            username: view.draft_username.clone(),
            password: view.draft_password.clone(),
            shared: view.draft_shared,
            view_only: view.draft_view_only,
            label: Some(self.fields.label.to_owned()),
            save_profile: false,
            options: view.draft_options.clone(),
        }
    }

    fn sidebar_buttons(&mut self, ui: &mut egui::Ui) {
        if !self.fields.host.is_empty()
            && self.fields.profile_id.is_none()
            && appearance::sidebar_action(ui, "Save", "Save host to the VNC sidebar").clicked()
        {
            self.actions.save_to_sidebar = true;
        }
        if appearance::sidebar_action(ui, "PanelRight", "Open the VNC sidebar").clicked() {
            self.actions.open_sidebar = true;
        }
    }

    fn clipboard_menu(&mut self, ui: &mut egui::Ui) {
        let features = self.features;
        let view = &mut *self.view;
        let paste_shortcut = &mut *self.paste_shortcut;
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
                    .selectable_value(paste_shortcut, shortcut, label)
                    .clicked()
                {
                    ui.close();
                }
            }
        });
    }

    /// Toolbar button, or F11 while this remote screen has focus (F11 in
    /// fullscreen is handled earlier, by the app's escape keys).
    fn fullscreen_toggle(&mut self, ui: &mut egui::Ui) {
        let clicked =
            appearance::sidebar_action(ui, "Maximize", "Toggle remote fullscreen (F11)").clicked();
        let f11 = self.fullscreen.is_none()
            && self.view.focused
            && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F11));
        if !(clicked || f11) {
            return;
        }
        if self.fullscreen.is_some() {
            *self.fullscreen = None;
            if let Some(previous) = self.view.capture_before_fullscreen.take() {
                self.view.capture_shortcuts = previous;
            }
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            return;
        }
        // Fullscreen sends app shortcuts to the remote; leaving restores.
        self.view.capture_before_fullscreen = Some(self.view.capture_shortcuts);
        self.view.capture_shortcuts = true;
        let fields = self.fields;
        *self.fullscreen = Some(Tab::Vnc {
            id: fields.tab_id.into(),
            session: fields.session_id.to_owned(),
            profile_id: fields.profile_id.clone(),
            host: fields.host.to_owned(),
            port: fields.port,
            label: fields.label.to_owned(),
        });
        ui.ctx()
            .send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
    }

    fn keys_menu(&mut self, ui: &mut egui::Ui) {
        let view = &mut *self.view;
        let special_keys = &mut self.actions.special_keys;
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
                    special_keys.extend(Self::key_press_events(key, modifier));
                    ui.close();
                }
            }
        });
    }

    /// Press and release `key`, wrapped in `modifier` when it is non-zero.
    fn key_press_events(key: u32, modifier: u32) -> Vec<VncInputEvent> {
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
        events
    }

    pub(super) fn ctrl_alt_del_events() -> Vec<VncInputEvent> {
        [
            (0xFFE3, true),
            (0xFFE9, true),
            (0xFFFF, true),
            (0xFFFF, false),
            (0xFFE9, false),
            (0xFFE3, false),
        ]
        .into_iter()
        .map(|(keycode, down)| VncInputEvent::Key { keycode, down })
        .collect()
    }

    fn monitor_menu(&mut self, ui: &mut egui::Ui) {
        if self.view.monitors.len() <= 1 {
            return;
        }
        let view = &mut *self.view;
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

    /// Speed indicator while connected; hover for the details.
    fn speed_chip(&self, ui: &mut egui::Ui) {
        if !self
            .session
            .is_some_and(|s| s.status == VncSessionStatus::Connected)
        {
            return;
        }
        ui.separator();
        ui.label(
            RichText::new(self.view.speed.label())
                .size(11.0)
                .color(ui.visuals().weak_text_color()),
        )
        .on_hover_text(self.view.speed.detail());
    }

    /// Scale modes, plus resizing the remote desktop to the pane when the
    /// session was started with remote resize allowed.
    fn scale_menu(&mut self, ui: &mut egui::Ui) {
        let before = self.view.scale;
        let resize_allowed = self.session.is_some_and(|s| s.options.remote_resize);
        let view = &mut *self.view;
        appearance::text_menu_button(ui, before.label(), |ui| {
            for scale in VncScale::ALL {
                if ui
                    .selectable_value(&mut view.scale, scale, scale.label())
                    .clicked()
                {
                    ui.close();
                }
            }
            ui.separator();
            ui.add_enabled(
                resize_allowed,
                egui::Checkbox::new(
                    &mut view.draft_options.remote_resize,
                    "Resize remote desktop to the pane",
                ),
            )
            .on_disabled_hover_text(
                "Turn on in Properties > Options; applies when you connect again",
            );
        });
        // The texture filter follows the mode; reload it with the new one.
        if self.view.scale != before {
            self.view.force_full = true;
        }
    }

    /// Shown while app shortcuts go to the remote.
    fn capture_chip(&self, ui: &mut egui::Ui) {
        if self.view.capture_shortcuts && self.view.focused {
            ui.label(
                RichText::new("Capturing shortcuts · F8 to release")
                    .size(11.0)
                    .color(ui.visuals().warn_fg_color),
            );
        }
    }

    /// In-pane confirmation for Ctrl+Alt+Del, which can log out or lock
    /// the remote desktop.
    fn ctrl_alt_del_bar(&mut self, ui: &mut egui::Ui) {
        if !self.view.confirm_ctrl_alt_del {
            return;
        }
        let host = self.fields.host;
        ui.horizontal(|ui| {
            ui.label(format!("Send Ctrl+Alt+Del to {host}?"));
            if form::primary_button(ui, "Send").clicked() {
                self.actions.ctrl_alt_del = true;
                self.view.confirm_ctrl_alt_del = false;
            }
            if ui.button("Cancel").clicked() {
                self.view.confirm_ctrl_alt_del = false;
            }
        });
    }
}

use super::scale::VncScale;
