//! Painting a remote window tab and turning egui input into bridge input.
//! One app pixel is one point: most X11 apps do not scale themselves, so
//! 1:1 with physical pixels would make them tiny on HiDPI screens. The
//! tab's size in points is the size the app is asked to take.

use super::*;
use eframe::egui::{
    Event, EventFilter, Modifiers, MouseWheelUnit, PointerButton, Pos2, Rect, Sense,
    TextureOptions, Vec2,
};
use terminator_core::remote_apps::{PointerButton as RemoteButton, RemoteWindow};

/// A size must hold this long before it is sent, so a drag sends one resize.
const RESIZE_DEBOUNCE: Duration = Duration::from_millis(200);
/// App pixels per egui point (see the module docs).
const APP_PIXELS_PER_POINT: f32 = 1.0;
/// Pixels per wheel line for smooth scrolling.
const LINE_PIXELS: f64 = 15.0;

#[derive(Default)]
pub(crate) struct WindowView {
    texture: Option<egui::TextureHandle>,
    /// Size and focus of the last configure sent.
    sent: Option<(u32, u32, bool)>,
    /// A wanted configure, and since when.
    wanted: Option<((u32, u32, bool), Instant)>,
    held: Modifiers,
    keys_down: Vec<u32>,
    focused: bool,
}

impl WindowView {
    pub(super) fn set_image(&mut self, ctx: &egui::Context, name: String, image: ColorImage) {
        match &mut self.texture {
            Some(texture) => texture.set(image, TextureOptions::NEAREST),
            None => {
                self.texture = Some(ctx.load_texture(name, image, TextureOptions::NEAREST));
            }
        }
    }

    pub(super) fn has_focus(&self) -> bool {
        self.focused
    }

    /// The configure to send now, if the wanted one held long enough.
    fn configure_due(
        &mut self,
        wanted: (u32, u32, bool),
        now: Instant,
    ) -> Option<(u32, u32, bool)> {
        if self.sent == Some(wanted) {
            self.wanted = None;
            return None;
        }
        let since = match self.wanted {
            Some((w, since)) if w == wanted => since,
            _ => {
                self.wanted = Some((wanted, now));
                now
            }
        };
        // Focus changes go at once; sizes wait for the drag to settle.
        let only_focus = self
            .sent
            .is_some_and(|s| (s.0, s.1) == (wanted.0, wanted.1));
        if only_focus || now.duration_since(since) >= RESIZE_DEBOUNCE {
            self.sent = Some(wanted);
            self.wanted = None;
            return Some(wanted);
        }
        None
    }
}

/// The egui ID of a remote window's pane; it has keyboard focus while the
/// window takes keys.
pub(super) fn pane_id(project: &str, window: u64) -> egui::Id {
    egui::Id::unique(("remote-app", project, window))
}

/// Where one window is drawn in the tab.
struct Placed {
    id: u64,
    rect: Rect,
}

/// Every window drawn in a tab, bottom to top, and the points-to-pixels
/// factor.
struct Geometry {
    placed: Vec<Placed>,
    ppp: f32,
}

