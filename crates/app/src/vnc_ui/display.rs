use super::*;
/// One remote screen inside its pane: painting, pointer mapping, wheel,
/// keyboard events, and the pointer/key release bookkeeping that stops a
/// key or button from sticking when focus moves away mid-gesture.
pub(crate) struct VncScreenInput<'a> {
    pub(crate) ui: &'a mut egui::Ui,
    pub(crate) view: &'a mut VncViewState,
    pub(crate) texture: &'a TextureHandle,
    pub(crate) view_only: bool,
    pub(crate) paste_shortcut: VncPasteShortcut,
}

pub(super) fn vnc_screen(input: VncScreenInput<'_>) -> VncScreenOutcome {
    let VncScreenInput {
        ui,
        view,
        texture,
        view_only,
        paste_shortcut,
    } = input;
    let mut outcome = VncScreenOutcome::default();
    let monitor = view
        .selected_monitor
        .and_then(|id| view.monitors.iter().find(|m| m.id == id))
        .cloned();
    let geometry = ScreenGeometry::new(texture.size_vec2(), monitor.as_ref(), ui.available_size());
    // Output area: scaled to the pane ("Fit"), or shown at actual pixel
    // size inside scroll bars ("100%").
    let scroll_output = egui::ScrollArea::both()
        .id_salt("vnc-screen")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let mut pane = ScreenPane {
                view: &mut *view,
                geometry,
                events: Vec::new(),
                activated: false,
            };
            let response = pane.show(ui, texture);
            (response, pane.events, pane.activated)
        });
    let (response, pointer_events, activated) = scroll_output.inner;
    outcome.activated = activated;
    view.focus_id = Some(response.id);
    let keyboard_events = ScreenKeys::collect(ScreenKeysInput {
        ui,
        view: &mut *view,
        response: &response,
        paste_shortcut,
    });
    if !view_only && monitor.is_none() {
        ScreenGeometry::request_resize(view, geometry.available, &mut outcome.events);
    }
    if !view_only {
        outcome.events.extend(pointer_events);
        outcome.events.extend(keyboard_events);
    }
    outcome
}

/// Which part of the remote framebuffer the pane shows, and how much room
/// the pane has.
#[derive(Clone, Copy)]
struct ScreenGeometry {
    origin: egui::Vec2,
    tex_size: egui::Vec2,
    uv: egui::Rect,
    available: egui::Vec2,
}

impl ScreenGeometry {
    fn new(
        full_size: egui::Vec2,
        monitor: Option<&terminator_core::vnc::VncMonitor>,
        available: egui::Vec2,
    ) -> Self {
        let (origin, tex_size) = monitor.map_or((egui::vec2(0.0, 0.0), full_size), |m| {
            (
                egui::vec2(f32::from(m.x), f32::from(m.y)),
                egui::vec2(f32::from(m.width), f32::from(m.height)),
            )
        });
        let uv = egui::Rect::from_min_max(
            egui::pos2(origin.x / full_size.x, origin.y / full_size.y),
            egui::pos2(
                (origin.x + tex_size.x) / full_size.x,
                (origin.y + tex_size.y) / full_size.y,
            ),
        );
        Self {
            origin,
            tex_size,
            uv,
            available,
        }
    }

    fn display_size(&self, fit: bool) -> egui::Vec2 {
        let (available, tex_size) = (self.available, self.tex_size);
        if fit && tex_size.x > 0.0 && tex_size.y > 0.0 {
            let scale = (available.x / tex_size.x)
                .min(available.y / tex_size.y)
                .max(0.05);
            egui::vec2(tex_size.x * scale, tex_size.y * scale)
        } else {
            tex_size
        }
    }

    /// Pointer mapping: window point to remote pixel, clamped to the
    /// screen edge so a drag that leaves the pane keeps tracking.
    fn remote_at(
        &self,
        view: &VncViewState,
        image_rect: egui::Rect,
        pos: egui::Pos2,
    ) -> Option<(u16, u16)> {
        if view.width == 0 || view.height == 0 || !image_rect.is_positive() {
            return None;
        }
        let (origin, tex_size) = (self.origin, self.tex_size);
        let rel_x = ((pos.x - image_rect.left()) / image_rect.width()).clamp(0.0, 1.0);
        let rel_y = ((pos.y - image_rect.top()) / image_rect.height()).clamp(0.0, 1.0);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "Clamped to the framebuffer size"
        )]
        let x = (origin.x + rel_x * (tex_size.x - 1.0).max(0.0)) as u16;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "Clamped to the framebuffer size"
        )]
        let y = (origin.y + rel_y * (tex_size.y - 1.0).max(0.0)) as u16;
        Some((x, y))
    }

    /// Ask the remote to match the pane size once it has been stable for a
    /// moment.
    fn request_resize(
        view: &mut VncViewState,
        available: egui::Vec2,
        events: &mut Vec<VncInputEvent>,
    ) {
        if !(view.draft_options.remote_resize && view.resize_supported) {
            return;
        }
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "Clamped to 64..=8192"
        )]
        let target = (
            available.x.round().clamp(64.0, 8192.0) as u16,
            available.y.round().clamp(64.0, 8192.0) as u16,
        );
        if view.resize_target != Some(target) {
            view.resize_target = Some(target);
            view.resize_changed = Some(Instant::now());
        }
        if view
            .resize_changed
            .is_some_and(|at| at.elapsed() >= Duration::from_millis(350))
            && view.resize_sent != Some(target)
            && target != (view.width, view.height)
        {
            events.push(VncInputEvent::Resize {
                width: target.0,
                height: target.1,
            });
            view.resize_sent = Some(target);
        }
    }
}

