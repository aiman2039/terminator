//! Modeless typing: selection replacement, bracket pairs, auto-indent,
//! smart Backspace and outdent.
use super::super::super::doc::Buffer;
use super::super::mode::{Cursor, Effect, Key, Mode};
use super::super::state::VimEngine;
use super::text::IdeText;
use super::{INDENT_WIDTH, IdeMotion};

impl VimEngine {
    /// Delete a non-empty modeless selection (typing or Backspace over it).
    /// Returns false, changing nothing, when nothing is selected.
    pub fn ide_delete_selection<B: Buffer>(&mut self, doc: &mut B) -> bool {
        let Some((start, _)) = self.selection_bounds() else {
            return false;
        };
        let linewise = self.mode == Mode::VisualLine;
        let (from, to) = self.selection_range(doc, linewise);
        self.rec_discard();
        let cursor = self.delete_range(doc, from, to);
        self.mode = Mode::Normal;
        self.cursor = if linewise { cursor } else { start };
        self.anchor = self.cursor;
        self.preferred_col = self.cursor.col;
        self.clamp(doc);
        true
    }

    /// Leave any selection and open a modeless insert session at the caret.
    pub(super) fn ide_ensure_insert<B: Buffer>(&mut self, doc: &mut B) {
        if self.mode == Mode::Insert {
            return;
        }
        if self.mode.is_visual() {
            self.mode = Mode::Normal;
            self.anchor = self.cursor;
        }
        self.begin_insert_session(doc);
        self.typed_exact = false;
    }

    /// One discrete key with vim off: navigation, editing, and typing never
    /// act as vim commands. Command mode (the find and replace prompts)
    /// still reads its line through the vim engine.
    pub fn modeless_key<B: Buffer>(&mut self, doc: &mut B, key: Key) -> Vec<Effect> {
        if self.mode == Mode::Command {
            use super::super::mode::ModalEngine;
            return self.press_key(doc, key);
        }
        match key {
            Key::Left => self.ide_move(doc, IdeMotion::Left, false),
            Key::Right => self.ide_move(doc, IdeMotion::Right, false),
            Key::Up => self.ide_move(doc, IdeMotion::Up, false),
            Key::Down => self.ide_move(doc, IdeMotion::Down, false),
            Key::Home => self.ide_move(doc, IdeMotion::LineStart, false),
            Key::End => self.ide_move(doc, IdeMotion::LineEnd, false),
            Key::Escape => {
                if self.mode == Mode::Insert {
                    self.enter_normal(doc);
                } else if self.mode.is_visual() {
                    self.mode = Mode::Normal;
                    self.anchor = self.cursor;
                }
            }
            Key::Tab | Key::BackTab if self.mode.is_visual() => {
                return self.visual_key(doc, key);
            }
            Key::BackTab => self.ide_outdent_line(doc),
            Key::Backspace | Key::Delete if self.ide_delete_selection(doc) => {}
            Key::Char(c) => {
                self.ide_delete_selection(doc);
                self.ide_ensure_insert(doc);
                self.ide_type_char(doc, c);
            }
            Key::Enter | Key::Backspace | Key::Delete | Key::Tab => {
                self.ide_delete_selection(doc);
                self.ide_ensure_insert(doc);
                return self.insert_key(doc, key);
            }
            Key::CtrlV => return vec![Effect::Bell],
        }
        Vec::new()
    }

    /// Indent unit for new indentation: a tab when the file indents with
    /// tabs (the caret line first, else the first indented line), else
    /// [`INDENT_WIDTH`] spaces.
    pub(super) fn ide_indent_unit<B: Buffer>(doc: &B, line: usize) -> String {
        let tab = |text: String| match text.chars().next() {
            Some('\t') => Some(true),
            Some(' ') => Some(false),
            _ => None,
        };
        let uses_tab = tab(doc.line_text(line))
            .or_else(|| (0..doc.line_count().min(1000)).find_map(|line| tab(doc.line_text(line))));
        if uses_tab == Some(true) {
            "\t".into()
        } else {
            " ".repeat(INDENT_WIDTH)
        }
    }

    pub(super) fn ide_insert_at_caret<B: Buffer>(
        &mut self,
        doc: &mut B,
        text: &str,
        caret_advance: usize,
    ) {
        let at = self.char_idx(doc, self.cursor);
        doc.insert(at, text);
        let (line, col) = doc.line_col_at(at.saturating_add(caret_advance));
        self.cursor = Cursor { line, col };
        self.anchor = self.cursor;
        self.preferred_col = col;
        self.clamp(doc);
    }

    /// Modeless typing of one char: closers and quotes type over a matching
    /// next char, openers and `"` insert their pair where a pair can open.
    pub(in crate::vim) fn ide_type_char<B: Buffer>(&mut self, doc: &mut B, c: char) {
        let chars: Vec<char> = doc.line_text(self.cursor.line).chars().collect();
        let next = chars.get(self.cursor.col).copied();
        let prev = self
            .cursor
            .col
            .checked_sub(1)
            .and_then(|p| chars.get(p))
            .copied();
        if (IdeText::is_closer(c) || c == '"') && next == Some(c) {
            self.ide_move(doc, IdeMotion::Right, false);
            return;
        }
        if let Some(close) = IdeText::closer_for(c)
            && IdeText::pair_may_open_before(next)
        {
            self.ide_insert_at_caret(doc, &format!("{c}{close}"), 1);
            return;
        }
        if c == '"'
            && prev.is_none_or(|p| !p.is_alphanumeric() && p != '"' && p != '\\')
            && IdeText::pair_may_open_before(next)
        {
            self.ide_insert_at_caret(doc, "\"\"", 1);
            return;
        }
        let mut buf = [0u8; 4];
        self.ide_insert_at_caret(doc, c.encode_utf8(&mut buf), 1);
    }

