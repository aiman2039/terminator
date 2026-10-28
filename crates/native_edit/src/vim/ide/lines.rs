//! IDE line operations: duplicate, delete, move, comment, join, and
//! line/word selection helpers.
use super::super::super::doc::Buffer;
use super::super::helpers::range_text;
use super::super::mode::{Cursor, Mode};
use super::super::state::VimEngine;
use super::super::words::line_len;
use super::IdeMotion;
use super::text::IdeText;

impl VimEngine {
    /// Char index of the end of `line` (before its newline).
    pub(super) fn line_end_idx<B: Buffer>(doc: &B, line: usize) -> usize {
        doc.char_at_line_col(line, line_len(doc, line))
    }

    /// Duplicate the selection (selecting the copy), or the selected or
    /// caret lines below themselves (`IntelliJ` Cmd/Ctrl+D).
    pub fn ide_duplicate<B: Buffer>(&mut self, doc: &mut B) {
        self.ide_edit_begin(doc);
        if self.modeless
            && self.mode == Mode::Visual
            && let Some((start, end)) = self.selection_bounds()
        {
            let (from, to) = (self.char_idx(doc, start), self.char_idx(doc, end));
            let text = range_text(doc, from, to);
            doc.insert(to, &text);
            let (line, col) = doc.line_col_at(to);
            self.anchor = Cursor { line, col };
            let (line, col) = doc.line_col_at(to.saturating_add(text.chars().count()));
            self.cursor = Cursor { line, col };
        } else {
            let (top, bottom) = self.ide_line_span();
            let block: Vec<String> = (top..=bottom).map(|line| doc.line_text(line)).collect();
            let shift = block.len();
            doc.insert(
                Self::line_end_idx(doc, bottom),
                &format!("\n{}", block.join("\n")),
            );
            self.cursor.line = self.cursor.line.saturating_add(shift);
            self.anchor.line = self.anchor.line.saturating_add(shift);
        }
        self.ide_edit_end(doc);
    }

    /// Delete the selected or caret lines, newline included.
    pub fn ide_delete_lines<B: Buffer>(&mut self, doc: &mut B) {
        let (top, bottom) = self.ide_line_span();
        self.ide_edit_begin(doc);
        let last = doc.line_count().saturating_sub(1);
        let (from, to) = if bottom < last {
            (
                doc.char_at_line_col(top, 0),
                doc.char_at_line_col(bottom.saturating_add(1), 0),
            )
        } else if top > 0 {
            (
                Self::line_end_idx(doc, top.saturating_sub(1)),
                doc.len_chars(),
            )
        } else {
            (0, doc.len_chars())
        };
        doc.delete(from..to);
        if self.mode.is_visual() {
            self.mode = Mode::Normal;
        }
        let line = top.min(doc.line_count().saturating_sub(1));
        self.cursor = Cursor {
            line,
            col: self.preferred_col.min(line_len(doc, line)),
        };
        self.anchor = self.cursor;
        self.ide_edit_end(doc);
    }

    /// Move the selected or caret lines one line up or down, keeping the
    /// caret and selection on them.
    pub fn ide_move_lines<B: Buffer>(&mut self, doc: &mut B, up: bool) {
        let (top, bottom) = self.ide_line_span();
        let last = Self::last_addressable(doc);
        if (up && top == 0) || (!up && bottom >= last) {
            return;
        }
        self.ide_edit_begin(doc);
        let block: Vec<String> = (top..=bottom).map(|line| doc.line_text(line)).collect();
        let block = block.join("\n");
        let (from_line, to_line, text) = if up {
            let above = top.saturating_sub(1);
            (above, bottom, format!("{block}\n{}", doc.line_text(above)))
        } else {
            let below = bottom.saturating_add(1);
            (top, below, format!("{}\n{block}", doc.line_text(below)))
        };
        let from = doc.char_at_line_col(from_line, 0);
        let to = Self::line_end_idx(doc, to_line);
        doc.delete(from..to);
        doc.insert(from, &text);
        for end in [&mut self.cursor, &mut self.anchor] {
            end.line = if up {
                end.line.saturating_sub(1)
            } else {
                end.line.saturating_add(1)
            };
        }
        self.ide_edit_end(doc);
    }

