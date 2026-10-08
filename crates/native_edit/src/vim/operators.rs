use super::super::doc::Buffer;
use super::helpers::{doc_is_empty, range_text};
use super::mode::{Bit, BlockPending, Cursor, Effect, ModalEngine, Mode, PendingOp, Place, Repeat};
use super::state::VimEngine;
use super::words::line_len;
impl VimEngine {
    pub(super) fn delete_range<B: Buffer>(
        &mut self,
        doc: &mut B,
        from: usize,
        to: usize,
    ) -> Cursor {
        let (from, to) = (from.min(to), from.max(to));
        // A charwise delete invalidates any yanked rectangle: the kept
        // text (if any) pastes sequentially from here on.
        self.register.block_width = None;
        doc.delete(from..to);
        doc.end_run();
        let (line, col) = doc.line_col_at(from.min(doc.len_chars()));
        Cursor { line, col }
    }

    /// Whole-buffer visual selection (IDE select-all, Cmd/Ctrl+A).
    /// An IDE action, not a vim change: the last repeatable change keeps.
    pub fn select_all<B: Buffer>(&mut self, doc: &B) {
        self.rec_discard();
        let last = doc.line_count().saturating_sub(1);
        self.anchor = Cursor { line: 0, col: 0 };
        self.cursor = Cursor {
            line: last,
            col: line_len(doc, last),
        };
        self.preferred_col = self.cursor.col;
        self.mode = Mode::Visual;
        self.clear_pending();
    }

    /// IDE copy source: the selection, else the cursor line. Pure, so the
    /// view can copy without disturbing mode, cursor, or undo.
    pub fn copy_text<B: Buffer>(&self, doc: &B) -> String {
        if self.mode == Mode::VisualBlock {
            return self.block_text(doc).unwrap_or_default();
        }
        if self.selection().is_some() {
            let linewise = self.mode == Mode::VisualLine;
            let (from, to) = self.selection_range(doc, linewise);
            return range_text(doc, from, to);
        }
        let line = self.cursor.line.min(doc.line_count().saturating_sub(1));
        doc.line_text(line)
    }

    /// IDE cut: remove the copy source and park the cursor on the cut point.
    /// A cut selection collapses back to Normal, mirroring `d`. An IDE
    /// action, not a vim change: recording continues only inside an insert
    /// session, where the cut ends typed-text attribution.
    pub fn cut<B: Buffer>(&mut self, doc: &mut B) -> String {
        if self.mode == Mode::Insert {
            self.typed_exact = false;
        } else {
            self.rec_discard();
        }
        // A stale blockwise flag must not survive an IDE cut: the next `p`
        // would otherwise paste a rectangle from unrelated text.
        self.register.block_width = None;
        if self.mode == Mode::VisualBlock {
            let text = self.block_text(doc).unwrap_or_default();
            doc.begin_undo_group();
            let cursor = self.delete_selection_block(doc);
            doc.end_undo_group();
            self.cursor = cursor;
            self.preferred_col = cursor.col;
            self.enter_normal(doc);
            return text;
        }
        let linewise = self.mode == Mode::VisualLine;
        let (from, to) = if self.selection().is_some() {
            self.selection_range(doc, linewise)
        } else {
            let line = self.cursor.line.min(doc.line_count().saturating_sub(1));
            let from = doc.char_at_line_col(line, 0);
            let to = if line
                .checked_add(1)
                .is_some_and(|next| next < doc.line_count())
            {
                doc.char_at_line_col(line.checked_add(1).unwrap_or(line), 0)
            } else {
                doc.len_chars()
            };
            (from, to)
        };
        let text = range_text(doc, from, to);
        let cursor = self.delete_range(doc, from, to);
        self.cursor = cursor;
        self.preferred_col = cursor.col;
        if linewise || matches!(self.mode, Mode::Visual) {
            self.enter_normal(doc);
        } else {
            self.clear_pending();
            self.clamp(doc);
        }
        text
    }

