use super::super::doc::Buffer;
use super::cursor::GotoInput;
use super::mode::{Bit, Cursor, Effect, Key, Mode, PendingOp, Place};
use super::operators::{ApplyOperatorInput, JumpToMatchInput};
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
        if let Some(effects) = self.normal_prefix(key) {
            return effects;
        }
        let count = self.take_count();
        self.normal_command(doc, key, count)
    }

    /// Count digits and the `g` prefix: `Some` when the key was consumed
    /// before reaching a command.
    fn normal_prefix(&mut self, key: Key) -> Option<Vec<Effect>> {
        // Digits accumulate a count; a bare 0 is the line-start motion.
        if let Key::Char(c) = key
            && c.is_ascii_digit()
            && !(c == '0' && self.count.is_empty())
        {
            self.count.push(c);
            self.pending_g = false;
            self.rec_bit(Bit::Key(key));
            return Some(Vec::new());
        }
        // `gd` is goto-definition; any other non-`g` key after `g`
        // cancels the prefix.
        if self.pending_g && key != Key::Char('g') && key != Key::Char('d') {
            self.clear_pending();
            self.rec_discard();
            return Some(vec![Effect::Bell]);
        }
        None
    }

    fn normal_command<B: Buffer>(&mut self, doc: &mut B, key: Key, count: usize) -> Vec<Effect> {
        match key {
            Key::Char('h') | Key::Left => self.apply_motion(doc, 'h', count),
            Key::Char('j') | Key::Down => self.apply_motion(doc, 'j', count),
            Key::Char('k') | Key::Up => self.apply_motion(doc, 'k', count),
            Key::Char('l') | Key::Right => self.apply_motion(doc, 'l', count),
            Key::Char('w' | 'W') => self.apply_motion(doc, 'w', count),
            Key::Char('b' | 'B') => self.apply_motion(doc, 'b', count),
            Key::Char('e' | 'E') => self.apply_motion(doc, 'e', count),
            Key::Char('0') => self.apply_motion(doc, '0', 1),
            Key::Char('$') => self.apply_motion(doc, '$', 1),
            Key::Char('^') => self.motion_first_non_blank(doc),
            Key::Char('G') => self.motion_last_line(doc, count),
            Key::Char('g') => return self.g_key(doc),
            // The Delete key deletes like `x`, including under counts.
            Key::Char('x') | Key::Delete => self.delete_x(doc, count),
            Key::Char('d') => return self.d_key(doc),
            Key::Char('c') => return self.operator_key(doc, PendingOp::Change),
            Key::Char('y') => return self.operator_key(doc, PendingOp::Yank),
            Key::Char('p') => self.paste_key(doc),
            Key::Char('.') => return self.repeat_key(doc),
            Key::Char('u') => self.undo(doc),
            Key::Char('i') => self.insert_key_at(doc, key, Place::AtCursor),
            Key::Char('a') => self.insert_key_at(doc, key, Place::AfterCursor),
            Key::Char('A') => self.insert_key_at(doc, key, Place::LineEnd),
            Key::Char('I') => self.insert_key_at(doc, key, Place::FirstBlank),
            Key::Char('o') => self.insert_key_at(doc, key, Place::LineBelow),
            Key::Char('O') => self.insert_key_at(doc, key, Place::LineAbove),
            Key::Char('v') => self.start_visual(key, Mode::Visual),
            Key::Char('V') => self.start_visual(key, Mode::VisualLine),
            Key::CtrlV => self.start_visual(key, Mode::VisualBlock),
            Key::Char(':') => self.open_command_line(""),
            Key::Char('/') => self.open_command_line("/"),
            Key::Char('n') => return self.search_again(doc, true),
            Key::Char('N') => return self.search_again(doc, false),
            // `K` looks up the cursor word, like vim with `keywordprg` on
            // an LSP: no buffer change, nothing to repeat or record.
            Key::Char('K') => {
                self.clear_pending();
                self.rec_discard();
                return vec![Effect::Hover];
            }
            // Vim parity: Backspace moves left like `h` in Normal mode.
            Key::Backspace => self.apply_motion(doc, 'h', 1),
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

    /// `^`: first non-blank of the cursor line.
    fn motion_first_non_blank<B: Buffer>(&mut self, doc: &mut B) {
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

    /// `G`: the last line, or line `count`.
    fn motion_last_line<B: Buffer>(&mut self, doc: &mut B, count: usize) {
        let line = if self.count.is_empty() {
            doc.line_count().saturating_sub(1)
        } else {
            count.saturating_sub(1)
        };
        let col = Self::first_non_blank(doc, line.min(doc.line_count().saturating_sub(1)));
        self.apply_motion_to(doc, Cursor { line, col }, 'G');
        self.scroll_request = true;
    }

    /// `g` arms the prefix; `gg` goes to the first line.
    fn g_key<B: Buffer>(&mut self, doc: &mut B) -> Vec<Effect> {
        if !self.pending_g {
            self.pending_g = true;
            return Vec::new();
        }
        self.clear_pending();
        let col = Self::first_non_blank(doc, 0);
        self.apply_motion_to(doc, Cursor { line: 0, col }, 'g');
        self.scroll_request = true;
        Vec::new()
    }

    /// `gd` is goto-definition; a plain `d` starts the delete operator.
    fn d_key<B: Buffer>(&mut self, doc: &mut B) -> Vec<Effect> {
        if self.pending_g {
            self.clear_pending();
            self.rec_discard();
            self.scroll_request = true;
            return vec![Effect::GotoDefinition];
        }
        self.operator_key(doc, PendingOp::Delete)
    }

    fn delete_x<B: Buffer>(&mut self, doc: &mut B, count: usize) {
        self.rec_bit(Bit::Key(Key::Char('x')));
        self.delete_chars_forward(doc, count);
        self.rec_commit();
    }

    fn paste_key<B: Buffer>(&mut self, doc: &mut B) {
        self.rec_bit(Bit::Key(Key::Char('p')));
        self.paste(doc);
        self.clear_pending();
        self.rec_commit();
    }

    /// `.` replays the last change; it bells under an operator or with
    /// nothing to repeat.
    fn repeat_key<B: Buffer>(&mut self, doc: &mut B) -> Vec<Effect> {
        if self.pending.is_some() {
            self.clear_pending();
            self.rec_discard();
            return vec![Effect::Bell];
        }
        let Some(change) = self.last_change.clone() else {
            return vec![Effect::Bell];
        };
        self.exec_repeat(doc, change)
    }

    fn insert_key_at<B: Buffer>(&mut self, doc: &mut B, key: Key, place: Place) {
        self.rec_bit(Bit::Key(key));
        self.start_insert_place(doc, place);
    }

    fn start_visual(&mut self, key: Key, mode: Mode) {
        self.rec_bit(Bit::Key(key));
        self.mode = mode;
        self.anchor = self.cursor;
        self.clear_pending();
    }

    /// `:` opens an empty command line, `/` one starting with `/`.
    fn open_command_line(&mut self, prefix: &str) {
        self.mode = Mode::Command;
        self.command.clear();
        self.command.push_str(prefix);
        self.clear_pending();
        self.rec_discard();
    }

    /// `n` / `N`: the next match of the last search after (or before) the
    /// cursor, wrapping.
    fn search_again<B: Buffer>(&mut self, doc: &mut B, forward: bool) -> Vec<Effect> {
        if self.search_last.is_empty() {
            self.rec_discard();
            return vec![Effect::Bell];
        }
        let pattern = self.search_last.clone();
        let at = self.char_idx(doc, self.cursor);
        let from = if forward { at.saturating_add(1) } else { at };
        self.clear_pending();
        self.rec_discard();
        if !self.jump_to_match(
            doc,
            JumpToMatchInput {
                pattern: &pattern,
                from,
                forward,
            },
        ) {
            return vec![Effect::Bell];
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
            let Some((target, inclusive)) = self.operator_target(doc, kind, count) else {
                self.rec_discard();
                return;
            };
            self.apply_operator(
                doc,
                ApplyOperatorInput {
                    op,
                    target,
                    inclusive,
                },
            );
            return;
        }
        self.pending_g = false;
        self.count.clear();
        self.rec_discard();
        self.move_by(doc, kind, count);
        doc.end_run();
    }

    /// Target and inclusivity of `kind` under an operator; `None` for
    /// motions operators do not take.
    fn operator_target<B: Buffer>(
        &mut self,
        doc: &mut B,
        kind: char,
        count: usize,
    ) -> Option<(Cursor, bool)> {
        match kind {
            'h' | 'l' | '0' | '$' | 'w' | 'b' | 'e' => Some(self.motion_target(doc, kind, count)),
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
                Some((target, target.line != saved.line))
            }
            _ => None,
        }
    }

    /// Plain cursor motion (no operator pending).
    fn move_by<B: Buffer>(&mut self, doc: &mut B, kind: char, count: usize) {
        match kind {
            'h' => self.move_h(doc, count),
            'j' => self.move_j(doc, count),
            'k' => self.move_k(doc, count),
            'l' => self.move_l(doc, count),
            'w' | 'b' | 'e' => {
                let (target, _) = self.motion_target(doc, kind, count);
                self.goto(
                    doc,
                    GotoInput {
                        line: target.line,
                        col: target.col,
                        preferred: true,
                    },
                );
            }
            '0' => self.goto(
                doc,
                GotoInput {
                    line: self.cursor.line,
                    col: 0,
                    preferred: true,
                },
            ),
            '$' => {
                let len = line_len(doc, self.cursor.line);
                self.goto(
                    doc,
                    GotoInput {
                        line: self.cursor.line,
                        col: len.saturating_sub(1),
                        preferred: true,
                    },
                );
            }
            _ => {}
        }
    }

    pub(super) fn apply_motion_to<B: Buffer>(&mut self, doc: &mut B, target: Cursor, motion: char) {
        if let Some(op) = self.pending.take() {
            self.pending_g = false;
            self.count.clear();
            self.rec_bit(Bit::Motion(motion));
            self.apply_operator(
                doc,
                ApplyOperatorInput {
                    op,
                    target,
                    inclusive: false,
                },
            );
            return;
        }
        self.pending_g = false;
        self.count.clear();
        self.rec_discard();
        self.goto(
            doc,
            GotoInput {
                line: target.line,
                col: target.col,
                preferred: true,
            },
        );
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