/// The scroll area's contents: the painted screen and its pointer events.
struct ScreenPane<'a> {
    view: &'a mut VncViewState,
    geometry: ScreenGeometry,
    events: Vec<VncInputEvent>,
    activated: bool,
}

impl ScreenPane<'_> {
    fn show(&mut self, ui: &mut egui::Ui, texture: &TextureHandle) -> egui::Response {
        let display_size = self.geometry.display_size(self.view.fit);
        let size = if self.view.fit {
            self.geometry.available.max(egui::Vec2::ZERO)
        } else {
            display_size
        };
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        ui.painter()
            .rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
        let image_rect = egui::Rect::from_center_size(rect.center(), display_size);
        egui::Image::new(texture)
            .uv(self.geometry.uv)
            .paint_at(ui, image_rect);
        self.paint_remote_cursor(ui, &response, image_rect);
        if response.clicked() || response.drag_started() {
            response.request_focus();
            self.activated = true;
        }
        self.pointer_events(ui, &response, image_rect);
        response
    }

    fn paint_remote_cursor(
        &self,
        ui: &egui::Ui,
        response: &egui::Response,
        image_rect: egui::Rect,
    ) {
        let view = &*self.view;
        if !(view.draft_options.show_remote_cursor && view.cursor_signature.is_some()) {
            return;
        }
        let Some(pos) = response.hover_pos().filter(|pos| image_rect.contains(*pos)) else {
            return;
        };
        ui.ctx().set_cursor_icon(egui::CursorIcon::None);
        let Some(cursor) = &view.cursor_texture else {
            return;
        };
        let scale = image_rect.width() / self.geometry.tex_size.x;
        let cursor_size = cursor.size_vec2();
        let size = egui::vec2(cursor_size.x * scale, cursor_size.y * scale);
        let min = egui::pos2(
            pos.x - f32::from(view.cursor_hotspot[0]) * scale,
            pos.y - f32::from(view.cursor_hotspot[1]) * scale,
        );
        egui::Image::new(cursor).paint_at(ui, egui::Rect::from_min_size(min, size));
    }

    fn pointer_events(&mut self, ui: &egui::Ui, response: &egui::Response, image_rect: egui::Rect) {
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
        if let Some((x, y)) =
            pos.and_then(|pos| self.geometry.remote_at(self.view, image_rect, pos))
        {
            self.move_pointer(ui, (x, y), buttons);
        } else if let Some((x, y, held)) = self.view.last_pointer
            && held != 0
        {
            // The press happened here but the release landed outside the
            // pane: release the button, or the remote keeps it down.
            self.view.last_pointer = Some((x, y, 0));
            self.events
                .push(VncInputEvent::Pointer { x, y, buttons: 0 });
        }
    }

    fn move_pointer(&mut self, ui: &egui::Ui, (x, y): (u16, u16), buttons: u8) {
        let next = (x, y, buttons);
        if self.view.last_pointer != Some(next) {
            self.view.last_pointer = Some(next);
            self.events.push(VncInputEvent::Pointer { x, y, buttons });
        }
        // Wheel: forwarded only when the remote fills the pane; at
        // 100% the local scroll bars own the wheel.
        let scroll_delta = ui.input(|input| input.smooth_scroll_delta);
        if self.view.fit && scroll_delta.y.abs() > 1.0 {
            let wheel_bit = if scroll_delta.y > 0.0 { 8u8 } else { 16u8 };
            self.events.push(VncInputEvent::Pointer {
                x,
                y,
                buttons: buttons | wheel_bit,
            });
            self.events.push(VncInputEvent::Pointer { x, y, buttons });
        }
    }
}

struct ScreenKeysInput<'a> {
    ui: &'a egui::Ui,
    view: &'a mut VncViewState,
    response: &'a egui::Response,
    paste_shortcut: VncPasteShortcut,
}

/// Keyboard and clipboard events for a focused or hovered screen.
struct ScreenKeys<'a> {
    view: &'a mut VncViewState,
    paste_shortcut: VncPasteShortcut,
    events: Vec<VncInputEvent>,
}

