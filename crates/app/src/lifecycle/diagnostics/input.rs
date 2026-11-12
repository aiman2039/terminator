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
        if let Some(press) = self.press.take() {
            self.press_at(input, press);
            return;
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
        // Lend the script out so firing can update the runner state.
        let actions = std::mem::take(&mut self.actions);
        if let Some(action) = actions.get(self.action_index) {
            self.fire_action(ctx, input, action);
        }
        self.actions = actions;
    }

    fn fire_action(
        &mut self,
        ctx: &egui::Context,
        input: &mut egui::RawInput,
        action: &FixtureAction,
    ) {
        let rect = if action.check {
            None
        } else {
            target_rect(ctx, &action.target, ctx.cumulative_frame_nr())
        };
        if !self.action_may_fire(ctx, action, rect) {
            return;
        }
        if let Some(rect) = rect {
            self.send_action_input(action, rect, input);
        }
        if action.capture {
            self.request_fixture_capture(ctx);
            // A final check action ends the case once its state is reached.
            self.requested |=
                action.check && std::env::var_os("TERMINATOR_TEST_KEEP_OPEN").is_none();
        }
        self.last_action = Some((action.at_ms, self.started.elapsed()));
        crate::console::Console::err(format_args!("Fixture action: {}", action.target));
        self.action_index = self.action_index.saturating_add(1);
    }

    /// Whether `action` fires now, given its target `rect` (`None` while it
    /// is not shown, and always for a check action). Waiting actions fire on
    /// state, timed actions on their `at_ms` schedule.
    fn action_may_fire(
        &mut self,
        ctx: &egui::Context,
        action: &FixtureAction,
        rect: Option<egui::Rect>,
    ) -> bool {
        let frame = ctx.cumulative_frame_nr();
        let elapsed = self.started.elapsed();
        let Some(wait) = &action.wait else {
            return (action.check || rect.is_some()) && self.action_ready(action, elapsed);
        };
        let frames = self.pending.observe(self.action_index, rect, elapsed);
        let stable = (!action.check).then_some(frames);
        let since_previous = self
            .last_action
            .map_or(elapsed, |(_, at)| elapsed.saturating_sub(at));
        let unmet = if !action.check && elapsed < super::wait::STARTUP_SETTLE {
            Some("startup to settle".to_owned())
        } else if stable == Some(0) {
            Some("target to show".to_owned())
        } else {
            let shown = |target: &str| target_rect(ctx, target, frame).is_some();
            wait.unmet(shown, since_previous, stable)
        };
        if let Some(reason) = unmet {
            if stable == Some(0)
                && self.action_index > 0
                && let Some(after) = wait.retry_after()
                && self.pending.take_retry(after, elapsed)
            {
                // The previous step's input was lost (a popup did not open).
                self.action_index = self.action_index.saturating_sub(1);
                crate::console::Console::err(format_args!(
                    "Fixture retrying the step before {}",
                    action.target
                ));
                return false;
            }
            self.pending.report_slow(&action.target, &reason, elapsed);
            return false;
        }
        true
    }

    fn send_action_input(
        &mut self,
        action: &FixtureAction,
        rect: egui::Rect,
        input: &mut egui::RawInput,
    ) {
        let pos = action_position(action, rect);
        let moved = self.pointer != Some(pos);
        self.pointer = Some(pos);
        input.focused = true;
        input.events.push(egui::Event::PointerMoved(pos));
        match action_input(action) {
            ActionInput::Events(events) => input.events.extend(events),
            // A pointer that moves presses one frame later, as a hand does:
            // controls that show on hover (pane caption buttons) only take
            // a click after they were painted.
            ActionInput::Press(button) if moved => {
                self.press = Some((pos, button, action.hold));
            }
            ActionInput::Press(button) => self.press_at(input, (pos, button, action.hold)),
            // Ends a held drag (see `hold`): the press stays down
            // across the hover moves between the two actions.
            ActionInput::ReleaseHeld => {
                if let Some(held) = self.held.take() {
                    input.events.push(pointer_button(pos, held, false));
                }
            }
            ActionInput::None => {}
        }
    }

    /// Press `button` at `pos`; it is released next frame unless `hold`.
    fn press_at(
        &mut self,
        input: &mut egui::RawInput,
        (pos, button, hold): (egui::Pos2, egui::PointerButton, bool),
    ) {
        input.events.push(pointer_button(pos, button, true));
        if hold {
            self.held = Some(button);
        } else {
            self.release = Some((pos, button));
        }
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
pub(super) fn target_rect(
    ctx: &egui::Context,
    target: &str,
    current_frame: u64,
) -> Option<egui::Rect> {
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
        ActionInput::Events(key_events(action, key))
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

/// A real keyboard sends the key press and, for a printable key, its
/// typed text too.
fn key_events(action: &FixtureAction, name: &str) -> Vec<egui::Event> {
    let Some(key) = egui::Key::from_name(name) else {
        return Vec::new();
    };
    let modifiers = egui::Modifiers {
        ctrl: action.ctrl,
        command: action.command || (action.ctrl && !cfg!(target_os = "macos")),
        mac_cmd: action.command && cfg!(target_os = "macos"),
        shift: action.shift,
        ..Default::default()
    };
    terminator_native_edit::view::keyboard_events(key, modifiers)
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
