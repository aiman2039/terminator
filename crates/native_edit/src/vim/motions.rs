use super::super::doc::Buffer;
use super::mode::{BlockPending, Cursor, Mode, Place};
use super::state::VimEngine;
use super::words::{flat_text, line_len, word_backward, word_end, word_forward};
impl VimEngine {
    /// Drive an operator motion the way the live keys do (including the
    /// `cw`→`ce` special case), for `.` replay.
    pub(super) fn replay_motion<B: Buffer>(&mut self, doc: &mut B, motion: char, count: usize) {
        match motion {
            '^' | 'G' | 'g' => {
                let target = self.goto_target(doc, motion, count);
                self.apply_motion_to(doc, target, motion);
            }
            _ => {
                self.apply_motion(doc, motion, count.max(1));
            }
        }
    }

    /// Cursor target for the count-aware motions (`^`, `G`, `gg`), shared
    /// by the live keys and `.` replay. A `G` count of 0 means bare `G`
    /// (last line); every other motion treats 0 as 1.
    pub(super) fn goto_target<B: Buffer>(&self, doc: &B, motion: char, count: usize) -> Cursor {
        match motion {
            '^' => Cursor {
                line: self.cursor.line,
                col: Self::first_non_blank(doc, self.cursor.line),
            },
            'G' => {
                let last = doc.line_count().saturating_sub(1);
                let line = if count == 0 {
                    last
                } else {
                    count.saturating_sub(1).min(last)
                };
                Cursor {
                    line,
                    col: Self::first_non_blank(doc, line),
                }
            }
            _ => Cursor {
                line: 0,
                col: Self::first_non_blank(doc, 0),
            },
        }
    }

    pub(super) fn word_target<B: Buffer>(
        &self,
        doc: &B,
        kind: char,
        count: usize,
    ) -> (Cursor, bool) {
        let text = flat_text(doc);
        let from = self.char_idx(doc, self.cursor);
        let (idx, inclusive) = match kind {
            'w' => (word_forward(&text, from, count), false),
            'b' => (word_backward(&text, from, count), false),
            'e' => (word_end(&text, from, count), true),
            _ => (from, false),
        };
        let total: usize = text.len();
        let idx = idx.min(total);
        // Map the flat index back to line/col by walking line lengths.
        let mut rest = idx;
        let mut line = 0;
        while line < doc.line_count() {
            let len = line_len(doc, line);
            if rest <= len || line.checked_add(1) == Some(doc.line_count()) {
                break;
            }
            let step = len.checked_add(1).unwrap_or(len);
            rest = rest.saturating_sub(step);
            line = line.checked_add(1).unwrap_or(line);
        }
        (Cursor { line, col: rest }, inclusive)
    }

    /// Park the cursor for an insert session and open its undo unit.
    /// Used by the live keys and `.` replay alike. The unit opens before
    /// positioning edits (`o`/`O` newlines), so the whole change undoes
    /// as one; typed-text tracking still starts from the parked cursor.
    pub(super) fn start_insert_place<B: Buffer>(&mut self, doc: &mut B, place: Place) {
        self.begin_change_group(doc);
        match place {
            Place::AtCursor => {}
            Place::AfterCursor => {
                let len = line_len(doc, self.cursor.line);
                if len > 0 {
                    self.cursor.col = self
                        .cursor
                        .col
                        .checked_add(1)
                        .map_or(len, |col| col.min(len));
                }
            }
            Place::LineEnd => {
                self.cursor.col = line_len(doc, self.cursor.line);
            }
            Place::FirstBlank => {
                self.cursor.col = Self::first_non_blank(doc, self.cursor.line);
            }
            Place::LineBelow => {
                // Split at the start of the next line (or append): the new
                // blank line is always `cursor.line + 1`.
                let next = self.cursor.line.saturating_add(1);
                let at = if next < doc.line_count() {
                    doc.char_at_line_col(next, 0)
                } else {
                    doc.len_chars()
                };
                doc.insert(at, "\n");
                self.cursor = Cursor { line: next, col: 0 };
            }
            Place::LineAbove => {
                let at = doc.char_at_line_col(self.cursor.line, 0);
                doc.insert(at, "\n");
                let (line, _) = doc.line_col_at(at);
                self.cursor = Cursor { line, col: 0 };
            }
        }
        self.preferred_col = self.cursor.col;
        self.begin_insert_session(doc);
        self.clamp(doc);
    }

    /// Begin a visual-block insert (`I` left edge, `A` past the right
    /// edge, `c` after deleting): parks on the top row and stages the
    /// replay dims. The caller owns the undo unit.
    pub(super) fn start_block_insert<B: Buffer>(&mut self, doc: &mut B, append: bool) {
        let (top, bottom, left, right) = match self.block_span() {
            Some(span) => span,
            None => return,
        };
        let right = right.saturating_sub(1);
        self.block_pending = Some(BlockPending {
            top,
            bottom,
            left,
            right,
            append,
            skip_first: true,
        });
        let col = if append {
            right.saturating_add(1)
        } else {
            left
        };
        self.cursor = Cursor {
            line: top,
            col: col.min(line_len(doc, top)),
        };
        self.preferred_col = self.cursor.col;
        self.begin_insert_session(doc);
        self.clamp(doc);
    }

    /// Forward character delete (`x` with count), shared by the live key
    /// and `.` replay. One buffer op: already a single undo step.
    pub(super) fn delete_chars_forward<B: Buffer>(&mut self, doc: &mut B, count: usize) {
        let from = self.char_idx(doc, self.cursor);
        let len = line_len(doc, self.cursor.line);
        if self.cursor.col < len {
            let room = len.saturating_sub(self.cursor.col);
            let to = from
                .checked_add(count.max(1))
                .unwrap_or(from)
                .min(from.checked_add(room).unwrap_or(from));
            let cursor = self.delete_range(doc, from, to);
            self.cursor = cursor;
        }
        self.clear_pending();
        self.clamp(doc);
    }

    /// Open find (`/`) or an ex prompt from any mode (IDE Ctrl+F/Ctrl+H):
    /// the engine owns the prompt, the view only asks.
    pub fn begin_search(&mut self) {
        self.mode = Mode::Command;
        self.command.clear();
        self.command.push('/');
        self.clear_pending();
        self.rec_discard();
    }

    /// Open an ex prompt prefilled with `initial` (e.g. `"%s/"`).
    pub fn begin_ex(&mut self, initial: &str) {
        self.mode = Mode::Command;
        self.command.clear();
        self.command.push_str(initial);
        self.clear_pending();
        self.rec_discard();
    }
}