    /// IDE paste: insert plain text at the cursor in any mode (Cmd/Ctrl+V).
    /// One undo unit; mode and selection are left alone. In Normal mode
    /// the paste becomes the repeatable change (insert at cursor); inside
    /// an insert session its text joins the session recording exactly when
    /// it lands at the session end.
    pub fn paste_text<B: Buffer>(&mut self, doc: &mut B, text: &str) {
        if text.is_empty() {
            return;
        }
        self.clear_pending();
        let at = self.char_idx(doc, self.cursor).min(doc.len_chars());
        if !self.replaying {
            if self.mode == Mode::Insert {
                self.recording.push(Bit::Type(text.to_string()));
                if at == self.typed_end {
                    self.typed_end = self.typed_end.saturating_add(text.chars().count());
                } else {
                    self.typed_exact = false;
                }
            } else if self.mode == Mode::Normal {
                self.last_change = Some(Repeat::Insert {
                    text: text.to_string(),
                    place: Place::AtCursor,
                });
                self.recording.clear();
            } else {
                self.rec_discard();
            }
        }
        doc.insert(at, text);
        doc.end_run();
        let end = at
            .checked_add(text.chars().count().min(doc.len_chars().saturating_sub(at)))
            .unwrap_or(at);
        let (line, col) = doc.line_col_at(end);
        self.cursor = Cursor { line, col };
        self.settle_cursor(doc);
        self.clamp(doc);
        self.preferred_col = self.cursor.col;
    }

    /// Consume a pending cursor-reveal request (search jumps, `G`/`gg`).
    /// The view scrolls the cursor row into sight when this returns true.
    pub fn take_scroll_request(&mut self) -> bool {
        std::mem::replace(&mut self.scroll_request, false)
    }

    /// All match starts (char indices) of a literal pattern, in buffer
    /// order. Literal and case-sensitive (no vim regex); single-line
    /// patterns only, so a pattern containing `\n` never matches.
    pub(super) fn search_matches<B: Buffer>(doc: &B, pattern: &str) -> Vec<usize> {
        if pattern.is_empty() || pattern.contains('\n') {
            return Vec::new();
        }
        let mut out = Vec::new();
        for line in 0..doc.line_count() {
            let text = doc.line_text(line);
            for (byte, _) in text.match_indices(pattern) {
                let col = text.get(..byte).map_or(0, |prefix| prefix.chars().count());
                out.push(doc.char_at_line_col(line, col));
            }
        }
        out
    }

    /// Jump to the next (`forward`) or previous match from `from`, wrapping
    /// around the buffer. Forward is inclusive of `from` (landing on the
    /// match under the cursor); backward is exclusive (so `N` steps back).
    /// Returns false — and leaves the cursor alone — when nothing matches.
    pub(super) fn jump_to_match<B: Buffer>(
        &mut self,
        doc: &mut B,
        pattern: &str,
        from: usize,
        forward: bool,
    ) -> bool {
        let matches = Self::search_matches(doc, pattern);
        if matches.is_empty() {
            return false;
        }
        let next = if forward {
            matches
                .iter()
                .find(|m| **m >= from)
                .or(matches.first())
                .copied()
        } else {
            matches
                .iter()
                .rev()
                .find(|m| **m < from)
                .or(matches.last())
                .copied()
        };
        let Some(at) = next else {
            return false;
        };
        let (line, col) = doc.line_col_at(at);
        self.cursor = Cursor { line, col };
        self.preferred_col = col;
        self.clamp(doc);
        self.scroll_request = true;
        true
    }

    /// IDE undo/redo: same as `u`/Ctrl-R but callable in any mode, with the
    /// cursor clamped back into range afterwards. Undo is never repeatable.
    pub fn undo<B: Buffer>(&mut self, doc: &mut B) {
        doc.undo();
        self.clear_pending();
        self.rec_discard();
        self.clamp(doc);
    }

    /// See [`Self::undo`].
    pub fn redo<B: Buffer>(&mut self, doc: &mut B) {
        doc.redo();
        self.clear_pending();
        self.rec_discard();
        self.clamp(doc);
    }

    pub(super) fn apply_operator<B: Buffer>(
        &mut self,
        doc: &mut B,
        op: PendingOp,
        target: Cursor,
        inclusive: bool,
    ) -> Vec<Effect> {
        let from = self.char_idx(doc, self.cursor);
        let mut to = self.char_idx(doc, target);
        if inclusive {
            to = to
                .checked_add(1)
                .map_or(doc.len_chars(), |next| next.min(doc.len_chars()));
        }
        let (from, to) = (from.min(to), from.max(to));
        match op {
            PendingOp::Delete => {
                let cursor = self.delete_range(doc, from, to);
                self.cursor = cursor;
                self.enter_normal(doc);
                self.rec_commit();
            }
            PendingOp::Change => {
                self.begin_change_group(doc);
                let cursor = self.delete_range(doc, from, to);
                self.cursor = cursor;
                self.begin_insert_session(doc);
                self.clamp(doc);
            }
            PendingOp::Yank => {
                self.set_register(range_text(doc, from, to), false, None);
                self.cursor = target;
                self.enter_normal(doc);
                // Yank mutates nothing: the last change survives.
                self.rec_discard();
            }
        }
        Vec::new()
    }

