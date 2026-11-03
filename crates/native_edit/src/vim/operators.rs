use super::super::doc::Buffer;
use super::helpers::{doc_is_empty, range_text};
use super::mode::{Bit, Cursor, Effect, ModalEngine, Mode, PendingOp, Place, Repeat};
use super::state::VimEngine;
use super::words::line_len;

/// Byte offsets and lengths of every `pattern` match in `text`. ASCII
/// case folding keeps the offsets valid for the original string, which
/// is all `.vimrc` `ignorecase` needs to promise.
pub(super) fn find_matches(text: &str, pattern: &str, ignorecase: bool) -> Vec<(usize, usize)> {
    if ignorecase {
        let folded = text.to_ascii_lowercase();
        folded
            .match_indices(&pattern.to_ascii_lowercase())
            .map(|(byte, matched)| (byte, matched.len()))
            .collect()
    } else {
        text.match_indices(pattern)
            .map(|(byte, matched)| (byte, matched.len()))
            .collect()
    }
}

pub(super) struct JumpToMatchInput<'a> {
    pub(super) pattern: &'a str,
    pub(super) from: usize,
    pub(super) forward: bool,
}

pub(super) struct ApplyOperatorInput {
    pub(super) op: PendingOp,
    pub(super) target: Cursor,
    pub(super) inclusive: bool,
}

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
    /// it lands at the session end. Pasted `\r\n` and `\r` become `\n`, so
    /// a paste never adds stray carriage returns to the buffer.
    pub fn paste_text<B: Buffer>(&mut self, doc: &mut B, text: &str) {
        let normalized = super::helpers::normalize_newlines(text);
        let text = normalized.as_ref();
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
    /// order. Literal (no vim regex) and case-sensitive unless
    /// `ignorecase` is set; single-line patterns only, so a pattern
    /// containing `\n` never matches.
    pub(super) fn search_matches<B: Buffer>(
        doc: &B,
        pattern: &str,
        ignorecase: bool,
    ) -> Vec<usize> {
        if pattern.is_empty() || pattern.contains('\n') {
            return Vec::new();
        }
        let mut out = Vec::new();
        for line in 0..doc.line_count() {
            let text = doc.line_text(line);
            for (byte, _) in find_matches(&text, pattern, ignorecase) {
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
        input: JumpToMatchInput<'_>,
    ) -> bool {
        let JumpToMatchInput {
            pattern,
            from,
            forward,
        } = input;
        let matches = Self::search_matches(doc, pattern, self.ignorecase());
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
        input: ApplyOperatorInput,
    ) -> Vec<Effect> {
        let ApplyOperatorInput {
            op,
            target,
            inclusive,
        } = input;
        let from = self.char_idx(doc, self.cursor);
        let mut to = self.char_idx(doc, target);
        if inclusive {
            to = to
                .checked_add(1)
                .map_or(doc.len_chars(), |next| next.min(doc.len_chars()));
        }
        let (from, to) = (from.min(to), from.max(to));
        // Vim parity: `d` and `c` fill the unnamed register, so `dwp` works.
        if from < to && op != PendingOp::Yank {
            self.set_register_local(range_text(doc, from, to));
        }
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

    /// `p` (`before == false`) or `P`, `count` copies in one insert:
    /// linewise below or above the cursor line, charwise after or at the
    /// cursor.
    pub(super) fn put<B: Buffer>(&mut self, doc: &mut B, before: bool, count: usize) {
        if self.register.text.is_empty() {
            return;
        }
        let count = count.max(1);
        if !self.register.linewise && self.register.block_width.is_some() {
            for _ in 0..count {
                self.paste_block(doc);
            }
            return;
        }
        if self.register.linewise {
            let lines = doc.line_count();
            let line = if before {
                self.cursor.line.min(lines.saturating_sub(1))
            } else {
                self.cursor
                    .line
                    .checked_add(1)
                    .map_or(lines, |line| line.min(lines))
            };
            let at = if line < lines {
                doc.char_at_line_col(line, 0)
            } else {
                doc.len_chars()
            };
            let prefix = if line == lines && !doc_is_empty(doc) {
                "\n"
            } else {
                ""
            };
            let body = format!("{}\n", self.register.text).repeat(count);
            doc.insert(at, &format!("{prefix}{body}"));
            doc.end_run();
            self.cursor.line = line.min(doc.line_count().saturating_sub(1));
            self.cursor.col = Self::first_non_blank(doc, self.cursor.line);
        } else {
            let cursor_at = self.char_idx(doc, self.cursor);
            let at = if before {
                cursor_at
            } else {
                cursor_at.saturating_add(1)
            }
            .min(doc.len_chars());
            let text = self.register.text.repeat(count);
            doc.insert(at, &text);
            doc.end_run();
            self.land_after_charwise_paste(doc, at, &text);
        }
        self.enter_normal(doc);
    }

    /// Vim parity: after a charwise put the cursor is on the last pasted
    /// char, or on the first one when the text spans lines.
    fn land_after_charwise_paste<B: Buffer>(&mut self, doc: &B, at: usize, text: &str) {
        let landed = if text.contains('\n') {
            at
        } else {
            at.saturating_add(text.chars().count())
                .saturating_sub(1)
                .max(at)
        };
        let (line, col) = doc.line_col_at(landed);
        self.cursor = Cursor { line, col };
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
}