    /// Modeless Enter: keep the line's indentation, indent one level after
    /// an opener, and split `{|}` onto three lines.
    pub(in crate::vim) fn ide_enter<B: Buffer>(&mut self, doc: &mut B) {
        let line = self.cursor.line;
        let text = doc.line_text(line);
        let chars: Vec<char> = text.chars().collect();
        let col = self.cursor.col.min(chars.len());
        let indent: String = IdeText::leading_whitespace(&text)
            .chars()
            .take(col)
            .collect();
        let before = chars
            .get(..col)
            .unwrap_or(&[])
            .iter()
            .rev()
            .find(|c| !c.is_whitespace())
            .copied();
        let after = chars.get(col..).unwrap_or(&[]);
        let opener = before.and_then(IdeText::closer_for);
        // Whitespace after the caret would otherwise sit at the start of the
        // new line, ahead of its indentation.
        let trailing_blank = after.iter().take_while(|c| c.is_whitespace()).count();
        let next = after.get(trailing_blank).copied();
        let at = self.char_idx(doc, self.cursor);
        if trailing_blank > 0 {
            doc.delete(at..at.saturating_add(trailing_blank));
        }
        let Some(close) = opener else {
            let text = format!("\n{indent}");
            let caret = text.chars().count();
            self.ide_insert_at_caret(doc, &text, caret);
            return;
        };
        let inner = format!("\n{indent}{}", Self::ide_indent_unit(doc, line));
        let caret = inner.chars().count();
        if next == Some(close) {
            self.ide_insert_at_caret(doc, &format!("{inner}\n{indent}"), caret);
        } else {
            self.ide_insert_at_caret(doc, &inner, caret);
        }
    }

    /// Modeless Tab: one indent unit (spaces up to the next stop).
    pub(in crate::vim) fn ide_tab<B: Buffer>(&mut self, doc: &mut B) {
        let unit = Self::ide_indent_unit(doc, self.cursor.line);
        if unit == "\t" {
            self.ide_insert_at_caret(doc, "\t", 1);
        } else {
            let width = INDENT_WIDTH.saturating_sub(self.cursor.col % INDENT_WIDTH);
            self.ide_insert_at_caret(doc, &" ".repeat(width), width);
        }
    }

    /// Modeless Backspace extras: delete an empty bracket or quote pair as
    /// one, and unindent by a whole level inside leading spaces. Returns
    /// false when a plain one-char Backspace should run instead.
    pub(in crate::vim) fn ide_backspace<B: Buffer>(&mut self, doc: &mut B) -> bool {
        let chars: Vec<char> = doc.line_text(self.cursor.line).chars().collect();
        let col = self.cursor.col.min(chars.len());
        let Some(prev) = col.checked_sub(1).and_then(|p| chars.get(p)).copied() else {
            return false;
        };
        let next = chars.get(col).copied();
        let at = self.char_idx(doc, self.cursor);
        let (from, to) = if (IdeText::closer_for(prev).is_some()
            && IdeText::closer_for(prev) == next)
            || (prev == '"' && next == Some('"'))
        {
            (at.saturating_sub(1), at.saturating_add(1))
        } else if chars
            .get(..col)
            .is_some_and(|head| head.iter().all(|c| *c == ' '))
        {
            let width = match col % INDENT_WIDTH {
                0 => INDENT_WIDTH,
                rest => rest,
            };
            (at.saturating_sub(width.min(col)), at)
        } else {
            return false;
        };
        let cursor = self.delete_range(doc, from, to);
        self.cursor = cursor;
        self.anchor = cursor;
        self.preferred_col = cursor.col;
        self.clamp(doc);
        true
    }

    /// Open an IDE edit: one undo unit that joins an open insert session.
    pub(super) fn ide_edit_begin<B: Buffer>(&mut self, doc: &mut B) {
        self.clear_pending();
        if self.mode == Mode::Insert {
            self.typed_exact = false;
        } else {
            self.rec_discard();
        }
        self.last_change = None;
        doc.begin_undo_group();
    }

    pub(super) fn ide_edit_end<B: Buffer>(&mut self, doc: &mut B) {
        doc.end_undo_group();
        doc.end_run();
        self.clamp(doc);
        self.preferred_col = self.cursor.col;
    }

    /// Shift-Tab without a selection: remove one indent level from the
    /// caret line.
    pub(super) fn ide_outdent_line<B: Buffer>(&mut self, doc: &mut B) {
        let line = self.cursor.line;
        let text = doc.line_text(line);
        let width = if text.starts_with('\t') {
            1
        } else {
            text.chars()
                .take(INDENT_WIDTH)
                .take_while(|c| *c == ' ')
                .count()
        };
        if width == 0 {
            return;
        }
        self.ide_edit_begin(doc);
        let at = doc.char_at_line_col(line, 0);
        doc.delete(at..at.saturating_add(width));
        self.cursor.col = self.cursor.col.saturating_sub(width);
        self.anchor = self.cursor;
        self.ide_edit_end(doc);
    }
}
