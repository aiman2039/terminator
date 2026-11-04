//! Visual-mode keys and the selection helpers they share with operators.
use super::super::doc::Buffer;
use super::helpers::range_text;
use super::mode::{Bit, Cursor, Effect, Key, Mode, PendingOp, Prefix, Repeat};
use super::motion::{Count, Mo};
use super::state::VimEngine;

impl VimEngine {
    pub(super) fn visual_key<B: Buffer>(&mut self, doc: &mut B, key: Key) -> Vec<Effect> {
        let key = if key == Key::Char(' ') {
            Key::Char('l')
        } else {
            key
        };
        // The state each key starts from is the one `gv` and `'<` recall.
        self.x.last_visual = Some((self.mode, self.anchor, self.cursor));
        // `g` waits for its key but still takes count digits.
        if let Some(prefix) = self.x.prefix.take_if(|p| *p != Prefix::G) {
            return self.visual_prefix(doc, prefix, key);
        }
        // Digits accumulate counts for motions.
        if let Key::Char(c) = key
            && c.is_ascii_digit()
            && !(c == '0' && self.count.is_empty())
        {
            self.count.push(c);
            return Vec::new();
        }
        let count = self.take_count();
        if self.x.prefix.take().is_some() {
            return self.visual_g(doc, key, count);
        }
        if let Some(effects) = self.visual_command(doc, key, count) {
            return effects;
        }
        if let Some(mo) = Self::motion_for_key(key) {
            return self.visual_motion(doc, mo, count);
        }
        if let Some(effects) = self.visual_edit(doc, key) {
            return effects;
        }
        self.count.clear();
        self.rec_discard();
        vec![Effect::Bell]
    }

    /// Keys that wait for one more char in Visual mode.
    fn visual_prefix<B: Buffer>(&mut self, doc: &mut B, prefix: Prefix, key: Key) -> Vec<Effect> {
        let count = self.take_count();
        self.count.clear();
        let Key::Char(ch) = key else {
            return Vec::new();
        };
        match prefix {
            Prefix::Find(kind) => self.visual_motion(doc, Mo::Find { kind, ch }, count),
            Prefix::Jump { exact } => self.visual_motion(doc, Mo::Mark { name: ch, exact }, count),
            Prefix::Object { inner } => self.visual_object(doc, (ch, inner), count),
            Prefix::Replace => self.visual_replace(doc, ch),
            Prefix::Register => {
                self.x.reg_sel = Some(ch);
                Vec::new()
            }
            _ => vec![Effect::Bell],
        }
    }

    /// `w`/`b`/`G`/`f`/`%`/… move the active end.
    fn visual_motion<B: Buffer>(&mut self, doc: &mut B, mo: Mo, count: usize) -> Vec<Effect> {
        let typed = self.has_count();
        self.count.clear();
        let Some(motion) = self.resolve(doc, mo, Count { n: count, typed }) else {
            return vec![Effect::Bell];
        };
        self.x.block_eol = mo == Mo::LineEnd && self.mode == Mode::VisualBlock;
        self.move_to(doc, mo, motion);
        Vec::new()
    }

    /// `g` in Visual: `gg`, `ge`, `gJ`, `gu`/`gU`/`g~`, `gv`.
    fn visual_g<B: Buffer>(&mut self, doc: &mut B, key: Key, count: usize) -> Vec<Effect> {
        match key {
            Key::Char('g') => self.visual_motion(doc, Mo::GotoLine { last: false }, count),
            Key::Char(c @ ('e' | 'E')) => {
                self.visual_motion(doc, Mo::EndBack { big: c == 'E' }, count)
            }
            Key::Char('J') => self.visual_join(doc, false),
            Key::Char('u') => self.visual_case(doc, PendingOp::Lower),
            Key::Char('U') => self.visual_case(doc, PendingOp::Upper),
            Key::Char('~') => self.visual_case(doc, PendingOp::Toggle),
            _ => vec![Effect::Bell],
        }
    }

