//! Visual-block (rectangle) yank, delete, paste and insert.
use super::super::doc::Buffer;
use super::super::rows::{col_at_display_x, display_x};
use super::mode::{BlockPending, Cursor, Mode, Register};
use super::state::VimEngine;
use super::words::line_len;

/// Per-row char ranges `(line, from, to)` of the rectangle covering display
/// cells `left..right` on `lines`: a tab or a wide (CJK) char inside those
/// cells counts once, so ragged and tabbed text never shifts columns.
/// Rows ending before `left` give an empty range at their end; `eol`
/// (`$`) reaches every line end.
fn cell_row_ranges<B: Buffer>(
    doc: &B,
    lines: std::ops::RangeInclusive<usize>,
    (left, right): (usize, usize),
    eol: bool,
) -> Vec<(usize, usize, usize)> {
    let last = doc.line_count().saturating_sub(1);
    let tab = doc.indent_style().tab_cells();
    let mut out = Vec::new();
    for line in *lines.start()..=(*lines.end()).min(last) {
        let text = doc.line_text(line);
        let len = text.chars().count();
        if display_x(&text, len, tab) <= left {
            out.push((line, len, len));
            continue;
        }
        let from = col_at_display_x(&text, left, tab);
        let to = if eol {
            len
        } else {
            col_at_display_x(&text, right.saturating_sub(1), tab)
                .saturating_add(1)
                .min(len)
        };
        out.push((line, from, to.max(from)));
    }
    out
}

/// Delete `rows` (from [`cell_row_ranges`]); the caller owns the undo unit.
fn delete_row_ranges<B: Buffer>(doc: &mut B, rows: &[(usize, usize, usize)]) {
    for &(line, from, to) in rows {
        let start = doc.char_at_line_col(line, from);
        let end = doc.char_at_line_col(line, to);
        if start < end {
            doc.delete(start..end);
        }
    }
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

    /// Per-row char ranges of the live block, by display cells (see
    /// [`cell_row_ranges`]). `$` reaches each line end.
    pub(super) fn block_row_ranges<B: Buffer>(
        &self,
        doc: &B,
    ) -> Option<Vec<(usize, usize, usize)>> {
        let (top, bottom, _, _) = self.block_span()?;
        let tab = doc.indent_style().tab_cells();
        let cells = |c: Cursor| {
            let text = doc.line_text(c.line);
            (
                display_x(&text, c.col, tab),
                display_x(&text, c.col.saturating_add(1), tab),
            )
        };
        let (a0, a1) = cells(self.anchor);
        let (c0, c1) = cells(self.cursor);
        let cells = (a0.min(c0), a1.max(c1));
        Some(cell_row_ranges(doc, top..=bottom, cells, self.x.block_eol))
    }

    /// One entry per row in `[top..=bottom]`: short rows contribute what
    /// they reach (possibly empty).
    pub(super) fn block_rows<B: Buffer>(&self, doc: &B) -> Option<Vec<String>> {
        let rows = self
            .block_row_ranges(doc)?
            .into_iter()
            .map(|(line, from, to)| {
                doc.line_text(line)
                    .chars()
                    .skip(from)
                    .take(to.saturating_sub(from))
                    .collect()
            })
            .collect();
        Some(rows)
    }

    /// Rectangle text, rows joined with newlines (for yank and IDE copy).
    pub(super) fn block_text<B: Buffer>(&self, doc: &B) -> Option<String> {
        self.block_rows(doc).map(|rows| rows.join("\n"))
    }

    /// The live block as register text and its column width.
    pub(super) fn block_register<B: Buffer>(&self, doc: &B) -> Option<(String, usize)> {
        let (_, _, left, right) = self.block_span()?;
        Some((self.block_text(doc)?, right.saturating_sub(left)))
    }

    /// Delete the live block selection; the caller owns the undo unit.
    /// Returns the rest position (top row, left edge clamped).
    pub(super) fn delete_selection_block<B: Buffer>(&mut self, doc: &mut B) -> Cursor {
        let Some(rows) = self.block_row_ranges(doc) else {
            return self.cursor;
        };
        let mut rest = rows
            .first()
            .map_or(self.cursor, |&(line, from, _)| Cursor { line, col: from });
        delete_row_ranges(doc, &rows);
        rest.col = rest.col.min(line_len(doc, rest.line));
        rest
    }

    /// Delete a replayed rectangle (`.`): columns `left..=right` on rows
    /// `top..=bottom`, the same per-row ranges as a live block delete.
    pub(super) fn delete_block_rect<B: Buffer>(&mut self, doc: &mut B, block: &BlockPending) {
        let cells = (block.left, block.right.saturating_add(1));
        let rows = cell_row_ranges(doc, block.top..=block.bottom, cells, false);
        delete_row_ranges(doc, &rows);
    }

    pub(super) fn yank_block<B: Buffer>(&mut self, doc: &mut B) {
        let Some((text, width)) = self.block_register(doc) else {
            return;
        };
        self.write_yank(text, false, Some(width.max(1)));
        self.cursor = self.anchor;
    }

    /// Paste a blockwise register: each yanked row (`count` times)
    /// inserts after the cursor column (`P`: at it) of successive lines,
    /// padding short lines with spaces the way vim does. Extends the
    /// buffer with blank lines past EOF.
    pub(super) fn paste_block<B: Buffer>(
        &mut self,
        doc: &mut B,
        reg: &Register,
        (before, count): (bool, usize),
    ) {
        let rows: Vec<&str> = reg.text.split('\n').collect();
        if rows.iter().all(|row| row.is_empty()) {
            return;
        }
        let width = reg.block_width.unwrap_or(0);
        let here = line_len(doc, self.cursor.line);
        let col = if before || here == 0 {
            self.cursor.col
        } else {
            self.cursor.col.saturating_add(1).min(here)
        };
        doc.begin_undo_group();
        for (i, part) in rows.iter().enumerate() {
            let line = self.cursor.line.saturating_add(i);
            while line >= doc.line_count() {
                doc.insert(doc.len_chars(), "\n");
            }
            let len = line_len(doc, line);
            // Narrow rows pad to the block width when text follows.
            let mut piece = String::new();
            for _ in 0..count.max(1) {
                piece.push_str(part);
                if len > col {
                    let pad = width.saturating_sub(part.chars().count());
                    piece.extend(std::iter::repeat_n(' ', pad));
                }
            }
            if len >= col {
                doc.insert(doc.char_at_line_col(line, col), &piece);
            } else {
                let mut padded = " ".repeat(col.saturating_sub(len));
                padded.push_str(&piece);
                doc.insert(doc.char_at_line_col(line, len), &padded);
            }
        }
        doc.end_undo_group();
        self.cursor = Cursor {
            line: self.cursor.line,
            col,
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
            // Vim parity: `I` skips rows that end before the block.
            if !block.append && len < block.left {
                continue;
            }
            let col = if block.append {
                block.right.saturating_add(1).min(len)
            } else {
                block.left.min(len)
            };
            doc.insert(doc.char_at_line_col(line, col), text);
        }
    }
}
