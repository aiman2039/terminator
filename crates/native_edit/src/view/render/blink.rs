//! Caret blink: 530 ms on, 530 ms off. Any input or caret move shows the
//! caret again, and the view repaints only at the next toggle.
use super::SourceView;
use crate::doc::Buffer;
use crate::vim::{Cursor, ModalEngine};

/// Half of one blink cycle, in seconds.
pub(super) const BLINK_HALF: f64 = 0.53;

/// When the blink cycle last restarted, and at which caret.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct BlinkState {
    pub since: f64,
    pub cursor: Cursor,
}

impl BlinkState {
    /// Restart the cycle at `now` on input or a caret move; otherwise
    /// keep the running one.
    pub fn next(previous: Option<Self>, now: f64, cursor: Cursor, input: bool) -> Self {
        match previous {
            Some(state) if !input && state.cursor == cursor => state,
            _ => Self { since: now, cursor },
        }
    }
}

/// Whether the caret shows `elapsed` seconds into the cycle, and the
/// seconds until it next toggles.
pub(super) fn blink_phase(elapsed: f64) -> (bool, f64) {
    let elapsed = elapsed.max(0.0);
    let phase = (elapsed / BLINK_HALF).floor();
    let visible = phase % 2.0 < 1.0;
    let until = ((phase + 1.0) * BLINK_HALF - elapsed).max(0.001);
    (visible, until)
}

impl<B: Buffer> SourceView<'_, B> {
    /// Caret visibility this frame. Blink runs only while focused and
    /// enabled; otherwise the caret stays solid.
    pub(super) fn caret_on(&self, ui: &egui::Ui, focused: bool) -> bool {
        if !self.options.caret_blink || !focused {
            return true;
        }
        let key = self.geometry.focus_id.with("caret-blink");
        let (now, input) = ui.input(|input| {
            let typed = input.events.iter().any(|event| {
                matches!(
                    event,
                    egui::Event::Key { pressed: true, .. }
                        | egui::Event::Text(_)
                        | egui::Event::Paste(_)
                        | egui::Event::PointerButton { pressed: true, .. }
                )
            });
            (input.time, typed)
        });
        let cursor = self.engine.cursor();
        let state = ui.data_mut(|data| {
            let state = BlinkState::next(data.get_temp(key), now, cursor, input);
            data.insert_temp(key, state);
            state
        });
        let (visible, until) = blink_phase(now - state.since);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(until));
        visible
    }
}