impl App {
    pub(crate) fn remote_app_view(&mut self, ui: &mut egui::Ui, project: &str, window: u64) {
        let session = self.remote_apps.sessions.get(project);
        let state = session.and_then(|s| s.state.clone());
        let error = session.and_then(|s| s.error.clone());
        let session_id = session.and_then(|s| s.id.clone());
        let key = (project.to_owned(), window);
        let shown = state.as_ref().and_then(|s| {
            s.windows.iter().find(|w| w.id.0 == window).filter(|_| {
                self.remote_apps
                    .windows
                    .get(&key)
                    .is_some_and(|v| v.texture.is_some())
            })
        });
        let (Some(state), Some(_), Some(session_id)) = (state.as_deref(), shown, session_id) else {
            self.remote_app_placeholder(ui, project, (state.as_deref(), error));
            return;
        };
        let rect = ui.available_rect_before_wrap();
        let id = pane_id(project, window);
        let response = ui.interact(rect, id, Sense::click_and_drag());
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                EventFilter {
                    tab: true,
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                },
            );
        });
        let ppp = APP_PIXELS_PER_POINT;
        let geometry = Geometry {
            placed: place_windows(state, window, (rect.min, ppp)),
            ppp,
        };
        let painter = ui.painter_at(rect);
        for place in &geometry.placed {
            if let Some(texture) = self
                .remote_apps
                .windows
                .get(&(project.to_owned(), place.id))
                .and_then(|v| v.texture.as_ref())
            {
                let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                painter.image(texture.id(), place.rect, uv, egui::Color32::WHITE);
            }
        }
        if let Some(pos) = response.hover_pos()
            && let Some(cursor) = self.remote_apps.cursors.get(project)
        {
            cursor.show(ui, pos, ppp);
        }
        if response.clicked() || response.drag_started() {
            response.request_focus();
            self.focus_non_terminal();
        }
        let focused = response.has_focus() && ui.ctx().input(|i| i.focused);
        let size = (
            physical(rect.width(), ppp),
            physical(rect.height(), ppp),
            focused,
        );
        let mut messages = Vec::new();
        let view = self.remote_apps.windows.entry(key).or_default();
        if let Some((width, height, focused)) = view.configure_due(size, Instant::now()) {
            messages.push(ClientMessage::Configure {
                id: WindowId(window),
                width,
                height,
                scale: 1,
                focused,
            });
        } else if view.wanted.is_some() {
            ui.ctx().request_repaint_after(RESIZE_DEBOUNCE);
        }
        let events = ui.input(|i| i.events.clone());
        let modifiers = ui.input(|i| i.modifiers);
        pointer_messages(&events, &geometry, &response, &mut messages);
        let paste = key_messages(view, (window, focused), (&events, modifiers), &mut messages);
        match paste {
            // One request keeps the order: clipboard first, then the keys.
            Some(text) if text.len() <= terminator_core::remote_apps::MAX_CLIPBOARD => {
                messages.truncate(terminator_core::remote_apps::MAX_INPUT);
                self.send(terminator_core::Request::RemoteAppsClipboard {
                    session: session_id,
                    text,
                    then: messages,
                });
            }
            _ if !messages.is_empty() => self.send_remote_input(session_id, messages),
            _ => {}
        }
    }

    fn remote_app_placeholder(
        &mut self,
        ui: &mut egui::Ui,
        project: &str,
        (state, error): (Option<&RemoteAppsState>, Option<String>),
    ) {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() / 3.0);
            let status = state.map(|s| s.status);
            let text = match status {
                None | Some(RemoteAppsStatus::Connecting) => match state {
                    Some(s) if s.retry_in_seconds > 0 => {
                        format!("Reconnecting in {} s…", s.retry_in_seconds)
                    }
                    _ => "Connecting to the remote app…".to_owned(),
                },
                Some(RemoteAppsStatus::Connected) => "The window is closed on the host.".to_owned(),
                Some(RemoteAppsStatus::Failed) => "Remote apps stopped.".to_owned(),
                Some(RemoteAppsStatus::Disconnected) => "Remote apps are disconnected.".to_owned(),
            };
            ui.label(text);
            if let Some(error) = error.or_else(|| state.and_then(|s| s.error.clone())) {
                ui.label(egui::RichText::new(error).weak());
            }
            if matches!(
                status,
                Some(RemoteAppsStatus::Failed | RemoteAppsStatus::Disconnected)
            ) && ui.button("Connect").clicked()
            {
                self.connect_remote_apps(project);
            }
        });
    }
}

/// The toplevel at `origin` and its popups (and their popups) at their
/// offsets, bottom to top.
#[expect(
    clippy::arithmetic_side_effects,
    clippy::cast_precision_loss,
    reason = "f32 screen geometry; offsets are window pixels."
)]
fn place_windows(state: &RemoteAppsState, window: u64, (origin, ppp): (Pos2, f32)) -> Vec<Placed> {
    let mut placed: Vec<(u64, Pos2)> = vec![(window, origin)];
    // Popups come after their parents in the list (the daemon sorts them),
    // so one pass finds every level.
    for w in &state.windows {
        if let WindowRole::Popup { parent, x, y } = w.role
            && let Some(&(_, at)) = placed.iter().find(|(id, _)| *id == parent.0)
        {
            placed.push((w.id.0, at + Vec2::new(x as f32, y as f32) / ppp));
        }
    }
    placed
        .into_iter()
        .filter_map(|(id, at)| {
            let w: &RemoteWindow = state.windows.iter().find(|w| w.id.0 == id)?;
            let size = Vec2::new(f32::from(w.width), f32::from(w.height)) / ppp;
            Some(Placed {
                id,
                rect: Rect::from_min_size(at, size),
            })
        })
        .collect()
}

fn physical(points: f32, ppp: f32) -> u32 {
    let pixels = (points * ppp).round();
    if pixels.is_finite() && pixels >= 1.0 {
        // In range: checked above, and tabs are far below u32::MAX pixels.
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "Finite, positive and small; checked above."
        )]
        let pixels = pixels.min(16_384.0) as u32;
        pixels
    } else {
        1
    }
}

/// The topmost window under `pos`, and `pos` in its pixels.
#[expect(clippy::arithmetic_side_effects, reason = "f32 screen geometry.")]
fn target(geometry: &Geometry, pos: Pos2) -> Option<(u64, f64, f64)> {
    let placed = &geometry.placed;
    let place = placed
        .iter()
        .rev()
        .find(|p| p.rect.contains(pos))
        .or(placed.first())?;
    let local = (pos - place.rect.min) * geometry.ppp;
    Some((place.id, f64::from(local.x), f64::from(local.y)))
}