impl ScreenKeys<'_> {
    fn collect(input: ScreenKeysInput<'_>) -> Vec<VncInputEvent> {
        let ScreenKeysInput {
            ui,
            view,
            response,
            paste_shortcut,
        } = input;
        let modifiers = ui.input(|input| modifier_bits(input.modifiers));
        let window_focused = ui.ctx().input(|input| input.focused);
        let focused = response.has_focus() && window_focused;
        let mut keys = ScreenKeys {
            view,
            paste_shortcut,
            events: Vec::new(),
        };
        if focused || response.hovered() || response.dragged() {
            keys.sync_modifiers(modifiers);
        }
        if focused {
            ui.input(|input| keys.read_events(&input.events));
        }
        // Leaving the pane (or the window) mid-chord must not leave keys or
        // modifiers stuck down on the remote.
        if keys.view.focused && !focused {
            keys.release_all();
        }
        keys.view.focused = focused;
        keys.events
    }

    /// Modifiers reach the remote as their own key presses, so
    /// Shift+click selects and Ctrl+C interrupts there too.
    fn sync_modifiers(&mut self, modifiers: u8) {
        if self.view.modifier_bits == modifiers {
            return;
        }
        for (keysym, down) in modifier_transitions(self.view.modifier_bits, modifiers) {
            self.events.push(VncInputEvent::Key {
                keycode: keysym,
                down,
            });
        }
        self.view.modifier_bits = modifiers;
    }

    fn read_events(&mut self, events: &[egui::Event]) {
        let ime_commit = events
            .iter()
            .any(|event| matches!(event, egui::Event::Ime(egui::ImeEvent::Commit(_))));
        for event in events {
            match event {
                egui::Event::Key {
                    key,
                    pressed,
                    modifiers,
                    ..
                } => self.on_key(*key, *pressed, *modifiers),
                egui::Event::Text(text) if !ime_commit => self.type_text(text),
                egui::Event::Ime(egui::ImeEvent::Commit(text)) => self.type_text(text),
                egui::Event::Paste(_) | egui::Event::Copy | egui::Event::Cut => {
                    self.on_clipboard(event);
                }
                _ => {}
            }
        }
    }

    fn on_key(&mut self, key: egui::Key, pressed: bool, modifiers: egui::Modifiers) {
        let keysym = egui_key_to_x11(key).or_else(|| {
            (modifiers.ctrl || modifiers.command)
                .then(|| accelerator_keysym(key))
                .flatten()
        });
        let Some(keysym) = keysym else {
            return;
        };
        if pressed {
            if !self.view.held_keys.contains(&keysym) {
                self.view.held_keys.push(keysym);
            }
        } else {
            self.view.held_keys.retain(|held| *held != keysym);
        }
        self.events.push(VncInputEvent::Key {
            keycode: keysym,
            down: pressed,
        });
    }

    fn type_text(&mut self, text: &str) {
        for keycode in text.chars().filter_map(keyboard::text_keysym) {
            self.events.extend([
                VncInputEvent::Key {
                    keycode,
                    down: true,
                },
                VncInputEvent::Key {
                    keycode,
                    down: false,
                },
            ]);
        }
    }

    fn on_clipboard(&mut self, event: &egui::Event) {
        if let egui::Event::Paste(text) = event
            && !self.accept_paste(text)
        {
            return;
        }
        self.events.extend(remote_clipboard_events(
            event,
            self.paste_shortcut,
            self.view.modifier_bits,
        ));
    }

    /// True when the paste should go out as the remote paste shortcut;
    /// otherwise it was typed out, or refused with an error.
    fn accept_paste(&mut self, text: &str) -> bool {
        let view = &mut *self.view;
        if !view.draft_options.clipboard.send() {
            view.error = Some("Sending clipboard text is disabled for this connection".into());
            return false;
        }
        if view.paste_as_keystrokes {
            if text.chars().count() > 4096 {
                view.error = Some("Paste as keystrokes is limited to 4096 characters".into());
            } else {
                self.events.push(VncInputEvent::TypeText(text.to_owned()));
            }
            return false;
        }
        if text.len() > terminator_core::vnc::MAX_CLIPBOARD
            || text.chars().any(|c| u32::from(c) > 255)
        {
            view.error = Some("The remote clipboard supports Latin-1 text up to 256 KiB. Select Paste as keystrokes for Unicode text.".into());
            return false;
        }
        true
    }

    fn release_all(&mut self) {
        for (bit, keysym) in MODIFIER_KEYSYMS {
            if self.view.modifier_bits & bit != 0 {
                self.events.push(VncInputEvent::Key {
                    keycode: keysym,
                    down: false,
                });
            }
        }
        self.view.modifier_bits = 0;
        for keysym in self.view.held_keys.drain(..) {
            self.events.push(VncInputEvent::Key {
                keycode: keysym,
                down: false,
            });
        }
    }
}
