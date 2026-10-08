use super::super::doc::Buffer;
use super::mode::{Bit, Cursor, Effect, Key, ModalEngine, Mode};
use super::state::VimEngine;
impl ModalEngine for VimEngine {
    fn mode(&self) -> Mode {
        self.mode
    }

    fn cursor(&self) -> Cursor {
        self.cursor
    }

    fn selection(&self) -> Option<(Cursor, Cursor)> {
        match self.mode {
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock => {
                let a = self.anchor;
                let b = self.cursor;
                Some(if (a.line, a.col) <= (b.line, b.col) {
                    (a, b)
                } else {
                    (b, a)
                })
            }
            _ => None,
        }
    }

    fn command_line(&self) -> &str {
        &self.command
    }

    fn press_key<B: Buffer>(&mut self, doc: &mut B, key: Key) -> Vec<Effect> {
        match self.mode {
            Mode::Normal => self.normal_key(doc, key),
            Mode::Insert => self.insert_key(doc, key),
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock => self.visual_key(doc, key),
            Mode::Command => self.command_key(doc, key),
        }
    }

    fn type_text<B: Buffer>(&mut self, doc: &mut B, text: &str) -> Vec<Effect> {
        if self.mode != Mode::Insert || text.is_empty() {
            return Vec::new();
        }
        let at = self.char_idx(doc, self.cursor);
        if !self.replaying {
            self.recording.push(Bit::Type(text.to_string()));
            if at == self.typed_end {
                self.typed_end = self.typed_end.saturating_add(text.chars().count());
            } else {
                // Typing mid-text after a move: the joined recording no
                // longer replays in order.
                self.typed_exact = false;
            }
        }
        doc.insert(at, text);
        let (line, col) = doc.line_col_at(
            at.checked_add(text.chars().count().min(doc.len_chars().saturating_sub(at)))
                .unwrap_or(at),
        );
        self.cursor = Cursor { line, col };
        self.preferred_col = col;
        self.clamp(doc);
        Vec::new()
    }
}
