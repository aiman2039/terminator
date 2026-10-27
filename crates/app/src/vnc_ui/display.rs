use super::*;
/// One remote screen inside its pane: painting, pointer mapping, wheel,
/// keyboard events, and the pointer/key release bookkeeping that stops a
/// key or button from sticking when focus moves away mid-gesture.
pub(super) fn vnc_screen(
    ui: &mut egui::Ui,
    view: &mut VncViewState,
    texture: &TextureHandle,
    view_only: bool,
    paste_shortcut: VncPasteShortcut,
) -> VncScreenOutcome {
    let mut outcome = VncScreenOutcome::default();
    let full_size = texture.size_vec2();
    let monitor = view
        .selected_monitor
        .and_then(|id| view.monitors.iter().find(|m| m.id == id))
        .cloned();
    let (origin, tex_size) = monitor
        .as_ref()
        .map_or((egui::vec2(0.0, 0.0), full_size), |m| {
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
            .rect_filled(rect, 0.0, ui.visuals().extreme_bg_color);
        image_rect = egui::Rect::from_center_size(rect.center(), display_size);
        egui::Image::new(texture).uv(uv).paint_at(ui, image_rect);
        if view.draft_options.show_remote_cursor
            && view.cursor_signature.is_some()
            && let Some(pos) = response.hover_pos().filter(|pos| image_rect.contains(*pos))
        {
            ui.ctx().set_cursor_icon(egui::CursorIcon::None);
            if let Some(cursor) = &view.cursor_texture {
                let scale = image_rect.width() / tex_size.x;
                let cursor_size = cursor.size_vec2();
                let size = egui::vec2(cursor_size.x * scale, cursor_size.y * scale);
                let min = egui::pos2(
                    pos.x - f32::from(view.cursor_hotspot[0]) * scale,
                    pos.y - f32::from(view.cursor_hotspot[1]) * scale,
                );
                egui::Image::new(cursor).paint_at(ui, egui::Rect::from_min_size(min, size));
            }
        }

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

    view.focus_id = Some(scroll_output.inner.id);
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
            let ime_commit=input.events.iter().any(|event|matches!(event,egui::Event::Ime(egui::ImeEvent::Commit(_))));
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
                    egui::Event::Text(text) if !ime_commit => {
                        for keycode in text.chars().filter_map(keyboard::text_keysym) {
                            keyboard_events.extend([VncInputEvent::Key{keycode,down:true},VncInputEvent::Key{keycode,down:false}]);
                        }
                    }
                    egui::Event::Ime(egui::ImeEvent::Commit(text))=>{
                        for keycode in text.chars().filter_map(keyboard::text_keysym) {
                            keyboard_events.extend([VncInputEvent::Key{keycode,down:true},VncInputEvent::Key{keycode,down:false}]);
                        }
                    }
                    egui::Event::Paste(_) | egui::Event::Copy | egui::Event::Cut => {
                        if let egui::Event::Paste(text)=event {
                            if !view.draft_options.clipboard.send(){view.error=Some("Sending clipboard text is disabled for this connection".into());continue;}
                            if view.paste_as_keystrokes {
                                if text.chars().count()>4096{view.error=Some("Paste as keystrokes is limited to 4096 characters".into());}
                                else{keyboard_events.push(VncInputEvent::TypeText(text.clone()));}
                                continue;
                            }
                            if text.len()>terminator_core::vnc::MAX_CLIPBOARD||text.chars().any(|c|u32::from(c)>255){view.error=Some("The remote clipboard supports Latin-1 text up to 256 KiB. Select Paste as keystrokes for Unicode text.".into());continue;}
                        }
                        keyboard_events.extend(remote_clipboard_events(event,paste_shortcut,view.modifier_bits));
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

    if !view_only && view.draft_options.remote_resize && view.resize_supported && monitor.is_none()
    {
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
            outcome.events.push(VncInputEvent::Resize {
                width: target.0,
                height: target.1,
            });
            view.resize_sent = Some(target);
        }
    }
    if !view_only {
        outcome.events.extend(pointer_events);
        outcome.events.extend(keyboard_events);
    }
    outcome
}
