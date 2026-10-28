//! Visual-mode keys and the selection helpers they share with operators.
use super::super::doc::Buffer;
use super::cursor::GotoInput;
use super::helpers::range_text;
use super::mode::{Bit, Effect, Key, Mode, Repeat};
use super::operators::DeleteBlockRangesInput;
use super::state::VimEngine;
use super::words::line_len;

impl VimEngine {
    pub(super) fn visual_key<B: Buffer>(&mut self, doc: &mut B, key: Key) -> Vec<Effect> {
        let linewise = self.mode == Mode::VisualLine;
        let key = if key == Key::Char(' ') {
            Key::Char('l')
        } else {
            key
        };
        // Digits accumulate counts for motions.
        if let Key::Char(c) = key
            && c.is_ascii_digit()
            && !(c == '0' && self.count.is_empty())
        {
            self.count.push(c);
            return Vec::new();
        }
        let count = self.take_count();
        match key {
            Key::Escape => {
                self.enter_normal(doc);
                self.rec_discard();
            }
            Key::Tab | Key::Char('>') => self.indent_selection(doc, false),
            Key::BackTab | Key::Char('<') => self.indent_selection(doc, true),
            // Mode switches keep the anchor: `v`/`V`/Ctrl-V reshape the
            // same selection instead of restarting it.
            Key::Char('v') => self.switch_visual(key, Mode::Visual),
            Key::Char('V') => self.switch_visual(key, Mode::VisualLine),
            Key::CtrlV => self.toggle_visual_block(doc),
            // `o` swaps the active end, so both rectangle corners (and both
            // charwise ends) stay adjustable without restarting.
            Key::Char('o') => {
                std::mem::swap(&mut self.anchor, &mut self.cursor);
                self.preferred_col = self.cursor.col;
                self.clear_pending();
            }
            Key::Char('h') | Key::Left => self.move_h(doc, count),
            Key::Char('j') | Key::Down => self.move_j(doc, count),
            Key::Char('k') | Key::Up => self.move_k(doc, count),
            Key::Char('l') | Key::Right => self.move_l(doc, count),
            Key::Char('w' | 'W') => self.visual_word_motion(doc, 'w', count),
            Key::Char('b' | 'B') => self.visual_word_motion(doc, 'b', count),
            Key::Char('e' | 'E') => self.visual_word_motion(doc, 'e', count),
            Key::Char('0') => self.visual_goto_col(doc, 0),
            Key::Char('$') => {
                let len = line_len(doc, self.cursor.line);
                self.visual_goto_col(doc, len.saturating_sub(1));
            }
            Key::Char('y') => self.visual_yank(doc, linewise),
            // Vim parity: Backspace deletes the selection like `x`.
            Key::Char('d' | 'x') | Key::Backspace | Key::Delete => {
                self.visual_delete(doc, linewise);
            }
            Key::Char('c') => self.visual_change(doc, linewise),
            // Block insert: `I` at the left edge, `A` past the right edge.
            // Typing replays on every other row at Escape.
            Key::Char('I') if self.mode == Mode::VisualBlock => {
                self.rec_bit(Bit::Key(key));
                self.start_block_insert(doc, false);
            }
            Key::Char('A') if self.mode == Mode::VisualBlock => {
                self.rec_bit(Bit::Key(key));
                self.start_block_insert(doc, true);
            }
            _ => {
                self.rec_discard();
                return vec![Effect::Bell];
            }
        }
        Vec::new()
    }

    fn switch_visual(&mut self, key: Key, mode: Mode) {
        self.rec_bit(Bit::Key(key));
        self.mode = mode;
        self.clear_pending();
    }

    /// Ctrl-V: enter block mode, or leave it when already blockwise.
    fn toggle_visual_block<B: Buffer>(&mut self, doc: &mut B) {
        self.rec_bit(Bit::Key(Key::CtrlV));
        if self.mode == Mode::VisualBlock {
            self.enter_normal(doc);
            self.rec_discard();
        } else {
            self.mode = Mode::VisualBlock;
            self.clear_pending();
        }
    }

