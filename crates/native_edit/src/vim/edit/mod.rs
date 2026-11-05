//! Operators over motion and text-object spans, charwise or linewise.
mod case;
mod commands;
pub(crate) use commands::{repeat_more, repeat_text};
mod join;
mod number;
mod replace;
mod visual_ops;

use super::super::doc::Buffer;
use super::helpers::{grapheme_right, range_text};
use super::mode::{Cursor, PendingOp};
use super::motion::{MKind, Mo, Motion};
use super::state::VimEngine;
use super::words::line_len;

impl VimEngine {
    /// Apply `op` from the cursor to a resolved motion, with vim's
    /// exclusive-motion adjustments (`:help exclusive`).
    pub(crate) fn operate_motion<B: Buffer>(
        &mut self,
        doc: &mut B,
        op: PendingOp,
        (mo, motion): (Mo, Motion),
    ) {
        let start = self.cursor;
        let (a, mut b) = if (start.line, start.col) <= (motion.target.line, motion.target.col) {
            (start, motion.target)
        } else {
            (motion.target, start)
        };
        match motion.kind {
            MKind::Linewise => self.operate_lines(doc, op, (a.line, b.line)),
            MKind::Inclusive => {
                let from = self.char_idx(doc, a);
                let to = self.inclusive_end(doc, b);
                self.operate_chars(doc, op, (from, to));
            }
            MKind::Exclusive => {
                if b.line > a.line && (matches!(mo, Mo::Word { .. }) || b.col == 0) {
                    let prev = b.line.saturating_sub(1);
                    // `dw` on a line's last word stops at the line end; an
                    // end in column 0 from the line start goes linewise.
                    if !matches!(mo, Mo::Word { .. })
                        && a.col <= Self::first_non_blank(doc, a.line)
                        && op != PendingOp::Yank
                    {
                        self.operate_lines(doc, op, (a.line, prev));
                        return;
                    }
                    b = Cursor {
                        line: prev,
                        col: line_len(doc, prev),
                    };
                }
                let from = self.char_idx(doc, a);
                let to = self.char_idx(doc, b);
                self.operate_chars(doc, op, (from, to.max(from)));
            }
        }
    }

    /// Exclusive end of an inclusive span ending at `b`: past its
    /// grapheme, or nothing on an empty line (`d$` keeps the newline).
    fn inclusive_end<B: Buffer>(&self, doc: &B, b: Cursor) -> usize {
        let text = doc.line_text(b.line);
        let len = text.chars().count();
        if b.col >= len {
            return self.char_idx(doc, b);
        }
        let end = grapheme_right(&text, b.col, 1);
        doc.char_at_line_col(b.line, 0).saturating_add(end)
    }

    /// Apply `op` to chars `from..to`.
    pub(crate) fn operate_chars<B: Buffer>(
        &mut self,
        doc: &mut B,
        op: PendingOp,
        (from, to): (usize, usize),
    ) {
        let text = range_text(doc, from, to);
        match op {
            PendingOp::Delete | PendingOp::Change => {
                if from < to {
                    self.write_delete(text, false);
                }
                if op == PendingOp::Change {
                    self.begin_change_group(doc);
                }
                self.cursor = self.delete_range(doc, from, to);
                if op == PendingOp::Change {
                    self.begin_insert_session(doc);
                    self.clamp(doc);
                } else {
                    self.enter_normal(doc);
                    self.rec_commit();
                }
            }
            PendingOp::Yank => {
                self.write_yank(text, false, None);
                self.cursor = Self::cursor_at(doc, from);
                self.enter_normal(doc);
                // Yank mutates nothing: the last change survives.
                self.rec_discard();
            }
            PendingOp::Lower | PendingOp::Upper | PendingOp::Toggle => {
                self.replace_text(doc, (from, to), &case::convert(&text, op));
                self.cursor = Self::cursor_at(doc, from);
                self.enter_normal(doc);
                self.rec_commit();
            }
            PendingOp::Indent | PendingOp::Outdent => {
                let first = Self::cursor_at(doc, from).line;
                let last = Self::cursor_at(doc, to.saturating_sub(1).max(from)).line;
                self.operate_lines(doc, op, (first, last));
            }
        }
    }

