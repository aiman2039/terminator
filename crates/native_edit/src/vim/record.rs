use super::super::doc::Buffer;
use super::mode::{Bit, Cursor, Effect, Mode};
use super::state::VimEngine;
impl VimEngine {
    /// Record one input bit for `.` repeat. Skipped while replaying, so a
    /// repeat never overwrites the change it replays.
    pub(super) fn rec_bit(&mut self, bit: Bit) {
        if !self.replaying {
            self.recording.push(bit);
        }
    }

    /// Drop the change in progress, keeping the last completed change:
    /// navigation, yank, undo, ex commands, and search are not repeatable.
    pub(super) fn rec_discard(&mut self) {
        if !self.replaying {
            self.recording.clear();
        }
    }

    /// Complete a Normal-mode change with no insert session (`x`, `dd`,
    /// `dw`, `p`): parse the recorded bits into the last change.
    pub(super) fn rec_commit(&mut self) {
        if !self.replaying {
            self.last_change = Self::parse_repeat(&self.recording, None);
            self.recording.clear();
        }
    }

    /// Net typed text of the current recording: concatenated `Type` bits.
    /// Backspace pops the last typed char when it can only erase session
    /// text; anything else (arrows, forward delete, clicks, IDE paste)
    /// ends attribution, so replay and block insert stay exact.
    pub(super) fn typed_text(recording: &[Bit]) -> String {
        let mut out = String::new();
        for bit in recording {
            if let Bit::Type(text) = bit {
                out.push_str(text);
            }
        }
        out
    }

    /// Pointer click: move the cursor (or the active end of a selection).
    /// Never edits, so it returns no effects. Outside an insert session a
    /// click is navigation and drops the change in progress; inside one
    /// the typed text stays exactly attributable, so recording continues.
    pub fn place_cursor<B: Buffer>(&mut self, doc: &mut B, line: usize, col: usize) -> Vec<Effect> {
        if self.mode == Mode::Command {
            return Vec::new();
        }
        self.clear_pending();
        doc.end_run();
        self.cursor = Cursor { line, col };
        if !matches!(
            self.mode,
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock
        ) {
            self.anchor = self.cursor;
        }
        if self.mode != Mode::Insert {
            self.rec_discard();
        }
        self.preferred_col = col;
        self.clamp(doc);
        Vec::new()
    }
}
