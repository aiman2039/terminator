use super::super::doc::Buffer;
use super::mode::{Cursor, Mode};
use super::state::VimEngine;
use super::words::line_len;
impl VimEngine {
    /// Last cursor-addressable line. A trailing empty line exists only to
    /// terminate the final newline (vim shows no line there), so it is
    /// excluded; every genuine line, empty or not, stays reachable.
    pub(super) fn last_addressable<B: Buffer>(doc: &B) -> usize {
        let lines = doc.line_count().max(1);
        let prev = lines.saturating_sub(1);
        if lines > 1 && doc.line_text(prev).is_empty() {
            lines.saturating_sub(2)
        } else {
            prev
        }
    }

    pub(super) fn clamp<B: Buffer>(&mut self, doc: &B) {
        let last = if self.mode == Mode::Insert {
            // Insert may sit just past a trailing newline (the cursor stays
            // where editing left it), so backspace can delete that newline
            // and keep going into the previous line.
            doc.line_count().saturating_sub(1)
        } else {
            Self::last_addressable(doc)
        };
        self.cursor.line = self.cursor.line.min(last);
        let len = line_len(doc, self.cursor.line);
        let max_col = if self.mode == Mode::Insert || len == 0 {
            len
        } else {
            len.saturating_sub(1)
        };
        self.cursor.col = self.cursor.col.min(max_col);
        self.anchor.line = self.anchor.line.min(last);
        let alen = line_len(doc, self.anchor.line);
        self.anchor.col = self.anchor.col.min(alen);
    }

    /// Editing can leave the cursor just past a trailing newline, a spot only
    /// insert mode holds. Other modes land at the end of the last real line
    /// (vim has no cursor past the final newline) instead of line start.
    pub(super) fn settle_cursor<B: Buffer>(&mut self, doc: &B) {
        if self.mode == Mode::Insert {
            return;
        }
        let last = Self::last_addressable(doc);
        if self.cursor.line > last {
            self.cursor = Cursor {
                line: last,
                col: line_len(doc, last),
            };
        }
    }

    pub(super) fn char_idx<B: Buffer>(&self, doc: &B, cursor: Cursor) -> usize {
        doc.char_at_line_col(cursor.line, cursor.col)
    }

    pub(super) fn goto<B: Buffer>(&mut self, doc: &B, line: usize, col: usize, preferred: bool) {
        self.cursor.line = line;
        self.cursor.col = col;
        if preferred {
            self.preferred_col = col;
        }
        self.clamp(doc);
    }

    pub(super) fn first_non_blank<B: Buffer>(doc: &B, line: usize) -> usize {
        doc.line_text(line)
            .chars()
            .take_while(|c| c.is_whitespace())
            .count()
    }

    pub(super) fn move_h<B: Buffer>(&mut self, doc: &B, count: usize) {
        let col = self.cursor.col.saturating_sub(count);
        self.goto(doc, self.cursor.line, col, true);
    }

    pub(super) fn move_l<B: Buffer>(&mut self, doc: &B, count: usize) {
        let col = self.cursor.col.saturating_add(count);
        self.goto(doc, self.cursor.line, col, true);
    }

    pub(super) fn move_j<B: Buffer>(&mut self, doc: &B, count: usize) {
        let line = self.cursor.line.saturating_add(count);
        self.cursor.line = line;
        self.cursor.col = self.preferred_col;
        self.clamp(doc);
    }

    pub(super) fn move_k<B: Buffer>(&mut self, doc: &B, count: usize) {
        let line = self.cursor.line.saturating_sub(count);
        self.cursor.line = line;
        self.cursor.col = self.preferred_col;
        self.clamp(doc);
    }

    /// Begin an insert session: one undo unit covering the session (and a
    /// preceding operator delete for `c`/`cc`, whose caller opens the unit
    /// first so delete-plus-insert undoes as one change). Re-entering
    /// while a unit is open (operator delete, then typing) joins it.
    pub(super) fn begin_insert_session<B: Buffer>(&mut self, doc: &mut B) {
        if !self.insert_group {
            doc.begin_undo_group();
            self.insert_group = true;
        }
        self.mode = Mode::Insert;
        self.clear_pending();
        self.typed_exact = true;
        self.typed_start = self.char_idx(doc, self.cursor);
        self.typed_end = self.typed_start;
    }

    /// Open the unit for an operator delete that flows into insert
    /// (`c{motion}`, `cc`): the delete banks into the session's unit.
    pub(super) fn begin_change_group<B: Buffer>(&mut self, doc: &mut B) {
        if !self.insert_group {
            doc.begin_undo_group();
            self.insert_group = true;
        }
    }

    pub(super) fn enter_normal<B: Buffer>(&mut self, doc: &mut B) {
        let ending_insert = self.mode == Mode::Insert;
        self.mode = Mode::Normal;
        self.clear_pending();
        self.command.clear();
        if ending_insert {
            // Block insert replays the session's net typed text on the
            // remaining rows, inside the same undo unit. Only pure typing
            // on the top row replays: cursor moves or newlines end the
            // session without replicating (buffer stays correct).
            let pending = self.block_pending.take();
            if let Some(block) = pending
                && self.cursor.line == block.top
                && self.typed_exact
            {
                let text = Self::typed_text(&self.recording);
                if !text.is_empty() {
                    self.apply_block_insert(doc, &block, &text);
                }
            }
            if !self.replaying {
                self.last_change = if self.typed_exact {
                    Self::parse_repeat(&self.recording, pending)
                } else {
                    None
                };
            }
            self.recording.clear();
            if self.insert_group {
                doc.end_undo_group();
                self.insert_group = false;
            }
        }
        doc.end_run();
        self.settle_cursor(doc);
        self.clamp(doc);
    }
}
