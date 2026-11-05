use super::super::doc::Buffer;
use super::cursor::GotoInput;
use super::mode::{Bit, Cursor, Effect, Key};
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
                // The document's indent style, like `expandtab`.
                let text = self.tab_text_at_caret(doc);
                let _ = self.insert_text(doc, &text);
            }
            Key::Ctrl('w') => self.insert_delete_back(doc, false),
            Key::Ctrl('u') => self.insert_delete_back(doc, true),
            Key::Ctrl('o') => self.insert_ctrl_o(doc),
            // Unreachable through the view (Ctrl-V pastes outside vim
            // Normal/Visual); bell rather than guess.
            Key::CtrlV | Key::CtrlCaret | Key::BackTab | Key::Ctrl(_) => return vec![Effect::Bell],
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
        if self.replace_backspace(doc) {
            self.typed_exact = false;
            self.clamp(doc);
            return;
        }
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

    /// Insert Ctrl-W (word before the cursor) and Ctrl-U (back to the
    /// indent, or the line start); at column 0 both join the lines.
    fn insert_delete_back<B: Buffer>(&mut self, doc: &mut B, line: bool) {
        let at = self.char_idx(doc, self.cursor);
        let text: Vec<char> = doc.line_text(self.cursor.line).chars().collect();
        let col = self.cursor.col.min(text.len());
        let start = if col == 0 {
            at.saturating_sub(1)
        } else if line {
            let indent = Self::first_non_blank(doc, self.cursor.line);
            at.saturating_sub(if col > indent {
                col.saturating_sub(indent)
            } else {
                col
            })
        } else {
            let mut c = col;
            while c > 0
                && text
                    .get(c.saturating_sub(1))
                    .is_some_and(|ch| ch.is_whitespace())
            {
                c = c.saturating_sub(1);
            }
            let class = text
                .get(c.saturating_sub(1))
                .map(|ch| super::words::word_class(*ch));
            while c > 0
                && text
                    .get(c.saturating_sub(1))
                    .map(|ch| super::words::word_class(*ch))
                    == class
            {
                c = c.saturating_sub(1);
            }
            at.saturating_sub(col.saturating_sub(c))
        };
        if start < at {
            self.typed_exact = false;
            self.x.keys_change = true;
            doc.delete(start..at);
            self.cursor = Self::cursor_at(doc, start);
            self.preferred_col = self.cursor.col;
        }
        self.clamp(doc);
    }

    /// Insert Ctrl-O: run one Normal command, then come back to Insert.
    fn insert_ctrl_o<B: Buffer>(&mut self, doc: &mut B) {
        let insert_at = self.cursor;
        self.x.oneshot = Some((insert_at, insert_at, false));
        self.enter_normal(doc);
        self.x.oneshot = Some((insert_at, self.cursor, false));
    }
}