    /// `w`/`b`/`e` (and their WORD forms) move the active end.
    fn visual_word_motion<B: Buffer>(&mut self, doc: &mut B, kind: char, count: usize) {
        let (target, _) = self.motion_target(doc, kind, count);
        self.goto(
            doc,
            GotoInput {
                line: target.line,
                col: target.col,
                preferred: true,
            },
        );
        self.count.clear();
    }

    fn visual_goto_col<B: Buffer>(&mut self, doc: &mut B, col: usize) {
        self.goto(
            doc,
            GotoInput {
                line: self.cursor.line,
                col,
                preferred: true,
            },
        );
        self.count.clear();
    }

    fn visual_yank<B: Buffer>(&mut self, doc: &mut B, linewise: bool) {
        if self.mode == Mode::VisualBlock {
            self.yank_block(doc);
        } else {
            self.yank_selection(doc, linewise);
        }
        self.enter_normal(doc);
        self.rec_discard();
    }

    fn visual_delete<B: Buffer>(&mut self, doc: &mut B, linewise: bool) {
        if self.mode != Mode::VisualBlock {
            self.rec_bit(Bit::Key(Key::Char('v')));
            self.delete_selection(doc, linewise);
            self.enter_normal(doc);
            // Charwise visual changes are not repeatable yet: the repeat
            // bells instead of replaying stale input.
            if !self.replaying {
                self.last_change = None;
                self.recording.clear();
            }
            return;
        }
        // Capture the rectangle before deleting: a blockwise delete
        // replays from dims, not from motion bits.
        let span = self.block_span();
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
    }

    fn visual_change<B: Buffer>(&mut self, doc: &mut B, linewise: bool) {
        if self.mode != Mode::VisualBlock {
            self.rec_bit(Bit::Key(Key::Char('v')));
            self.rec_bit(Bit::Key(Key::Char('c')));
            self.delete_selection(doc, linewise);
            self.begin_insert_session(doc);
            self.clamp(doc);
            return;
        }
        self.begin_change_group(doc);
        if let Some((top, bottom, left, right)) = self.block_span() {
            self.delete_block_ranges(
                doc,
                DeleteBlockRangesInput {
                    top,
                    bottom,
                    left,
                    right_excl: right,
                },
            );
        }
        self.rec_bit(Bit::Key(Key::Char('c')));
        self.start_block_insert(doc, false);
    }

    /// Shift whole selected lines by one Markdown-compatible indent level.
    /// Keep the selection active so repeated Tab/Shift-Tab can adjust it.
    fn indent_selection<B: Buffer>(&mut self, doc: &mut B, remove: bool) {
        let (top, bottom) = self.ide_line_span();
        doc.begin_undo_group();
        for line in (top..=bottom).rev() {
            let at = doc.char_at_line_col(line, 0);
            let amount = if remove {
                let text = doc.line_text(line);
                let amount = if text.starts_with('\t') {
                    1
                } else {
                    text.chars().take(4).take_while(|c| *c == ' ').count()
                };
                doc.delete(at..at.saturating_add(amount));
                amount
            } else {
                doc.insert(at, "    ");
                4
            };
            for end in [&mut self.anchor, &mut self.cursor] {
                if end.line == line {
                    end.col = if remove {
                        end.col.saturating_sub(amount)
                    } else {
                        end.col.saturating_add(amount)
                    };
                }
            }
        }
        doc.end_undo_group();
        self.preferred_col = self.cursor.col;
        self.clear_pending();
        self.rec_discard();
        // Indentation is not yet represented by the dot-repeat engine.
        self.last_change = None;
        self.clamp(doc);
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
        self.set_register(text, linewise, None);
        self.cursor = self.anchor;
    }

    pub(super) fn delete_selection<B: Buffer>(&mut self, doc: &mut B, linewise: bool) {
        let (from, to) = self.selection_range(doc, linewise);
        let cursor = self.delete_range(doc, from, to);
        self.cursor = cursor;
    }
}