    /// Toggle a line comment on the selected or caret lines. All non-blank
    /// lines commented: uncomment; otherwise comment at their common
    /// indentation. Without a selection the caret moves down a line.
    pub fn ide_toggle_comment<B: Buffer>(&mut self, doc: &mut B, token: &str) {
        if token.is_empty() {
            return;
        }
        let (top, bottom) = self.ide_line_span();
        let lines: Vec<(usize, String)> = (top..=bottom)
            .map(|line| (line, doc.line_text(line)))
            .filter(|(_, text)| !text.trim().is_empty())
            .collect();
        if lines.is_empty() {
            return;
        }
        let commented = lines
            .iter()
            .all(|(_, text)| text.trim_start().starts_with(token));
        let indent = lines
            .iter()
            .map(|(_, text)| IdeText::leading_whitespace(text).chars().count())
            .min()
            .unwrap_or(0);
        self.ide_edit_begin(doc);
        for (line, text) in lines.iter().rev() {
            let lead = IdeText::leading_whitespace(text).chars().count();
            if commented {
                let rest: String = text.chars().skip(lead).collect();
                let mut width = token.chars().count();
                if rest.chars().nth(width) == Some(' ') {
                    width = width.saturating_add(1);
                }
                let at = doc.char_at_line_col(*line, lead);
                doc.delete(at..at.saturating_add(width));
                for end in [&mut self.cursor, &mut self.anchor] {
                    if end.line == *line && end.col > lead {
                        end.col = end.col.saturating_sub(width).max(lead);
                    }
                }
            } else {
                let insert = format!("{token} ");
                let width = insert.chars().count();
                doc.insert(doc.char_at_line_col(*line, indent), &insert);
                for end in [&mut self.cursor, &mut self.anchor] {
                    if end.line == *line && end.col >= indent {
                        end.col = end.col.saturating_add(width);
                    }
                }
            }
        }
        if !self.mode.is_visual() && self.cursor.line < Self::last_addressable(doc) {
            self.cursor.line = self.cursor.line.saturating_add(1);
            self.anchor = self.cursor;
        }
        self.ide_edit_end(doc);
    }

    /// Join the caret line with the next: the newline and the next line's
    /// indentation become one space (none before a closer or after an
    /// opener or blank).
    pub fn ide_join_lines<B: Buffer>(&mut self, doc: &mut B) {
        let line = self.cursor.line;
        if line >= Self::last_addressable(doc) {
            return;
        }
        self.ide_edit_begin(doc);
        let current = doc.line_text(line);
        let next = doc.line_text(line.saturating_add(1));
        let trimmed = current.trim_end().chars().count();
        let lead = IdeText::leading_whitespace(&next).chars().count();
        let from = doc.char_at_line_col(line, trimmed);
        let to = doc.char_at_line_col(line.saturating_add(1), lead);
        doc.delete(from..to);
        let glue = match (
            current.trim_end().chars().last(),
            next.trim_start().chars().next(),
        ) {
            (None, _) | (_, None) => "",
            (Some(last), _) if IdeText::closer_for(last).is_some() => "",
            (_, Some(first)) if IdeText::is_closer(first) || matches!(first, ',' | ';' | '.') => "",
            _ => " ",
        };
        doc.insert(from, glue);
        self.mode = if self.mode == Mode::Insert {
            Mode::Insert
        } else {
            Mode::Normal
        };
        self.cursor = Cursor { line, col: trimmed };
        self.anchor = self.cursor;
        self.ide_edit_end(doc);
    }

    /// Delete back to the previous word start (Alt/Ctrl+Backspace), or the
    /// selection when there is one.
    pub fn ide_delete_word_back<B: Buffer>(&mut self, doc: &mut B) {
        self.ide_edit_begin(doc);
        if !self.ide_delete_selection(doc) {
            let target = self.ide_target(doc, IdeMotion::WordLeft);
            let (from, to) = (self.char_idx(doc, target), self.char_idx(doc, self.cursor));
            let cursor = self.delete_range(doc, from, to);
            self.cursor = cursor;
            self.anchor = cursor;
        }
        self.ide_edit_end(doc);
    }

    /// Select the caret's whole line (triple click), newline included.
    pub fn select_line_at<B: Buffer>(&mut self, doc: &mut B, line: usize) {
        self.begin_mouse_selection(doc, line, 0);
        let last = doc.line_count().saturating_sub(1);
        self.cursor = if self.modeless && line < last {
            Cursor {
                line: line.saturating_add(1),
                col: 0,
            }
        } else {
            Cursor {
                line,
                col: line_len(doc, line),
            }
        };
        self.preferred_col = self.cursor.col;
    }

    /// Shift+click: extend the selection (or start one at the caret) to a
    /// buffer position.
    pub fn extend_selection_to<B: Buffer>(&mut self, doc: &mut B, line: usize, col: usize) {
        if !self.mode.is_visual() {
            if self.mode == Mode::Insert {
                self.enter_normal(doc);
            }
            self.rec_discard();
            self.anchor = self.cursor;
            self.mode = Mode::Visual;
        }
        self.cursor = Cursor { line, col };
        self.clamp(doc);
        self.preferred_col = self.cursor.col;
    }
}
