//! Turns Terminator input messages into wprs events for wprsd.

use std::num::NonZeroU32;

use rkyv::rancor::Error as RancorError;
use wprs::serialization::Event;
use wprs::serialization::geometry::{Point, Size};
use wprs::serialization::wayland::{
    AxisScroll, AxisSource, KeyInner, KeyState, KeyboardEvent, Mode, ModifierState, OutputEvent,
    OutputInfo, PointerEvent, PointerEventKind, Subpixel, Transform, WlSurfaceId,
};
use wprs::serialization::xdg_shell::{
    DecorationMode, ToplevelClose, ToplevelConfigure, ToplevelEvent,
};

use crate::protocol::{ClientMessage, PointerButton, WindowId};

/// wayland-csd-frame `WindowState::ACTIVATED`: the window has keyboard focus.
/// Use only defined bits: wprsd panics on unknown ones.
const ACTIVATED: u16 = 0b1000;
/// `WindowState::TILED`: a tab fills its area, so the app draws no shadows.
const TILED: u16 = 0b1111_0000;

/// Input routing state: which window has the pointer and the keyboard, and
/// the next serial number.
#[derive(Default)]
pub struct Input {
    pointer: Option<WlSurfaceId>,
    keyboard: Option<WlSurfaceId>,
    serial: u32,
}

impl Input {
    /// The wprs events for `message`. `surface` maps a window to its wprs
    /// surface; messages for unknown windows give no events.
    pub fn events(
        &mut self,
        message: &ClientMessage,
        surface: impl Fn(WindowId) -> Option<WlSurfaceId>,
    ) -> Vec<Event> {
        match message {
            ClientMessage::Configure {
                id,
                width,
                height,
                focused,
                ..
            } => surface(*id)
                .map(|s| self.configure(s, (*width, *height), *focused))
                .unwrap_or_default(),
            ClientMessage::CloseRequested { id } => surface(*id)
                .map(|surface_id| {
                    vec![Event::Toplevel(ToplevelEvent::Close(ToplevelClose {
                        surface_id,
                    }))]
                })
                .unwrap_or_default(),
            ClientMessage::PointerMotion { id, x, y } => surface(*id)
                .map(|s| self.motion(s, Point { x: *x, y: *y }))
                .unwrap_or_default(),
            ClientMessage::PointerLeave { .. } => self.leave(),
            ClientMessage::PointerButton {
                id,
                button,
                pressed,
            } => surface(*id)
                .map(|s| vec![self.button(s, *button, *pressed)])
                .unwrap_or_default(),
            ClientMessage::Scroll {
                id,
                dx,
                dy,
                discrete_x,
                discrete_y,
            } => surface(*id)
                .map(|s| vec![scroll(s, (*dx, *dy), (*discrete_x, *discrete_y))])
                .unwrap_or_default(),
            ClientMessage::Key {
                keycode, pressed, ..
            } => vec![self.key(*keycode, *pressed)],
            ClientMessage::Modifiers {
                shift,
                ctrl,
                alt,
                logo,
                caps_lock,
                num_lock,
            } => vec![Event::KeyboardEvent(KeyboardEvent::Modifiers {
                modifier_state: ModifierState {
                    ctrl: *ctrl,
                    alt: *alt,
                    shift: *shift,
                    caps_lock: *caps_lock,
                    logo: *logo,
                    num_lock: *num_lock,
                },
                layout_index: 0,
            })],
            ClientMessage::Hello { .. }
            | ClientMessage::ClipboardOffer { .. }
            | ClientMessage::ClipboardRead { .. }
            | ClientMessage::ClipboardData { .. } => Vec::new(),
        }
    }

    fn next_serial(&mut self) -> u32 {
        self.serial = self.serial.wrapping_add(1);
        self.serial
    }

    fn configure(
        &mut self,
        surface_id: WlSurfaceId,
        size: (u32, u32),
        focused: bool,
    ) -> Vec<Event> {
        let mut events = Vec::new();
        if focused && self.keyboard != Some(surface_id) {
            self.keyboard = Some(surface_id);
            events.push(Event::KeyboardEvent(KeyboardEvent::Enter {
                serial: self.next_serial(),
                surface_id,
                keycodes: Vec::new(),
                keysyms: Vec::new(),
            }));
        } else if !focused && self.keyboard == Some(surface_id) {
            self.keyboard = None;
            events.push(Event::KeyboardEvent(KeyboardEvent::Leave {
                serial: self.next_serial(),
            }));
        }
        let state = if focused { TILED | ACTIVATED } else { TILED };
        if let Some(configure) = toplevel_configure(surface_id, size, state) {
            events.push(Event::Toplevel(ToplevelEvent::Configure(configure)));
        }
        events
    }

