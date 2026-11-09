//! Where the windows of a remote tab are drawn, and egui pointer events as
//! bridge input.

use eframe::egui::{self, Event, MouseWheelUnit, PointerButton, Pos2, Rect, Vec2};
use terminator_core::remote_apps::{
    ClientMessage, PointerButton as RemoteButton, RemoteAppsState, RemoteWindow, WindowId,
    WindowRole,
};

/// Pixels per wheel line for smooth scrolling.
const LINE_PIXELS: f64 = 15.0;

/// Where one window is drawn in the tab.
pub(super) struct Placed {
    pub(super) id: u64,
    pub(super) rect: Rect,
}

/// Every window drawn in a tab, bottom to top, and the points-to-pixels
/// factor.
pub(super) struct Geometry {
    pub(super) placed: Vec<Placed>,
    pub(super) ppp: f32,
}

impl Geometry {
    /// The toplevel at `origin` and its popups (and their popups) at their
    /// offsets, bottom to top.
    #[expect(
        clippy::arithmetic_side_effects,
        clippy::cast_precision_loss,
        reason = "f32 screen geometry; offsets are window pixels."
    )]
    pub(super) fn new(state: &RemoteAppsState, window: u64, (origin, ppp): (Pos2, f32)) -> Self {
        let mut placed: Vec<(u64, Pos2)> = vec![(window, origin)];
        // Popups come after their parents in the list (the daemon sorts
        // them), so one pass finds every level.
        for w in &state.windows {
            if let WindowRole::Popup { parent, x, y } = w.role
                && let Some(&(_, at)) = placed.iter().find(|(id, _)| *id == parent.0)
            {
                placed.push((w.id.0, at + Vec2::new(x as f32, y as f32) / ppp));
            }
        }
        let placed = placed
            .into_iter()
            .filter_map(|(id, at)| {
                let w: &RemoteWindow = state.windows.iter().find(|w| w.id.0 == id)?;
                let size = Vec2::new(f32::from(w.width), f32::from(w.height)) / ppp;
                Some(Placed {
                    id,
                    rect: Rect::from_min_size(at, size),
                })
            })
            .collect();
        Self { placed, ppp }
    }

    /// The topmost window under `pos`, and `pos` in its pixels.
    #[expect(clippy::arithmetic_side_effects, reason = "f32 screen geometry.")]
    fn target(&self, pos: Pos2) -> Option<(u64, f64, f64)> {
        let place = self
            .placed
            .iter()
            .rev()
            .find(|p| p.rect.contains(pos))
            .or(self.placed.first())?;
        let local = (pos - place.rect.min) * self.ppp;
        Some((place.id, f64::from(local.x), f64::from(local.y)))
    }
}

pub(super) fn pointer_messages(
    events: &[Event],
    geometry: &Geometry,
    response: &egui::Response,
    out: &mut Vec<ClientMessage>,
) {
    let area = response.rect;
    let pressed = response.is_pointer_button_down_on();
    for event in events {
        match event {
            Event::PointerMoved(pos) if area.contains(*pos) || pressed => {
                if let Some((id, x, y)) = geometry.target(*pos) {
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
                    (remote_button(*button), geometry.target(*pos))
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
                let Some((id, _, _)) = response.hover_pos().and_then(|pos| geometry.target(pos))
                else {
                    continue;
                };
                out.push(scroll(WindowId(id), *unit, *delta));
            }
            // Leaving is sent when the hover ends (see `remote_app_view`).
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
