use super::super::doc::Buffer;
use super::mode::{BlockPending, Cursor, Mode, Place};
use super::state::VimEngine;
use super::words::line_len;
impl VimEngine {
    /// Drive an operator motion the way the live keys do (including the
    /// `cw`→`ce` special case), for `.` replay.
    pub(super) fn replay_motion<B: Buffer>(&mut self, doc: &mut B, motion: char, count: usize) {
        self.apply_motion(doc, motion, count);
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
        // `$A` appends at every line end.
        let right = if self.x.block_eol {
            usize::MAX.saturating_sub(1)
        } else {
            right.saturating_sub(1)
        };
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
