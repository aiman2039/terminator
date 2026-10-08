use super::super::doc::Buffer;
use super::mode::{Bit, Cursor, Effect, Key, Mode, PendingOp, Place};
use super::state::VimEngine;
use super::words::{WordClass, current_char, line_len, word_class};
impl VimEngine {
    pub(super) fn normal_key<B: Buffer>(&mut self, doc: &mut B, key: Key) -> Vec<Effect> {
        // Vim parity: Space moves right like `l`, including under operators.
        let key = if key == Key::Char(' ') {
            Key::Char('l')
        } else {
            key
        };
        // Digits accumulate a count; a bare 0 is the line-start motion.
        if let Key::Char(c) = key
            && c.is_ascii_digit()
            && !(c == '0' && self.count.is_empty())
        {
            self.count.push(c);
            self.pending_g = false;
            self.rec_bit(Bit::Key(key));
            return Vec::new();
        }
        // `gd` is goto-definition; any other non-`g` key after `g`
        // cancels the prefix.
        if self.pending_g && key != Key::Char('g') && key != Key::Char('d') {
            self.clear_pending();
            self.rec_discard();
            return vec![Effect::Bell];
        }
        let count = self.take_count();
        match key {
            Key::Char('h') | Key::Left => {
                self.apply_motion(doc, 'h', count);
            }
            Key::Char('j') | Key::Down => {
                self.apply_motion(doc, 'j', count);
            }
            Key::Char('k') | Key::Up => {
                self.apply_motion(doc, 'k', count);
            }
            Key::Char('l') | Key::Right => {
                self.apply_motion(doc, 'l', count);
            }
            Key::Char('w' | 'W') => {
                self.apply_motion(doc, 'w', count);
            }
            Key::Char('b' | 'B') => {
                self.apply_motion(doc, 'b', count);
            }
            Key::Char('e' | 'E') => {
                self.apply_motion(doc, 'e', count);
            }
            Key::Char('0') => {
                self.apply_motion(doc, '0', 1);
            }
            Key::Char('$') => {
                self.apply_motion(doc, '$', 1);
            }
            Key::Char('^') => {
                let col = Self::first_non_blank(doc, self.cursor.line);
                self.apply_motion_to(
                    doc,
                    Cursor {
                        line: self.cursor.line,
                        col,
                    },
                    '^',
                );
            }
            Key::Char('G') => {
                let line = if self.count.is_empty() {
                    doc.line_count().saturating_sub(1)
                } else {
                    count.saturating_sub(1)
                };
                let col = Self::first_non_blank(doc, line.min(doc.line_count().saturating_sub(1)));
                self.apply_motion_to(doc, Cursor { line, col }, 'G');
                self.scroll_request = true;
            }
            Key::Char('g') => {
                if self.pending_g {
                    self.clear_pending();
                    let col = Self::first_non_blank(doc, 0);
                    self.apply_motion_to(doc, Cursor { line: 0, col }, 'g');
                    self.scroll_request = true;
                } else {
                    self.pending_g = true;
                    return Vec::new();
                }
            }
            Key::Char('x') => {
                self.rec_bit(Bit::Key(key));
                self.delete_chars_forward(doc, count);
                self.rec_commit();
            }
            Key::Char('d') => {
                if self.pending_g {
                    self.clear_pending();
                    self.rec_discard();
                    self.scroll_request = true;
                    return vec![Effect::GotoDefinition];
                }
                return self.operator_key(doc, PendingOp::Delete);
            }
            Key::Char('c') => {
                return self.operator_key(doc, PendingOp::Change);
            }
            Key::Char('y') => {
                return self.operator_key(doc, PendingOp::Yank);
            }
            Key::Char('p') => {
                self.rec_bit(Bit::Key(key));
                self.paste(doc);
                self.clear_pending();
                self.rec_commit();
            }
            Key::Char('.') => {
                if self.pending.is_some() {
                    self.clear_pending();
                    self.rec_discard();
                    return vec![Effect::Bell];
                }
                let Some(change) = self.last_change.clone() else {
                    return vec![Effect::Bell];
                };
                return self.exec_repeat(doc, change);
            }
            Key::Char('u') => {
                self.undo(doc);
            }
            Key::Char('i') => {
                self.rec_bit(Bit::Key(key));
                self.start_insert_place(doc, Place::AtCursor);
            }
            Key::Char('a') => {
                self.rec_bit(Bit::Key(key));
                self.start_insert_place(doc, Place::AfterCursor);
            }
            Key::Char('A') => {
                self.rec_bit(Bit::Key(key));
                self.start_insert_place(doc, Place::LineEnd);
            }
            Key::Char('I') => {
                self.rec_bit(Bit::Key(key));
                self.start_insert_place(doc, Place::FirstBlank);
            }
            Key::Char('o') => {
                self.rec_bit(Bit::Key(key));
                self.start_insert_place(doc, Place::LineBelow);
            }
            Key::Char('O') => {
                self.rec_bit(Bit::Key(key));
                self.start_insert_place(doc, Place::LineAbove);
            }
            Key::Char('v') => {
                self.rec_bit(Bit::Key(key));
                self.mode = Mode::Visual;
                self.anchor = self.cursor;
                self.clear_pending();
            }
            Key::Char('V') => {
                self.rec_bit(Bit::Key(key));
                self.mode = Mode::VisualLine;
                self.anchor = self.cursor;
                self.clear_pending();
            }
            Key::CtrlV => {
                self.rec_bit(Bit::Key(key));
                self.mode = Mode::VisualBlock;
                self.anchor = self.cursor;
                self.clear_pending();
            }
            Key::Char(':') => {
                self.mode = Mode::Command;
                self.command.clear();
                self.clear_pending();
                self.rec_discard();
            }
            Key::Char('/') => {
                self.mode = Mode::Command;
                self.command.clear();
                self.command.push('/');
                self.clear_pending();
                self.rec_discard();
            }
            Key::Char('n') => {
                if self.search_last.is_empty() {
                    self.rec_discard();
                    return vec![Effect::Bell];
                }
                let pattern = self.search_last.clone();
                let from = self.char_idx(doc, self.cursor).saturating_add(1);
                self.clear_pending();
                self.rec_discard();
                if !self.jump_to_match(doc, &pattern, from, true) {
                    return vec![Effect::Bell];
                }
            }
            Key::Char('N') => {
                if self.search_last.is_empty() {
                    self.rec_discard();
                    return vec![Effect::Bell];
                }
                let pattern = self.search_last.clone();
                let from = self.char_idx(doc, self.cursor);
                self.clear_pending();
                self.rec_discard();
                if !self.jump_to_match(doc, &pattern, from, false) {
                    return vec![Effect::Bell];
                }
            }
            // `K` looks up the cursor word, like vim with `keywordprg`
            // on an LSP: no buffer change, nothing to repeat or record.
            Key::Char('K') => {
                self.clear_pending();
                self.rec_discard();
                return vec![Effect::Hover];
            }
            // Vim parity: Backspace moves left like `h` in Normal mode.
            Key::Backspace => self.apply_motion(doc, 'h', 1),
            // The Delete key deletes like `x`, including under counts.
            Key::Delete => {
                self.rec_bit(Bit::Key(Key::Char('x')));
                self.delete_chars_forward(doc, count);
                self.rec_commit();
            }
            Key::Home => self.apply_motion(doc, '0', 1),
            Key::End => self.apply_motion(doc, '$', 1),
            Key::Escape => {
                self.clear_pending();
                self.rec_discard();
            }
            _ => {
                self.rec_discard();
                return vec![Effect::Bell];
            }
        }
        Vec::new()
    }