    /// Mode switches, ends, prefixes, and the operators.
    fn visual_command<B: Buffer>(
        &mut self,
        doc: &mut B,
        key: Key,
        count: usize,
    ) -> Option<Vec<Effect>> {
        let linewise = self.mode == Mode::VisualLine;
        let prefix = Self::prefix_for_key(key).or(match key {
            Key::Char('r') => Some(Prefix::Replace),
            Key::Char('"') => Some(Prefix::Register),
            _ => None,
        });
        if let Some(prefix) = prefix {
            self.x.prefix = Some(prefix);
            return Some(Vec::new());
        }
        Some(match key {
            Key::Escape => {
                self.enter_normal(doc);
                self.rec_discard();
                Vec::new()
            }
            Key::Tab | Key::Char('>') => self.indent_selection(doc, false, count),
            Key::BackTab | Key::Char('<') => self.indent_selection(doc, true, count),
            // Mode switches keep the anchor: `v`/`V`/Ctrl-V reshape the
            // same selection instead of restarting it.
            Key::Char('v') => self.switch_visual(doc, key, Mode::Visual),
            Key::Char('V') => self.switch_visual(doc, key, Mode::VisualLine),
            Key::CtrlV => self.toggle_visual_block(doc),
            // `o` swaps the active end, so both rectangle corners (and both
            // charwise ends) stay adjustable without restarting.
            Key::Char('o' | 'O') => {
                std::mem::swap(&mut self.anchor, &mut self.cursor);
                self.preferred_col = self.cursor.col;
                self.clear_pending();
                Vec::new()
            }
            Key::Char('y') => self.visual_yank(doc, linewise),
            // Vim parity: Backspace deletes the selection like `x`.
            Key::Char('d' | 'x') | Key::Backspace | Key::Delete => {
                self.visual_delete(doc, linewise)
            }
            Key::Char('c' | 's') => self.visual_change(doc, linewise),
            Key::Char(':') => {
                self.enter_normal(doc);
                self.rec_discard();
                self.begin_ex("'<,'>");
                Vec::new()
            }
            _ => return None,
        })
    }

    fn switch_visual<B: Buffer>(&mut self, doc: &mut B, key: Key, mode: Mode) -> Vec<Effect> {
        // `v` in charwise Visual (and `V` in linewise) ends it.
        if self.mode == mode {
            self.enter_normal(doc);
            self.rec_discard();
            return Vec::new();
        }
        self.rec_bit(Bit::Key(key));
        self.mode = mode;
        self.clear_pending();
        Vec::new()
    }

    /// Ctrl-V: enter block mode, or leave it when already blockwise.
    fn toggle_visual_block<B: Buffer>(&mut self, doc: &mut B) -> Vec<Effect> {
        self.rec_bit(Bit::Key(Key::CtrlV));
        if self.mode == Mode::VisualBlock {
            self.enter_normal(doc);
            self.rec_discard();
        } else {
            self.mode = Mode::VisualBlock;
            self.clear_pending();
        }
        Vec::new()
    }

    /// `iw`, `a(`, `ip`, … select the object.
    fn visual_object<B: Buffer>(
        &mut self,
        doc: &mut B,
        (obj, inner): (char, bool),
        count: usize,
    ) -> Vec<Effect> {
        let Some(span) = self.text_object(doc, (obj, inner), count) else {
            return vec![Effect::Bell];
        };
        if span.linewise {
            self.mode = Mode::VisualLine;
            self.anchor = Self::cursor_at(doc, span.from);
            self.cursor = Self::cursor_at(doc, span.to);
        } else if span.to > span.from {
            self.anchor = Self::cursor_at(doc, span.from);
            self.cursor = Self::cursor_at(doc, span.to.saturating_sub(1));
        }
        self.preferred_col = self.cursor.col;
        Vec::new()
    }

    fn visual_yank<B: Buffer>(&mut self, doc: &mut B, linewise: bool) -> Vec<Effect> {
        let start = self.selection_start();
        if self.mode == Mode::VisualBlock {
            self.yank_block(doc);
        } else {
            self.yank_selection(doc, linewise);
        }
        // Vim parity: the cursor goes to the start of the yanked text.
        self.cursor = start;
        self.enter_normal(doc);
        self.rec_discard();
        Vec::new()
    }

    /// Top-left of the selection (block: top row, left column).
    fn selection_start(&self) -> Cursor {
        let (a, b) = (self.anchor, self.cursor);
        match self.mode {
            Mode::VisualBlock => Cursor {
                line: a.line.min(b.line),
                col: a.col.min(b.col),
            },
            Mode::VisualLine if a.line != b.line => {
                if a.line < b.line {
                    a
                } else {
                    b
                }
            }
            Mode::VisualLine => b,
            _ => {
                if (a.line, a.col) <= (b.line, b.col) {
                    a
                } else {
                    b
                }
            }
        }
    }

    fn visual_delete<B: Buffer>(&mut self, doc: &mut B, linewise: bool) -> Vec<Effect> {
        if self.mode != Mode::VisualBlock {
            self.rec_bit(Bit::Key(Key::Char('v')));
            self.x.keys_change = true;
            self.delete_selection(doc, linewise);
            self.enter_normal(doc);
            self.rec_commit();
            return Vec::new();
        }
        // Capture the rectangle before deleting: a blockwise delete
        // replays from dims, not from motion bits.
        let span = self.block_span();
        if let Some((text, width)) = self.block_register(doc) {
            self.write_delete_shaped(text, false, Some(width));
        }
        doc.begin_undo_group();
        let cursor = self.delete_selection_block(doc);
        doc.end_undo_group();
        self.cursor = cursor;
        self.preferred_col = cursor.col;
        self.enter_normal(doc);
        if !self.replaying {
            self.last_change = span.map(|(top, bottom, left, right)| Repeat::BlockDelete {
                height: bottom.saturating_sub(top),
                left,
                right: right.saturating_sub(1),
            });
            self.recording.clear();
        }
        Vec::new()
    }

