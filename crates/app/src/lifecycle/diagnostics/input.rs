//! Scripted fixture input: one action per ready frame, injected before layout.
use super::{Diagnostics, FixtureAction};
use eframe::egui;

/// What an action does besides moving the pointer onto its target.
enum ActionInput {
    /// Wheel, copy, modifier, key or text events.
    Events(Vec<egui::Event>),
    /// Press `button`; it is released next frame unless the action holds it.
    Press(egui::PointerButton),
    /// Release a press held by an earlier `hold` action.
    ReleaseHeld,
    /// Hover only.
    None,
}

impl Diagnostics {
    pub fn input(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        if self.path.is_none() {
            return;
        }
        self.reload_dynamic_actions();
        isolate_fixture_input(input);
        if let Some(pos) = self.pointer {
            input.events.push(egui::Event::PointerMoved(pos));
        }
        if let Some((pos, button)) = self.release.take() {
            input.events.push(pointer_button(pos, button, false));
            return;
        }
        self.fire_next_action(ctx, input);
    }

    fn reload_dynamic_actions(&mut self) {
        if let Some(path) = std::env::var_os("TERMINATOR_TEST_ACTIONS_PATH")
            && let Ok(bytes) = std::fs::read(path)
            && let Ok(actions) = serde_json::from_slice::<Vec<FixtureAction>>(&bytes)
        {
            if self.actions.len() != actions.len() {
                crate::console::Console::err(format_args!(
                    "Fixture loaded {} dynamic actions",
                    actions.len()
                ));
            }
            self.actions = actions;
        }
    }

    fn fire_next_action(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        let current_frame = ctx.cumulative_frame_nr();
        let Some(action) = self.actions.get(self.action_index) else {
            return;
        };
        if !self.action_ready(action, self.started.elapsed()) {
            return;
        }
        let Some(rect) = target_rect(ctx, &action.target, current_frame) else {
            return;
        };
        let pos = action_position(action, rect);
        self.pointer = Some(pos);
        input.focused = true;
        input.events.push(egui::Event::PointerMoved(pos));
        match action_input(action) {
            ActionInput::Events(events) => input.events.extend(events),
            ActionInput::Press(button) => {
                input.events.push(pointer_button(pos, button, true));
                if action.hold {
                    self.held = Some(button);
                } else {
                    self.release = Some((pos, button));
                }
            }
            // Ends a held drag (see `hold`): the press stays down
            // across the hover moves between the two actions.
            ActionInput::ReleaseHeld => {
                if let Some(held) = self.held.take() {
                    input.events.push(pointer_button(pos, held, false));
                }
            }
            ActionInput::None => {}
        }
        if action.capture {
            self.request_fixture_capture(ctx);
        }
        self.last_action = Some((action.at_ms, self.started.elapsed()));
        crate::console::Console::err(format_args!("Fixture action: {}", action.target));
        self.action_index = self.action_index.saturating_add(1);
    }
}

/// A fixture owns its input. Do not let concurrent desktop typing or
/// clipboard shortcuts reach its shells/editors or leak into captures.
fn isolate_fixture_input(input: &mut egui::RawInput) {
    if std::env::var_os("TERMINATOR_TEST_NATIVE_INPUT").is_none() {
        input
            .events
            .retain(|event| matches!(event, egui::Event::WindowFocused(_)));
    }
    if std::env::var_os("TERMINATOR_TEST_BACKGROUND").is_some() {
        input.focused = true;
    }
}

/// The target's rect, if it was recorded on the previous frame.
fn target_rect(ctx: &egui::Context, target: &str, current_frame: u64) -> Option<egui::Rect> {
    ctx.data(|d| {
        d.get_temp::<egui::Rect>(egui::Id::unique(("fixture-target", target)))
            .filter(|_| {
                d.get_temp::<u64>(egui::Id::unique(("fixture-target-frame", target)))
                    .is_none_or(|frame| frame.checked_add(1) == Some(current_frame))
            })
    })
}

fn action_position(action: &FixtureAction, rect: egui::Rect) -> egui::Pos2 {
    if let Some([x, y]) = action.hover_offset {
        egui::pos2(rect.min.x + x, rect.min.y + y)
    } else if action.hover {
        egui::pos2(rect.min.x + 45.0, rect.min.y + 10.0)
    } else {
        rect.center()
    }
}

fn pointer_button(pos: egui::Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

/// The first matching kind wins: scroll, copy, modifier, key, input text,
/// replacement text, then a click unless the action only hovers.
fn action_input(action: &FixtureAction) -> ActionInput {
    if let Some(delta) = action.scroll {
        ActionInput::Events(vec![wheel_event(action, delta)])
    } else if action.copy {
        ActionInput::Events(vec![egui::Event::Copy])
    } else if let Some(modifier) = &action.modifier {
        ActionInput::Events(vec![egui::Event::ModifiersChanged(named_modifiers(
            modifier,
        ))])
    } else if let Some(key) = &action.key {
        ActionInput::Events(key_event(action, key).into_iter().collect())
    } else if let Some(text) = &action.input {
        ActionInput::Events(vec![egui::Event::Text(text.clone())])
    } else if let Some(text) = &action.text {
        ActionInput::Events(replace_text(text))
    } else if action.hover {
        ActionInput::None
    } else if action.release_button {
        ActionInput::ReleaseHeld
    } else if action.right_click {
        ActionInput::Press(egui::PointerButton::Secondary)
    } else {
        ActionInput::Press(egui::PointerButton::Primary)
    }
}

fn wheel_event(action: &FixtureAction, delta: f32) -> egui::Event {
    egui::Event::MouseWheel {
        unit: match action.wheel_unit.as_deref() {
            Some("line") => egui::MouseWheelUnit::Line,
            Some("page") => egui::MouseWheelUnit::Page,
            _ => egui::MouseWheelUnit::Point,
        },
        phase: match action.wheel_phase.as_deref() {
            Some("start") => egui::TouchPhase::Start,
            Some("end") => egui::TouchPhase::End,
            Some("cancel") => egui::TouchPhase::Cancel,
            _ => egui::TouchPhase::Move,
        },
        delta: egui::vec2(action.scroll_x.unwrap_or_default(), delta),
        source: egui::MouseWheelSource::Unknown,
        modifiers: egui::Modifiers {
            shift: action.shift,
            ..Default::default()
        },
    }
}

fn named_modifiers(modifier: &str) -> egui::Modifiers {
    match modifier {
        "ctrl" => egui::Modifiers {
            ctrl: true,
            command: !cfg!(target_os = "macos"),
            ..Default::default()
        },
        "cmd" => egui::Modifiers {
            command: true,
            mac_cmd: cfg!(target_os = "macos"),
            ..Default::default()
        },
        _ => egui::Modifiers::default(),
    }
}

fn key_event(action: &FixtureAction, key: &str) -> Option<egui::Event> {
    let key = egui::Key::from_name(key)?;
    let modifiers = egui::Modifiers {
        ctrl: action.ctrl,
        command: action.command || (action.ctrl && !cfg!(target_os = "macos")),
        mac_cmd: action.command && cfg!(target_os = "macos"),
        shift: action.shift,
        ..Default::default()
    };
    Some(egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    })
}

/// Select all, then type `text` over the selection.
fn replace_text(text: &str) -> Vec<egui::Event> {
    let modifiers = egui::Modifiers {
        command: true,
        mac_cmd: cfg!(target_os = "macos"),
        ctrl: !cfg!(target_os = "macos"),
        ..Default::default()
    };
    vec![
        egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        },
        egui::Event::Text(text.to_owned()),
    ]
}