fn pointer_messages(
    events: &[Event],
    geometry: &Geometry,
    response: &egui::Response,
    out: &mut Vec<ClientMessage>,
) {
    let placed = &geometry.placed;
    let area = response.rect;
    let pressed = response.is_pointer_button_down_on();
    for event in events {
        match event {
            Event::PointerMoved(pos) if area.contains(*pos) || pressed => {
                if let Some((id, x, y)) = target(geometry, *pos) {
                    out.push(ClientMessage::PointerMotion {
                        id: WindowId(id),
                        x,
                        y,
                    });
                }
            }
            Event::PointerButton {
                pos,
                button,
                pressed: down,
                ..
            } if area.contains(*pos) || !*down => {
                let (Some(button), Some((id, x, y))) =
                    (remote_button(*button), target(geometry, *pos))
                else {
                    continue;
                };
                out.push(ClientMessage::PointerMotion {
                    id: WindowId(id),
                    x,
                    y,
                });
                out.push(ClientMessage::PointerButton {
                    id: WindowId(id),
                    button,
                    pressed: *down,
                });
            }
            Event::MouseWheel { unit, delta, .. } if response.hovered() => {
                let Some((id, _, _)) = response.hover_pos().and_then(|pos| target(geometry, pos))
                else {
                    continue;
                };
                out.push(scroll(WindowId(id), *unit, *delta));
            }
            Event::PointerGone => {
                if let Some(first) = placed.first() {
                    out.push(ClientMessage::PointerLeave {
                        id: WindowId(first.id),
                    });
                }
            }
            _ => {}
        }
    }
}

/// egui wheel deltas are positive up/left; Wayland axes are positive
/// down/right.
fn scroll(id: WindowId, unit: MouseWheelUnit, delta: Vec2) -> ClientMessage {
    let (dx, dy) = (-f64::from(delta.x), -f64::from(delta.y));
    match unit {
        MouseWheelUnit::Point => ClientMessage::Scroll {
            id,
            dx,
            dy,
            discrete_x: 0,
            discrete_y: 0,
        },
        MouseWheelUnit::Line | MouseWheelUnit::Page => ClientMessage::Scroll {
            id,
            dx: dx * LINE_PIXELS,
            dy: dy * LINE_PIXELS,
            discrete_x: clicks(dx),
            discrete_y: clicks(dy),
        },
    }
}

fn clicks(lines: f64) -> i32 {
    let lines = lines.round().clamp(-100.0, 100.0);
    // Clamped to ±100 above.
    #[expect(clippy::cast_possible_truncation, reason = "Clamped to ±100.")]
    let lines = lines as i32;
    lines
}

fn remote_button(button: PointerButton) -> Option<RemoteButton> {
    Some(match button {
        PointerButton::Primary => RemoteButton::Left,
        PointerButton::Secondary => RemoteButton::Right,
        PointerButton::Middle => RemoteButton::Middle,
        PointerButton::Extra1 => RemoteButton::Back,
        PointerButton::Extra2 => RemoteButton::Forward,
    })
}

/// Keys while the tab has focus; on focus loss, releases what is held so
/// the app does not see a stuck key. Returns the text of a paste, which
/// must reach the app's clipboard before the paste key.
fn key_messages(
    view: &mut WindowView,
    (window, focused): (u64, bool),
    (events, modifiers): (&[Event], Modifiers),
    out: &mut Vec<ClientMessage>,
) -> Option<String> {
    let id = WindowId(window);
    let key = |keycode: u32, pressed: bool| ClientMessage::Key {
        id,
        keycode,
        pressed,
    };
    if !focused {
        if view.focused {
            out.extend(view.keys_down.drain(..).map(|code| key(code, false)));
            out.extend(
                keys::modifier_changes(view.held, Modifiers::NONE)
                    .into_iter()
                    .map(|(code, pressed)| key(code, pressed)),
            );
            view.held = Modifiers::NONE;
        }
        view.focused = false;
        return None;
    }
    view.focused = true;
    out.extend(
        keys::modifier_changes(view.held, modifiers)
            .into_iter()
            .map(|(code, pressed)| key(code, pressed)),
    );
    view.held = modifiers;
    let mut paste = None;
    for event in events {
        // egui turns Cmd/Ctrl+C, X and V into these with no key press (the
        // release still comes as a key): press the key for the app.
        let (code, pressed) = match event {
            Event::Copy => (Some(keys::KEY_C), true),
            Event::Cut => (Some(keys::KEY_X), true),
            Event::Paste(text) => {
                paste = Some(text.clone());
                (Some(keys::KEY_V), true)
            }
            Event::Key {
                key: logical,
                physical_key,
                pressed,
                repeat: false,
                ..
            } => (keys::evdev(physical_key.unwrap_or(*logical)), *pressed),
            _ => continue,
        };
        let Some(code) = code else {
            continue;
        };
        if pressed {
            if !view.keys_down.contains(&code) {
                view.keys_down.push(code);
            }
        } else {
            view.keys_down.retain(|c| *c != code);
        }
        out.push(key(code, pressed));
    }
    paste
}
