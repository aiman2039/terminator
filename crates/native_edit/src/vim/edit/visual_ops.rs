//! Visual-mode edits beyond `d`/`y`/`c`: case, join, replace, put,
//! linewise `D`/`Y`/`C`, and block insert.
use super::super::super::doc::Buffer;
use super::super::helpers::range_text;
use super::super::mode::{Bit, Cursor, Effect, Key, Mode, PendingOp};
use super::super::state::VimEngine;
use super::case;

impl VimEngine {
    /// Visual editing keys; `None` when `key` is not one.
    pub(crate) fn visual_edit<B: Buffer>(&mut self, doc: &mut B, key: Key) -> Option<Vec<Effect>> {
        Some(match key {
            Key::Char('u') => self.visual_case(doc, PendingOp::Lower),
            Key::Char('U') => self.visual_case(doc, PendingOp::Upper),
            Key::Char('~') => self.visual_case(doc, PendingOp::Toggle),
            Key::Char('J') => self.visual_join(doc, true),
            Key::Char(c @ ('p' | 'P')) => self.visual_put(doc, c == 'P'),
            Key::Char(c @ ('D' | 'X' | 'Y' | 'C' | 'S' | 'R')) => {
                let op = match c {
                    'Y' => PendingOp::Yank,
                    'D' | 'X' => PendingOp::Delete,
                    _ => PendingOp::Change,
                };
                self.x.keys_change = true;
                let (first, last) = self.ide_line_span();
                self.mode = Mode::Normal;
                self.operate_lines(doc, op, (first, last));
                Vec::new()
            }
            // Block insert: `I` at the left edge, `A` past the right edge.
            // Typing replays on every other row at Escape.
            Key::Char(c @ ('I' | 'A')) if self.mode == Mode::VisualBlock => {
                self.rec_bit(Bit::Key(key));
                self.start_block_insert(doc, c == 'A');
                Vec::new()
            }
            _ => return None,
        })
    }

    /// Visual `u` / `U` / `~` (and `gu`/`gU`/`g~`).
    pub(crate) fn visual_case<B: Buffer>(&mut self, doc: &mut B, op: PendingOp) -> Vec<Effect> {
        self.x.keys_change = true;
        if self.mode == Mode::VisualBlock {
            return self.map_block(doc, |text| case::convert(text, op));
        }
        let linewise = self.mode == Mode::VisualLine;
        let (from, to) = self.selection_range(doc, linewise);
        self.mode = Mode::Normal;
        self.operate_chars(doc, op, (from, to));
        Vec::new()
    }

    /// Visual `J` / `gJ`: join the selected lines (at least two).
    pub(crate) fn visual_join<B: Buffer>(&mut self, doc: &mut B, spaces: bool) -> Vec<Effect> {
        let (first, last) = self.ide_line_span();
        let last = last
            .max(first.saturating_add(1))
            .min(Self::last_addressable(doc));
        self.x.keys_change = true;
        self.mode = Mode::Normal;
        if last > first {
            self.join_lines(doc, (first, last), spaces);
        }
        self.enter_normal(doc);
        self.rec_commit();
        Vec::new()
    }

    /// Visual `r{c}`: every selected char becomes `c` (newlines stay).
    pub(crate) fn visual_replace<B: Buffer>(&mut self, doc: &mut B, ch: char) -> Vec<Effect> {
        self.x.keys_change = true;
        if self.mode == Mode::VisualBlock {
            return self.map_block(doc, |text| text.chars().map(|_| ch).collect());
        }
        let (from, to) = self.selection_range(doc, self.mode == Mode::VisualLine);
        let text = range_text(doc, from, to);
        let filled: String = text
            .chars()
            .map(|c| if c == '\n' { c } else { ch })
            .collect();
        self.replace_text(doc, (from, to), &filled);
        self.cursor = Self::cursor_at(doc, from);
        self.enter_normal(doc);
        self.rec_commit();
        Vec::new()
    }

    /// Rewrite each row's slice of the live block with `f`, as one undo
    /// unit, then park at the block's top-left in Normal mode.
    fn map_block<B: Buffer>(&mut self, doc: &mut B, f: impl Fn(&str) -> String) -> Vec<Effect> {
        let Some((top, _, left, _)) = self.block_span() else {
            return Vec::new();
        };
        let rows = self.block_row_ranges(doc).unwrap_or_default();
        doc.begin_undo_group();
        for (line, from, to) in rows {
            let from = doc.char_at_line_col(line, from);
            let to = doc.char_at_line_col(line, to);
            let text = range_text(doc, from, to);
            self.replace_text(doc, (from, to), &f(&text));
        }
        doc.end_undo_group();
        self.cursor = Cursor {
            line: top,
            col: left,
        };
        self.enter_normal(doc);
        self.rec_commit();
        Vec::new()
    }

    /// Visual `p`: replace the selection with the register; the deleted
    /// text then fills the unnamed register (`P` keeps it).
    fn visual_put<B: Buffer>(&mut self, doc: &mut B, keep: bool) -> Vec<Effect> {
        let Some(reg) = self.put_source() else {
            return vec![Effect::Bell];
        };
        self.x.keys_change = true;
        let linewise = self.mode == Mode::VisualLine;
        doc.begin_undo_group();
        let (from, to) = self.selection_range(doc, linewise);
        let mut old = range_text(doc, from, to);
        if linewise && old.ends_with('\n') {
            old.pop();
        }
        self.cursor = self.delete_range(doc, from, to);
        let text = if linewise {
            format!("{}\n", reg.text)
        } else if reg.linewise {
            format!("\n{}\n", reg.text)
        } else {
            reg.text.clone()
        };
        // A linewise selection at the end of the buffer lost its newline.
        let text = if linewise && from >= doc.len_chars() && from > 0 {
            format!("\n{}", text.trim_end_matches('\n'))
        } else {
            text
        };
        doc.insert(from, &text);
        doc.end_undo_group();
        doc.end_run();
        if !keep {
            self.x.reg_sel = None;
            self.write_delete(old, linewise);
        }
        self.cursor = Self::cursor_at(doc, from);
        self.enter_normal(doc);
        self.rec_commit();
        Vec::new()
    }
}
