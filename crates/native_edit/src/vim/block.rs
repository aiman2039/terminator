//! Visual-block (rectangle) yank, delete, paste and insert.
use super::super::doc::Buffer;
use super::mode::{BlockPending, Cursor, Mode};
use super::state::VimEngine;
use super::words::line_len;

pub(super) struct DeleteBlockRangesInput {
    pub(super) top: usize,
    pub(super) bottom: usize,
    pub(super) left: usize,
    pub(super) right_excl: usize,
}
impl VimEngine {
    /// Visual-block rectangle as (top line, bottom line, left column,
    /// right-exclusive column), unclamped: short rows clamp at each use.
    /// `None` outside visual-block mode.
    pub fn block_span(&self) -> Option<(usize, usize, usize, usize)> {
        if self.mode != Mode::VisualBlock {
            return None;
        }
        let top = self.anchor.line.min(self.cursor.line);
        let bottom = self.anchor.line.max(self.cursor.line);
        let left = self.anchor.col.min(self.cursor.col);
        let right = self.anchor.col.max(self.cursor.col);
        Some((top, bottom, left, right.saturating_add(1)))
    }

    /// One entry per row in `[top..=bottom]`: short rows contribute what
    /// they reach (possibly empty), so ragged text never shifts columns.
    pub(super) fn block_rows<B: Buffer>(&self, doc: &B) -> Option<(usize, usize, Vec<String>)> {
        let (top, bottom, left, right) = self.block_span()?;
        let mut out = Vec::with_capacity(bottom.saturating_sub(top).saturating_add(1));
        for line in top..=bottom {
            let len = line_len(doc, line);
            if left >= len {
                out.push(String::new());
            } else {
                let text = doc.line_text(line);
                out.push(
                    text.chars()
                        .skip(left)
                        .take(right.saturating_sub(left))
                        .collect(),
                );
            }
        }
        Some((top, bottom, out))
    }

    /// Rectangle text, rows joined with newlines (for yank and IDE copy).
    pub(super) fn block_text<B: Buffer>(&self, doc: &B) -> Option<String> {
        self.block_rows(doc).map(|(_, _, rows)| rows.join("\n"))
    }

    /// Delete explicit rectangle rows. The caller owns the undo unit, so
    /// plain deletes and change-into-insert share one code path.
    pub(super) fn delete_block_ranges<B: Buffer>(
        &mut self,
        doc: &mut B,
        input: DeleteBlockRangesInput,
    ) {
        let DeleteBlockRangesInput {
            top,
            bottom,
            left,
            right_excl,
        } = input;
        for line in top..=bottom {
            if line >= doc.line_count() {
                break;
            }
            let len = line_len(doc, line);
            if left < len {
                let from = doc.char_at_line_col(line, left);
                let to = doc.char_at_line_col(line, right_excl.min(len));
                if from < to {
                    doc.delete(from..to);
                }
            }
        }
    }

    /// Delete the live block selection; the caller owns the undo unit.
    /// Returns the rest position (top row, left edge clamped).
    pub(super) fn delete_selection_block<B: Buffer>(&mut self, doc: &mut B) -> Cursor {
        let Some((top, bottom, left, right)) = self.block_span() else {
            return self.cursor;
        };
        self.delete_block_ranges(
            doc,
            DeleteBlockRangesInput {
                top,
                bottom,
                left,
                right_excl: right,
            },
        );
        Cursor {
            line: top,
            col: left.min(line_len(doc, top)),
        }
    }

    pub(super) fn yank_block<B: Buffer>(&mut self, doc: &mut B) {
        let Some((_, _, rows)) = self.block_rows(doc) else {
            return;
        };
        let width = self
            .block_span()
            .map_or(0, |(_, _, left, right)| right.saturating_sub(left));
        self.set_register(rows.join("\n"), false, Some(width));
        self.cursor = self.anchor;
    }

    /// Paste a blockwise register: each yanked row inserts at the cursor
    /// column of successive lines, padding short lines with spaces the way
    /// vim does. Extends the buffer with blank lines past EOF.
    pub(super) fn paste_block<B: Buffer>(&mut self, doc: &mut B) {
        let rows: Vec<&str> = self.register.text.split('\n').collect();
        if rows.iter().all(|row| row.is_empty()) {
            return;
        }
        let col = self.cursor.col;
        doc.begin_undo_group();
        for (i, part) in rows.iter().enumerate() {
            let line = self.cursor.line.saturating_add(i);
            while line >= doc.line_count() {
                doc.insert(doc.len_chars(), "\n");
            }
            let len = line_len(doc, line);
            if len >= col {
                doc.insert(doc.char_at_line_col(line, col), part);
            } else {
                let mut padded = " ".repeat(col.saturating_sub(len));
                padded.push_str(part);
                doc.insert(doc.char_at_line_col(line, len), &padded);
            }
        }
        doc.end_undo_group();
        let last = (self
            .cursor
            .line
            .saturating_add(rows.len())
            .saturating_sub(1))
        .min(doc.line_count().saturating_sub(1));
        self.cursor = Cursor {
            line: last,
            col: col.saturating_add(rows.last().map_or(0, |row| row.chars().count())),
        };
        self.enter_normal(doc);
    }

    /// Insert `text` on every row of `block`: `I` at the left edge, `A`
    /// just past the right edge, each clamped to its own row length (short
    /// rows take text at end-of-line, never padded). `skip_first` leaves
    /// the top row alone — it already holds the live session's typing.
    pub(super) fn apply_block_insert<B: Buffer>(
        &mut self,
        doc: &mut B,
        block: &BlockPending,
        text: &str,
    ) {
        if text.is_empty() {
            return;
        }
        let first = if block.skip_first {
            block.top.saturating_add(1)
        } else {
            block.top
        };
        for line in first..=block.bottom {
            if line >= doc.line_count() {
                break;
            }
            let len = line_len(doc, line);
            let col = if block.append {
                block.right.saturating_add(1).min(len)
            } else {
                block.left.min(len)
            };
            doc.insert(doc.char_at_line_col(line, col), text);
        }
    }
}