    fn motion(&mut self, surface_id: WlSurfaceId, position: Point<f64>) -> Vec<Event> {
        let mut frame = Vec::new();
        if self.pointer != Some(surface_id) {
            if self.pointer.is_some() {
                frame.extend(self.leave_event());
            }
            self.pointer = Some(surface_id);
            let serial = self.next_serial();
            frame.push(PointerEvent {
                surface_id,
                position,
                kind: PointerEventKind::Enter { serial },
            });
        }
        frame.push(PointerEvent {
            surface_id,
            position,
            kind: PointerEventKind::Motion,
        });
        vec![Event::PointerFrame(frame)]
    }

    fn leave(&mut self) -> Vec<Event> {
        self.leave_event()
            .map(|event| vec![Event::PointerFrame(vec![event])])
            .unwrap_or_default()
    }

    fn leave_event(&mut self) -> Option<PointerEvent> {
        let surface_id = self.pointer.take()?;
        Some(PointerEvent {
            surface_id,
            position: Point { x: 0.0, y: 0.0 },
            kind: PointerEventKind::Leave {
                serial: self.next_serial(),
            },
        })
    }

    fn button(&mut self, surface_id: WlSurfaceId, button: PointerButton, pressed: bool) -> Event {
        let serial = self.next_serial();
        let button = evdev_button(button);
        let kind = if pressed {
            PointerEventKind::Press { button, serial }
        } else {
            PointerEventKind::Release { button, serial }
        };
        Event::PointerFrame(vec![PointerEvent {
            surface_id,
            position: Point { x: 0.0, y: 0.0 },
            kind,
        }])
    }

    fn key(&mut self, keycode: u32, pressed: bool) -> Event {
        Event::KeyboardEvent(KeyboardEvent::Key(KeyInner {
            serial: self.next_serial(),
            raw_code: keycode,
            state: if pressed {
                KeyState::Pressed
            } else {
                KeyState::Released
            },
        }))
    }
}

fn scroll(surface_id: WlSurfaceId, delta: (f64, f64), clicks: (i32, i32)) -> Event {
    let source = if clicks == (0, 0) {
        AxisSource::Finger
    } else {
        AxisSource::Wheel
    };
    let axis = |absolute: f64, clicks: i32| AxisScroll {
        absolute,
        // wl_pointer.axis_value120: one wheel click is 120.
        discrete: clicks.saturating_mul(120),
        stop: false,
    };
    Event::PointerFrame(vec![PointerEvent {
        surface_id,
        position: Point { x: 0.0, y: 0.0 },
        kind: PointerEventKind::Axis {
            horizontal: axis(delta.0, clicks.0),
            vertical: axis(delta.1, clicks.1),
            source: Some(source),
        },
    }])
}

/// linux/input-event-codes.h `BTN_*`.
fn evdev_button(button: PointerButton) -> u32 {
    match button {
        PointerButton::Left => 0x110,
        PointerButton::Right => 0x111,
        PointerButton::Middle => 0x112,
        PointerButton::Back => 0x113,
        PointerButton::Forward => 0x114,
    }
}

/// The one output wprsd needs before apps map windows. Its size is a
/// suggestion only; each window gets its own size through configure.
pub fn output() -> Event {
    Event::Output(OutputEvent::New(OutputInfo {
        id: 1,
        model: "Terminator".to_owned(),
        make: "Terminator".to_owned(),
        location: Point { x: 0, y: 0 },
        physical_size: Size { w: 0, h: 0 },
        subpixel: Subpixel::Unknown,
        transform: Transform::Normal,
        scale_factor: 1,
        mode: Mode {
            dimensions: Size { w: 3840, h: 2160 },
            refresh_rate: 60_000,
            current: true,
            preferred: true,
        },
        name: Some("terminator".to_owned()),
        description: None,
    }))
}

/// Same field order and types as wprs `ToplevelConfigure`, whose window
/// state field is private. rkyv gives both the same archived layout, so this
/// is serialized and read back as the wprs type, with bytecheck validation.
#[derive(rkyv::Archive, rkyv::Serialize)]
struct ConfigureMirror {
    surface_id: WlSurfaceId,
    new_size: Size<Option<NonZeroU32>>,
    suggested_bounds: Option<Size<u32>>,
    decoration_mode: DecorationMode,
    state: StateBits,
}

#[derive(rkyv::Archive, rkyv::Serialize)]
struct StateBits(u16);

pub fn toplevel_configure(
    surface_id: WlSurfaceId,
    size: (u32, u32),
    state: u16,
) -> Option<ToplevelConfigure> {
    let mirror = ConfigureMirror {
        surface_id,
        new_size: Size {
            w: NonZeroU32::new(size.0),
            h: NonZeroU32::new(size.1),
        },
        suggested_bounds: None,
        decoration_mode: DecorationMode::Server,
        state: StateBits(state),
    };
    let bytes = rkyv::to_bytes::<RancorError>(&mirror).ok()?;
    rkyv::from_bytes::<ToplevelConfigure, RancorError>(&bytes).ok()
}