    fn visual_change<B: Buffer>(&mut self, doc: &mut B, linewise: bool) -> Vec<Effect> {
        if self.mode != Mode::VisualBlock {
            self.rec_bit(Bit::Key(Key::Char('v')));
            self.rec_bit(Bit::Key(Key::Char('c')));
            self.x.keys_change = true;
            self.begin_change_group(doc);
            if linewise {
                let (first, last) = self.ide_line_span();
                self.change_line_range(doc, first, last);
                return Vec::new();
            }
            self.delete_selection(doc, linewise);
            self.begin_insert_session(doc);
            self.clamp(doc);
            return Vec::new();
        }
        self.begin_change_group(doc);
        if let Some((text, width)) = self.block_register(doc) {
            self.write_delete_shaped(text, false, Some(width));
        }
        self.delete_selection_block(doc);
        self.rec_bit(Bit::Key(Key::Char('c')));
        self.start_block_insert(doc, false);
        Vec::new()
    }

    /// Shift whole selected lines by one indent level per count. Keep the
    /// selection active so repeated Tab/Shift-Tab can adjust it.
    fn indent_selection<B: Buffer>(
        &mut self,
        doc: &mut B,
        remove: bool,
        count: usize,
    ) -> Vec<Effect> {
        let (top, bottom) = self.ide_line_span();
        let before: Vec<usize> = (top..=bottom)
            .map(|l| super::words::line_len(doc, l))
            .collect();
        doc.begin_undo_group();
        for _ in 0..count.max(1) {
            self.shift_lines(doc, (top, bottom), remove);
        }
        doc.end_undo_group();
        for end in [&mut self.anchor, &mut self.cursor] {
            if let Some(old) = end.line.checked_sub(top).and_then(|i| before.get(i)) {
                let new = super::words::line_len(doc, end.line);
                end.col = if new >= *old {
                    end.col.saturating_add(new.saturating_sub(*old))
                } else {
                    end.col.saturating_sub(old.saturating_sub(new))
                };
            }
        }
        self.preferred_col = self.cursor.col;
        self.clear_pending();
        self.rec_discard();
        // Indentation is not yet represented by the dot-repeat engine.
        self.last_change = None;
        self.clamp(doc);
        Vec::new()
    }

    pub(super) fn selection_range<B: Buffer>(&self, doc: &B, linewise: bool) -> (usize, usize) {
        // Order-aware: after `o` the cursor can sit before the anchor, and
        // the inclusive end must extend the far end, not the cursor's.
        let a = self.char_idx(doc, self.anchor);
        let b = self.char_idx(doc, self.cursor);
        let (a, mut b) = (a.min(b), a.max(b));
        if linewise {
            let (al, _) = doc.line_col_at(a);
            let (bl, _) = doc.line_col_at(b);
            let (lo, hi) = (al.min(bl), al.max(bl));
            let from = doc.char_at_line_col(lo, 0);
            b = if hi
                .checked_add(1)
                .is_some_and(|next| next < doc.line_count())
            {
                doc.char_at_line_col(hi.checked_add(1).unwrap_or(hi), 0)
            } else {
                doc.len_chars()
            };
            return (from, b);
        }
        // Modeless selections end at the caret; vim's include its cell.
        if self.modeless {
            return (a, b);
        }
        b = b
            .checked_add(1)
            .map_or(doc.len_chars(), |next| next.min(doc.len_chars()));
        (a.min(b), a.max(b))
    }

    pub(super) fn yank_selection<B: Buffer>(&mut self, doc: &mut B, linewise: bool) {
        let (from, to) = self.selection_range(doc, linewise);
        let mut text = range_text(doc, from, to);
        if linewise && text.ends_with('\n') {
            text.pop();
        }
        self.write_yank(text, linewise, None);
        self.cursor = self.anchor;
    }

    /// Delete the selection into the register (linewise or charwise).
    pub(super) fn delete_selection<B: Buffer>(&mut self, doc: &mut B, linewise: bool) {
        let (from, to) = self.selection_range(doc, linewise);
        let mut text = range_text(doc, from, to);
        if linewise && text.ends_with('\n') {
            text.pop();
        }
        if from < to {
            self.write_delete(text, linewise);
        }
        let cursor = self.delete_range(doc, from, to);
        self.cursor = cursor;
        if linewise {
            self.cursor.col = Self::first_non_blank(doc, self.cursor.line);
        }
    }
}