    /// Replace chars `from..to` with `text` (one undo unit).
    pub(crate) fn replace_text<B: Buffer>(
        &mut self,
        doc: &mut B,
        (from, to): (usize, usize),
        text: &str,
    ) {
        if range_text(doc, from, to) == text {
            return;
        }
        doc.begin_undo_group();
        doc.delete(from..to);
        doc.insert(from, text);
        doc.end_undo_group();
        doc.end_run();
    }

    /// Apply `op` to whole lines `first..=last` (clamped to real lines).
    pub(crate) fn operate_lines<B: Buffer>(
        &mut self,
        doc: &mut B,
        op: PendingOp,
        (first, last): (usize, usize),
    ) {
        let max = Self::last_addressable(doc);
        let first = first.min(max);
        let last = last.clamp(first, max);
        match op {
            PendingOp::Delete => {
                self.delete_line_range(doc, first, last);
                self.enter_normal(doc);
                self.rec_commit();
            }
            PendingOp::Yank => {
                let text = Self::lines_text(doc, first, last);
                self.write_yank(text, true, None);
                if self.cursor.line != first {
                    self.cursor.line = first;
                }
                self.enter_normal(doc);
                self.rec_discard();
            }
            PendingOp::Change => {
                self.begin_change_group(doc);
                self.change_line_range(doc, first, last);
            }
            PendingOp::Indent | PendingOp::Outdent => {
                self.shift_lines(doc, (first, last), op == PendingOp::Outdent);
                self.cursor = Cursor {
                    line: first,
                    col: Self::first_non_blank(doc, first),
                };
                self.enter_normal(doc);
                self.rec_commit();
            }
            PendingOp::Lower | PendingOp::Upper | PendingOp::Toggle => {
                let from = doc.char_at_line_col(first, 0);
                let to = doc.char_at_line_col(last, line_len(doc, last));
                let text = range_text(doc, from, to);
                self.replace_text(doc, (from, to), &case::convert(&text, op));
                self.cursor = Cursor {
                    line: first,
                    col: 0,
                };
                self.enter_normal(doc);
                self.rec_commit();
            }
        }
    }

    /// Text of lines `first..=last`, joined without a final newline.
    pub(crate) fn lines_text<B: Buffer>(doc: &B, first: usize, last: usize) -> String {
        (first..=last)
            .map(|line| doc.line_text(line))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Delete lines `first..=last` into the register (linewise); the
    /// cursor lands on the first non-blank of the line that follows.
    pub(crate) fn delete_line_range<B: Buffer>(&mut self, doc: &mut B, first: usize, last: usize) {
        let lines = doc.line_count();
        if lines == 0 {
            return;
        }
        self.write_delete(Self::lines_text(doc, first, last), true);
        let from = doc.char_at_line_col(first, 0);
        let next = last.saturating_add(1);
        let to = if next < lines {
            doc.char_at_line_col(next, 0)
        } else {
            doc.len_chars()
        };
        doc.delete(from..to);
        doc.end_run();
        self.cursor.line = first.min(Self::last_addressable(doc));
        self.cursor.col = Self::first_non_blank(doc, self.cursor.line);
    }

    /// `cc` / `S` / linewise `c`: empty lines `first..=last` into one line
    /// that keeps the first line's indent, then insert there.
    pub(crate) fn change_line_range<B: Buffer>(&mut self, doc: &mut B, first: usize, last: usize) {
        self.write_delete(Self::lines_text(doc, first, last), true);
        let indent = Self::first_non_blank(doc, first);
        let from = doc.char_at_line_col(first, indent);
        let to = doc.char_at_line_col(last, line_len(doc, last));
        doc.delete(from..to.max(from));
        doc.end_run();
        self.cursor = Cursor {
            line: first,
            col: indent,
        };
        self.begin_insert_session(doc);
        self.clamp(doc);
    }

    /// Shift lines right (or left) by one level of the document's indent
    /// style; blank lines stay empty when shifting right, like vim.
    pub(crate) fn shift_lines<B: Buffer>(
        &mut self,
        doc: &mut B,
        (first, last): (usize, usize),
        left: bool,
    ) {
        let style = doc.indent_style();
        let unit = style.unit();
        doc.begin_undo_group();
        for line in first..=last {
            let text = doc.line_text(line);
            let at = doc.char_at_line_col(line, 0);
            if left {
                let take = style.outdent_width(&text);
                doc.delete(at..at.saturating_add(take));
            } else if !text.is_empty() {
                doc.insert(at, &unit);
            }
        }
        doc.end_undo_group();
        doc.end_run();
    }
}