    pub(super) fn operator_key<B: Buffer>(&mut self, doc: &mut B, op: PendingOp) -> Vec<Effect> {
        let letter = match op {
            PendingOp::Delete => 'd',
            PendingOp::Change => 'c',
            PendingOp::Yank => 'y',
        };
        if self.pending == Some(op) {
            self.rec_bit(Bit::Op(letter));
            let count = self.take_count();
            self.clear_pending();
            match op {
                PendingOp::Delete => {
                    self.delete_lines(doc, count);
                    self.rec_commit();
                }
                PendingOp::Yank => {
                    self.yank_lines(doc, count);
                    // Yank mutates nothing: the last change survives.
                    self.rec_discard();
                }
                PendingOp::Change => {
                    self.begin_change_group(doc);
                    self.change_line(doc, count);
                }
            }
            return Vec::new();
        }
        self.rec_bit(Bit::Op(letter));
        self.pending = Some(op);
        self.pending_g = false;
        Vec::new()
    }

    pub(super) fn apply_motion<B: Buffer>(&mut self, doc: &mut B, kind: char, count: usize) {
        // Vim special case: `cw` on a word behaves like `ce`.
        let kind = if kind == 'w'
            && self.pending == Some(PendingOp::Change)
            && word_class(current_char(doc, self.cursor)) == WordClass::Word
        {
            'e'
        } else {
            kind
        };
        if let Some(op) = self.pending.take() {
            self.pending_g = false;
            self.count.clear();
            self.rec_bit(Bit::Motion(kind));
            match kind {
                'h' | 'l' | '0' | '$' | 'w' | 'b' | 'e' => {
                    let (target, inclusive) = self.motion_target(doc, kind, count);
                    self.apply_operator(doc, op, target, inclusive);
                }
                'j' | 'k' => {
                    // Vertical motions under an operator stay charwise in v1;
                    // dd/yy/cc cover the linewise case.
                    let saved = self.cursor;
                    if kind == 'j' {
                        self.move_j(doc, count);
                    } else {
                        self.move_k(doc, count);
                    }
                    let target = self.cursor;
                    self.cursor = saved;
                    let inclusive = target.line != saved.line;
                    self.apply_operator(doc, op, target, inclusive);
                }
                _ => {
                    self.rec_discard();
                }
            }
            return;
        }
        self.pending_g = false;
        self.count.clear();
        self.rec_discard();
        match kind {
            'h' => self.move_h(doc, count),
            'j' => self.move_j(doc, count),
            'k' => self.move_k(doc, count),
            'l' => self.move_l(doc, count),
            'w' | 'b' | 'e' => {
                let (target, _) = self.motion_target(doc, kind, count);
                self.goto(doc, target.line, target.col, true);
            }
            '0' => self.goto(doc, self.cursor.line, 0, true),
            '$' => {
                let len = line_len(doc, self.cursor.line);
                self.goto(doc, self.cursor.line, len.saturating_sub(1), true);
            }
            _ => {}
        }
        doc.end_run();
    }