    pub(super) fn delete_lines<B: Buffer>(&mut self, doc: &mut B, count: usize) {
        let lines = doc.line_count();
        let start = self.cursor.line.min(lines.saturating_sub(1));
        let end = start.checked_add(count).map_or(lines, |end| end.min(lines));
        if lines == 0 {
            return;
        }
        let from = doc.char_at_line_col(start, 0);
        let to = if end < lines {
            doc.char_at_line_col(end, 0)
        } else {
            doc.len_chars()
        };
        let mut yanked = range_text(doc, from, to);
        if end < lines && yanked.ends_with('\n') {
            yanked.pop();
        }
        self.set_register(yanked, true, None);
        doc.delete(from..to);
        doc.end_run();
        self.cursor.line = start.min(doc.line_count().saturating_sub(1));
        self.cursor.col = Self::first_non_blank(doc, self.cursor.line);
        self.enter_normal(doc);
    }

    pub(super) fn yank_lines<B: Buffer>(&mut self, doc: &mut B, count: usize) {
        let lines = doc.line_count();
        let start = self.cursor.line.min(lines.saturating_sub(1));
        let end = start.checked_add(count).map_or(lines, |end| end.min(lines));
        let mut yanked = Vec::new();
        for line in start..end {
            yanked.push(doc.line_text(line));
        }
        self.set_register(yanked.join("\n"), true, None);
        self.enter_normal(doc);
    }

    pub(super) fn paste<B: Buffer>(&mut self, doc: &mut B) {
        if self.register.text.is_empty() {
            return;
        }
        if !self.register.linewise && self.register.block_width.is_some() {
            self.paste_block(doc);
            return;
        }
        if self.register.linewise {
            let line = self
                .cursor
                .line
                .checked_add(1)
                .map_or(doc.line_count(), |line| line.min(doc.line_count()));
            let at = if line < doc.line_count() {
                doc.char_at_line_col(line, 0)
            } else {
                doc.len_chars()
            };
            let prefix = if line == doc.line_count() && !doc_is_empty(doc) {
                "\n"
            } else {
                ""
            };
            let text = format!("{}{}\n", prefix, self.register.text);
            doc.insert(at, &text);
            doc.end_run();
            self.cursor.line = line.min(doc.line_count().saturating_sub(1));
            self.cursor.col = Self::first_non_blank(doc, self.cursor.line);
        } else {
            let at = self
                .char_idx(doc, self.cursor)
                .checked_add(1)
                .map_or(doc.len_chars(), |at| at.min(doc.len_chars()));
            doc.insert(at, &self.register.text);
            doc.end_run();
            let landed = at
                .checked_add(self.register.text.chars().count())
                .and_then(|sum| sum.checked_sub(1))
                .unwrap_or(at);
            let (line, col) = doc.line_col_at(landed);
            self.cursor = Cursor { line, col };
        }
        self.enter_normal(doc);
    }

    pub(super) fn change_line<B: Buffer>(&mut self, doc: &mut B, count: usize) {
        let lines = doc.line_count();
        let start = self.cursor.line.min(lines.saturating_sub(1));
        let end = start.checked_add(count).map_or(lines, |end| end.min(lines));
        let from = doc.char_at_line_col(start, 0);
        // Full lines including their newlines; deleting every line leaves the
        // single empty final line, so structure is always preserved.
        let to = if end < lines {
            doc.char_at_line_col(end, 0)
        } else {
            doc.len_chars()
        };
        doc.delete(from..to);
        doc.end_run();
        self.cursor = Cursor {
            line: start,
            col: 0,
        };
        self.begin_insert_session(doc);
        self.clamp(doc);
    }

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
        top: usize,
        bottom: usize,
        left: usize,
        right_excl: usize,
    ) {
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
        self.delete_block_ranges(doc, top, bottom, left, right);
        Cursor {
            line: top,
            col: left.min(line_len(doc, top)),
        }
    }

    /// Write the unnamed register and stage the text for host clipboard
    /// sync. All register writes funnel through here — never assign the
    /// fields directly (except invalidation, which writes nothing).
    pub(super) fn set_register(
        &mut self,
        text: String,
        linewise: bool,
        block_width: Option<usize>,
    ) {
        if !text.is_empty() {
            self.clipboard = Some(text.clone());
        }
        self.register.text = text;
        self.register.linewise = linewise;
        self.register.block_width = block_width;
    }

    /// Take staged clipboard text, if a yank or register write happened
    /// since the last take. Pure host plumbing.
    #[must_use]
    pub fn take_clipboard(&mut self) -> Option<String> {
        self.clipboard.take()
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
