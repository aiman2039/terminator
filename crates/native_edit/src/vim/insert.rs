use super::super::doc::Buffer;
use super::cursor::GotoInput;
use super::mode::{Bit, Cursor, Effect, Key, ModalEngine};
use super::state::VimEngine;
use super::words::line_len;

impl VimEngine {
    pub(super) fn insert_key<B: Buffer>(&mut self, doc: &mut B, key: Key) -> Vec<Effect> {
        if self.modeless
            && let Some(effects) = self.modeless_insert_key(doc, key)
        {
            return effects;
        }
        match key {
            Key::Escape => {
                self.enter_normal(doc);
            }
            Key::Enter => self.insert_newline(doc),
            Key::Backspace => self.insert_backspace(doc),
            Key::Delete => self.insert_delete(doc),
            Key::Left => self.move_h(doc, 1),
            Key::Right => self.move_l(doc, 1),
            Key::Up => self.move_k(doc, 1),
            Key::Down => self.move_j(doc, 1),
            Key::Home => self.goto(
                doc,
                GotoInput {
                    line: self.cursor.line,
                    col: 0,
                    preferred: true,
                },
            ),
            Key::End => {
                let len = line_len(doc, self.cursor.line);
                self.goto(
                    doc,
                    GotoInput {
                        line: self.cursor.line,
                        col: len,
                        preferred: true,
                    },
                );
            }
            Key::Tab => {
                let _ = self.type_text(doc, "\t");
            }
            // Unreachable through the view (Ctrl-V pastes outside vim
            // Normal/Visual); bell rather than guess.
            Key::CtrlV | Key::BackTab => return vec![Effect::Bell],
            Key::Char(_) => return vec![Effect::Bell],
        }
        Vec::new()
    }

    /// Modeless editing keys (IDE Enter/Tab/Backspace and caret moves);
    /// `None` leaves the key to the vim insert handling.
    fn modeless_insert_key<B: Buffer>(&mut self, doc: &mut B, key: Key) -> Option<Vec<Effect>> {
        self.typed_exact = false;
        match key {
            Key::Enter => {
                self.ide_enter(doc);
                Some(Vec::new())
            }
            Key::Tab => {
                self.ide_tab(doc);
                Some(Vec::new())
            }
            Key::Backspace if self.ide_backspace(doc) => Some(Vec::new()),
            Key::Left | Key::Right | Key::Up | Key::Down | Key::Home | Key::End => {
                Some(self.modeless_key(doc, key))
            }
            _ => None,
        }
    }

    fn insert_newline<B: Buffer>(&mut self, doc: &mut B) {
        let at = self.char_idx(doc, self.cursor);
        if !self.replaying {
            self.recording.push(Bit::Type("\n".to_string()));
            if at == self.typed_end {
                self.typed_end = self.typed_end.saturating_add(1);
            } else {
                self.typed_exact = false;
            }
        }
        doc.insert(at, "\n");
        let (line, col) = doc.line_col_at(at.checked_add(1).unwrap_or(at));
        self.cursor = Cursor { line, col };
        self.preferred_col = col;
        self.clamp(doc);
    }

    fn insert_backspace<B: Buffer>(&mut self, doc: &mut B) {
        let at = self.char_idx(doc, self.cursor);
        if !self.replaying {
            self.unrecord_backspace(at);
        }
        if at > 0 {
            let prev = at.saturating_sub(1);
            doc.delete(prev..at);
            let (line, col) = doc.line_col_at(prev);
            self.cursor = Cursor { line, col };
            self.preferred_col = col;
        }
        self.clamp(doc);
    }

    /// Only popping the session's own tail keeps the recording exact: the
    /// cursor must sit at the end of session text, past its start.
    fn unrecord_backspace(&mut self, at: usize) {
        if !(self.typed_exact && at == self.typed_end && at > self.typed_start) {
            self.typed_exact = false;
            return;
        }
        let mut popped = false;
        while let Some(Bit::Type(typed)) = self.recording.last_mut() {
            if typed.pop().is_some() {
                popped = true;
                break;
            }
            self.recording.pop();
        }
        if popped {
            self.typed_end = self.typed_end.saturating_sub(1);
        } else {
            self.typed_exact = false;
        }
    }

    fn insert_delete<B: Buffer>(&mut self, doc: &mut B) {
        // Forward delete can eat pre-existing text: unattributable.
        if !self.replaying {
            self.typed_exact = false;
        }
        let at = self.char_idx(doc, self.cursor);
        if at < doc.len_chars() {
            doc.delete(at..at.checked_add(1).unwrap_or(at));
        }
        self.clamp(doc);
    }
}