    pub(super) fn apply_motion_to<B: Buffer>(&mut self, doc: &mut B, target: Cursor, motion: char) {
        if let Some(op) = self.pending.take() {
            self.pending_g = false;
            self.count.clear();
            self.rec_bit(Bit::Motion(motion));
            self.apply_operator(doc, op, target, false);
            return;
        }
        self.pending_g = false;
        self.count.clear();
        self.rec_discard();
        self.goto(doc, target.line, target.col, true);
        doc.end_run();
    }

    pub(super) fn motion_target<B: Buffer>(
        &self,
        doc: &B,
        kind: char,
        count: usize,
    ) -> (Cursor, bool) {
        match kind {
            'h' => (
                Cursor {
                    line: self.cursor.line,
                    col: self.cursor.col.saturating_sub(count),
                },
                false,
            ),
            'l' => {
                let len = line_len(doc, self.cursor.line);
                (
                    Cursor {
                        line: self.cursor.line,
                        col: self
                            .cursor
                            .col
                            .checked_add(count)
                            .map_or(len.saturating_sub(1), |col| col.min(len.saturating_sub(1))),
                    },
                    false,
                )
            }
            'w' | 'b' | 'e' => self.word_target(doc, kind, count),
            '0' => (
                Cursor {
                    line: self.cursor.line,
                    col: 0,
                },
                false,
            ),
            '$' => {
                let len = line_len(doc, self.cursor.line);
                (
                    Cursor {
                        line: self.cursor.line,
                        col: len.saturating_sub(1),
                    },
                    true,
                )
            }
            _ => (self.cursor, false),
        }
    }
}
