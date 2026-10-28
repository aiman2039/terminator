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
            self.scale_toggle(ui);
        });
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
            self.actions.ctrl_alt_del =
                appearance::sidebar_action(ui, "Keyboard", "Send Ctrl+Alt+Del to remote host")
                    .clicked();
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

    fn fullscreen_toggle(&mut self, ui: &mut egui::Ui) {
        if !appearance::sidebar_action(ui, "Maximize", "Toggle remote fullscreen (F11)").clicked() {
            return;
        }
        if self.fullscreen.is_some() {
            *self.fullscreen = None;
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            return;
        }
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

    fn scale_toggle(&mut self, ui: &mut egui::Ui) {
        if ui
            .selectable_label(self.view.fit, "Fit")
            .on_hover_text("Scale remote screen to pane")
            .clicked()
        {
            self.view.fit = true;
        }
        if ui
            .selectable_label(!self.view.fit, "100%")
            .on_hover_text("Actual pixel size")
            .clicked()
        {
            self.view.fit = false;
        }
    }
}
