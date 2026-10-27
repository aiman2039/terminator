//! IDE-style (`IntelliJ`) editing on top of the modal engine: caret motions
//! with Shift-selection, line operations, and the modeless typing helpers
//! (auto-indent, bracket pairs, smart Backspace).
//!
//! Modeless selections are end-exclusive: the caret sits between chars,
//! so `anchor..cursor` is exactly the selected text. Vim keeps its
//! inclusive cell semantics; [`VimEngine::set_modeless`] picks the model.
use super::super::doc::Buffer;
use super::helpers::range_text;
use super::mode::{Cursor, Effect, Key, Mode};
use super::state::VimEngine;
use super::words::{WordClass, line_len, word_class};

/// Width of one indent level when a file does not indent with tabs.
const INDENT_WIDTH: usize = 4;

/// Caret motion for IDE navigation keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdeMotion {
    Left,
    Right,
    Up,
    Down,
    WordLeft,
    WordRight,
    /// Smart Home: first non-blank, then column 0.
    LineStart,
    LineEnd,
    DocStart,
    DocEnd,
    PageUp(usize),
    PageDown(usize),
}

fn closer_for(open: char) -> Option<char> {
    match open {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        _ => None,
    }
}

fn is_closer(c: char) -> bool {
    matches!(c, ')' | ']' | '}')
}

/// A pair may open before end of line, whitespace, a closer, or a separator;
/// before a word it would wrap text the user did not mean to wrap.
fn pair_may_open_before(next: Option<char>) -> bool {
    next.is_none_or(|c| c.is_whitespace() || is_closer(c) || matches!(c, ',' | ';' | ':'))
}

fn leading_whitespace(text: &str) -> String {
    text.chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

impl VimEngine {
    /// Modeless (vim off): IDE caret and selection semantics. The caret may
    /// sit past the last char of a line and past the final newline, and a
    /// charwise selection excludes the cell under the caret.
    pub fn set_modeless(&mut self, on: bool) {
        self.modeless = on;
    }

    #[must_use]
    pub fn is_modeless(&self) -> bool {
        self.modeless
    }

    /// Selected line span, inclusive. A modeless selection ending at column
    /// 0 does not claim that last line (`IntelliJ` line operations agree).
    pub(super) fn ide_line_span(&self) -> (usize, usize) {
        let (a, b) = if self.mode.is_visual() {
            let (a, b) = (self.anchor, self.cursor);
            if (a.line, a.col) <= (b.line, b.col) {
                (a, b)
            } else {
                (b, a)
            }
        } else {
            (self.cursor, self.cursor)
        };
        let bottom = if self.modeless && self.mode == Mode::Visual && b.col == 0 && b.line > a.line
        {
            b.line.saturating_sub(1)
        } else {
            b.line
        };
        (a.line, bottom)
    }

    fn ide_target<B: Buffer>(&self, doc: &B, motion: IdeMotion) -> Cursor {
        let Cursor { line, col } = self.cursor;
        let last = doc.line_count().saturating_sub(1);
        let vertical = |line: usize| Cursor {
            line,
            col: self.preferred_col.min(line_len(doc, line)),
        };
        match motion {
            IdeMotion::Left => match col.checked_sub(1) {
                Some(col) => Cursor { line, col },
                None if line > 0 => {
                    let line = line.saturating_sub(1);
                    Cursor {
                        line,
                        col: line_len(doc, line),
                    }
                }
                None => self.cursor,
            },
            IdeMotion::Right => {
                if col < line_len(doc, line) {
                    Cursor {
                        line,
                        col: col.saturating_add(1),
                    }
                } else if line < last {
                    Cursor {
                        line: line.saturating_add(1),
                        col: 0,
                    }
                } else {
                    self.cursor
                }
            }
            IdeMotion::Up => vertical(line.saturating_sub(1)),
            IdeMotion::Down => vertical(line.saturating_add(1).min(last)),
            IdeMotion::PageUp(rows) => vertical(line.saturating_sub(rows.max(1))),
            IdeMotion::PageDown(rows) => vertical(line.saturating_add(rows.max(1)).min(last)),
            IdeMotion::WordLeft => {
                let chars: Vec<char> = doc.line_text(line).chars().collect();
                if col == 0 {
                    return self.ide_target(doc, IdeMotion::Left);
                }
                let mut i = col.min(chars.len());
                while i > 0
                    && chars
                        .get(i.saturating_sub(1))
                        .is_some_and(|c| c.is_whitespace())
                {
                    i = i.saturating_sub(1);
                }
                if let Some(class) = i
                    .checked_sub(1)
                    .and_then(|p| chars.get(p))
                    .map(|c| word_class(*c))
                {
                    while i > 0
                        && chars
                            .get(i.saturating_sub(1))
                            .is_some_and(|c| word_class(*c) == class)
                    {
                        i = i.saturating_sub(1);
                    }
                }
                Cursor { line, col: i }
            }
            IdeMotion::WordRight => {
                let chars: Vec<char> = doc.line_text(line).chars().collect();
                if col >= chars.len() {
                    return self.ide_target(doc, IdeMotion::Right);
                }
                let mut i = col;
                while chars.get(i).is_some_and(|c| c.is_whitespace()) {
                    i = i.saturating_add(1);
                }
                if let Some(class) = chars.get(i).map(|c| word_class(*c)) {
                    while chars.get(i).is_some_and(|c| word_class(*c) == class) {
                        i = i.saturating_add(1);
                    }
                }
                Cursor { line, col: i }
            }
            IdeMotion::LineStart => {
                let first = Self::first_non_blank(doc, line);
                Cursor {
                    line,
                    col: if col == first { 0 } else { first },
                }
            }
            IdeMotion::LineEnd => Cursor {
                line,
                col: line_len(doc, line),
            },
            IdeMotion::DocStart => Cursor { line: 0, col: 0 },
            IdeMotion::DocEnd => Cursor {
                line: last,
                col: line_len(doc, last),
            },
        }
    }

    /// Move the caret; `select` extends (or starts) a selection from where
    /// the caret was. Without it, Left/Right collapse a selection to its
    /// near edge and every other motion drops it.
    pub fn ide_move<B: Buffer>(&mut self, doc: &mut B, motion: IdeMotion, select: bool) {
        if self.mode == Mode::Command {
            return;
        }
        self.clear_pending();
        doc.end_run();
        let vertical = matches!(
            motion,
            IdeMotion::Up | IdeMotion::Down | IdeMotion::PageUp(_) | IdeMotion::PageDown(_)
        );
        if select {
            if !self.mode.is_visual() {
                if self.mode == Mode::Insert {
                    self.enter_normal(doc);
                }
                self.rec_discard();
                self.anchor = self.cursor;
                self.mode = Mode::Visual;
            }
            self.cursor = self.ide_target(doc, motion);
        } else if self.mode.is_visual() {
            let collapse = match (motion, self.selection_bounds()) {
                (IdeMotion::Left, Some((start, _))) => Some(start),
                (IdeMotion::Right, Some((_, end))) => Some(end),
                _ => None,
            };
            let target = collapse.unwrap_or_else(|| self.ide_target(doc, motion));
            self.mode = Mode::Normal;
            self.cursor = target;
            self.anchor = target;
        } else {
            self.cursor = self.ide_target(doc, motion);
            self.anchor = self.cursor;
        }
        self.clamp(doc);
        if !vertical {
            self.preferred_col = self.cursor.col;
        }
        if !matches!(motion, IdeMotion::Left | IdeMotion::Right) {
            self.scroll_request = true;
        }
    }

    /// Ordered (start, end) of a non-empty selection.
    fn selection_bounds(&self) -> Option<(Cursor, Cursor)> {
        use super::mode::ModalEngine;
        self.selection()
    }

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
    fn ide_ensure_insert<B: Buffer>(&mut self, doc: &mut B) {
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
            use super::mode::ModalEngine;
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
    fn ide_indent_unit<B: Buffer>(doc: &B, line: usize) -> String {
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

    fn ide_insert_at_caret<B: Buffer>(&mut self, doc: &mut B, text: &str, caret_advance: usize) {
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
    pub(super) fn ide_type_char<B: Buffer>(&mut self, doc: &mut B, c: char) {
        let chars: Vec<char> = doc.line_text(self.cursor.line).chars().collect();
        let next = chars.get(self.cursor.col).copied();
        let prev = self
            .cursor
            .col
            .checked_sub(1)
            .and_then(|p| chars.get(p))
            .copied();
        if (is_closer(c) || c == '"') && next == Some(c) {
            self.ide_move(doc, IdeMotion::Right, false);
            return;
        }
        if let Some(close) = closer_for(c)
            && pair_may_open_before(next)
        {
            self.ide_insert_at_caret(doc, &format!("{c}{close}"), 1);
            return;
        }
        if c == '"'
            && prev.is_none_or(|p| !p.is_alphanumeric() && p != '"' && p != '\\')
            && pair_may_open_before(next)
        {
            self.ide_insert_at_caret(doc, "\"\"", 1);
            return;
        }
        let mut buf = [0u8; 4];
        self.ide_insert_at_caret(doc, c.encode_utf8(&mut buf), 1);
    }

    /// Modeless Enter: keep the line's indentation, indent one level after
    /// an opener, and split `{|}` onto three lines.
    pub(super) fn ide_enter<B: Buffer>(&mut self, doc: &mut B) {
        let line = self.cursor.line;
        let text = doc.line_text(line);
        let chars: Vec<char> = text.chars().collect();
        let col = self.cursor.col.min(chars.len());
        let indent: String = leading_whitespace(&text).chars().take(col).collect();
        let before = chars
            .get(..col)
            .unwrap_or(&[])
            .iter()
            .rev()
            .find(|c| !c.is_whitespace())
            .copied();
        let after = chars.get(col..).unwrap_or(&[]);
        let opener = before.and_then(closer_for);
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
    pub(super) fn ide_tab<B: Buffer>(&mut self, doc: &mut B) {
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
    pub(super) fn ide_backspace<B: Buffer>(&mut self, doc: &mut B) -> bool {
        let chars: Vec<char> = doc.line_text(self.cursor.line).chars().collect();
        let col = self.cursor.col.min(chars.len());
        let Some(prev) = col.checked_sub(1).and_then(|p| chars.get(p)).copied() else {
            return false;
        };
        let next = chars.get(col).copied();
        let at = self.char_idx(doc, self.cursor);
        let (from, to) = if (closer_for(prev).is_some() && closer_for(prev) == next)
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
    fn ide_edit_begin<B: Buffer>(&mut self, doc: &mut B) {
        self.clear_pending();
        if self.mode == Mode::Insert {
            self.typed_exact = false;
        } else {
            self.rec_discard();
        }
        self.last_change = None;
        doc.begin_undo_group();
    }

    fn ide_edit_end<B: Buffer>(&mut self, doc: &mut B) {
        doc.end_undo_group();
        doc.end_run();
        self.clamp(doc);
        self.preferred_col = self.cursor.col;
    }

    /// Shift-Tab without a selection: remove one indent level from the
    /// caret line.
    fn ide_outdent_line<B: Buffer>(&mut self, doc: &mut B) {
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

    /// Char index of the end of `line` (before its newline).
    fn line_end_idx<B: Buffer>(doc: &B, line: usize) -> usize {
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
            .map(|(_, text)| leading_whitespace(text).chars().count())
            .min()
            .unwrap_or(0);
        self.ide_edit_begin(doc);
        for (line, text) in lines.iter().rev() {
            let lead = leading_whitespace(text).chars().count();
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
        let lead = leading_whitespace(&next).chars().count();
        let from = doc.char_at_line_col(line, trimmed);
        let to = doc.char_at_line_col(line.saturating_add(1), lead);
        doc.delete(from..to);
        let glue = match (
            current.trim_end().chars().last(),
            next.trim_start().chars().next(),
        ) {
            (None, _) | (_, None) => "",
            (Some(last), _) if closer_for(last).is_some() => "",
            (_, Some(first)) if is_closer(first) || matches!(first, ',' | ';' | '.') => "",
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

    /// The bracket at (or just before) the caret and its partner, for
    /// matching-brace highlighting. The scan is bounded so a huge
    /// unbalanced file costs at most a few thousand lines per frame.
    #[must_use]
    pub fn matching_bracket<B: Buffer>(&self, doc: &B) -> Option<(Cursor, Cursor)> {
        const MAX_LINES: usize = 2000;
        let Cursor { line, col } = self.cursor;
        let chars: Vec<char> = doc.line_text(line).chars().collect();
        let bracket = |c: char| matches!(c, '(' | ')' | '[' | ']' | '{' | '}');
        let at = [Some(col), col.checked_sub(1)]
            .into_iter()
            .flatten()
            .find(|i| chars.get(*i).copied().is_some_and(bracket))?;
        let open = *chars.get(at)?;
        let start = Cursor { line, col: at };
        if let Some(close) = closer_for(open) {
            let mut depth = 0usize;
            let last = doc.line_count().min(line.saturating_add(MAX_LINES));
            for l in line..last {
                let text: Vec<char> = doc.line_text(l).chars().collect();
                let from = if l == line { at } else { 0 };
                for (i, c) in text.iter().enumerate().skip(from) {
                    if *c == open {
                        depth = depth.saturating_add(1);
                    } else if *c == close {
                        depth = depth.saturating_sub(1);
                        if depth == 0 {
                            return Some((start, Cursor { line: l, col: i }));
                        }
                    }
                }
            }
            return None;
        }
        let close = open;
        let open = match close {
            ')' => '(',
            ']' => '[',
            _ => '{',
        };
        let mut depth = 0usize;
        let first = line.saturating_sub(MAX_LINES);
        for l in (first..=line).rev() {
            let text: Vec<char> = doc.line_text(l).chars().collect();
            let upto = if l == line {
                at.saturating_add(1)
            } else {
                text.len()
            };
            for i in (0..upto.min(text.len())).rev() {
                let c = text.get(i).copied();
                if c == Some(close) {
                    depth = depth.saturating_add(1);
                } else if c == Some(open) {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some((Cursor { line: l, col: i }, start));
                    }
                }
            }
        }
        None
    }

    /// Identifier under (or just before) the caret, for occurrence
    /// highlighting. Punctuation and blanks yield nothing.
    #[must_use]
    pub fn word_at_caret<B: Buffer>(&self, doc: &B) -> Option<String> {
        let chars: Vec<char> = doc.line_text(self.cursor.line).chars().collect();
        let col = self.cursor.col.min(chars.len());
        let on = |i: usize| {
            chars
                .get(i)
                .is_some_and(|c| word_class(*c) == WordClass::Word)
        };
        let hit = if on(col) {
            col
        } else {
            col.checked_sub(1).filter(|p| on(*p))?
        };
        let mut start = hit;
        while start > 0 && on(start.saturating_sub(1)) {
            start = start.saturating_sub(1);
        }
        let mut end = hit;
        while on(end) {
            end = end.saturating_add(1);
        }
        let word: String = chars.get(start..end)?.iter().collect();
        (!word.chars().all(|c| c.is_ascii_digit())).then_some(word)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Doc;
    use crate::vim::ModalEngine;

    fn modeless(text: &str) -> (VimEngine, Doc) {
        let mut engine = VimEngine::new();
        engine.set_modeless(true);
        (engine, Doc::new(text))
    }

    fn at(engine: &mut VimEngine, doc: &mut Doc, line: usize, col: usize) {
        engine.place_cursor(doc, line, col);
    }

    #[test]
    fn shift_right_selects_end_exclusive_text() {
        let (mut engine, mut doc) = modeless("hello");
        engine.ide_move(&mut doc, IdeMotion::Right, true);
        engine.ide_move(&mut doc, IdeMotion::Right, true);
        assert_eq!(engine.copy_text(&doc), "he");
        // Shrinking back to the anchor leaves a bare caret.
        engine.ide_move(&mut doc, IdeMotion::Left, true);
        engine.ide_move(&mut doc, IdeMotion::Left, true);
        assert!(engine.selection().is_none());
    }

    #[test]
    fn plain_arrows_collapse_a_selection_to_its_edge() {
        let (mut engine, mut doc) = modeless("hello");
        at(&mut engine, &mut doc, 0, 1);
        engine.ide_move(&mut doc, IdeMotion::Right, true);
        engine.ide_move(&mut doc, IdeMotion::Right, true);
        engine.ide_move(&mut doc, IdeMotion::Left, false);
        assert!(engine.selection().is_none());
        assert_eq!(engine.cursor(), Cursor { line: 0, col: 1 });
    }

    #[test]
    fn caret_reaches_line_end_and_wraps_across_lines() {
        let (mut engine, mut doc) = modeless("ab\ncd");
        engine.ide_move(&mut doc, IdeMotion::LineEnd, false);
        assert_eq!(engine.cursor(), Cursor { line: 0, col: 2 });
        engine.ide_move(&mut doc, IdeMotion::Right, false);
        assert_eq!(engine.cursor(), Cursor { line: 1, col: 0 });
        engine.ide_move(&mut doc, IdeMotion::Left, false);
        assert_eq!(engine.cursor(), Cursor { line: 0, col: 2 });
        // Typing at the end appends instead of inserting before the last char.
        engine.modeless_key(&mut doc, Key::Char('!'));
        assert_eq!(doc.text(), "ab!\ncd");
    }

    #[test]
    fn word_motions_stop_at_word_edges() {
        let (mut engine, mut doc) = modeless("foo bar.baz");
        engine.ide_move(&mut doc, IdeMotion::WordRight, false);
        assert_eq!(engine.cursor().col, 3);
        engine.ide_move(&mut doc, IdeMotion::WordRight, false);
        assert_eq!(engine.cursor().col, 7);
        engine.ide_move(&mut doc, IdeMotion::WordLeft, false);
        assert_eq!(engine.cursor().col, 4);
    }

    #[test]
    fn smart_home_toggles_between_indent_and_column_zero() {
        let (mut engine, mut doc) = modeless("    x = 1");
        at(&mut engine, &mut doc, 0, 7);
        engine.ide_move(&mut doc, IdeMotion::LineStart, false);
        assert_eq!(engine.cursor().col, 4);
        engine.ide_move(&mut doc, IdeMotion::LineStart, false);
        assert_eq!(engine.cursor().col, 0);
    }

    #[test]
    fn duplicate_line_is_one_undo_step() {
        let (mut engine, mut doc) = modeless("a\nb");
        engine.ide_duplicate(&mut doc);
        assert_eq!(doc.text(), "a\na\nb");
        assert_eq!(engine.cursor().line, 1);
        engine.undo(&mut doc);
        assert_eq!(doc.text(), "a\nb");
    }

    #[test]
    fn duplicate_selection_selects_the_copy() {
        let (mut engine, mut doc) = modeless("ab");
        engine.ide_move(&mut doc, IdeMotion::Right, true);
        engine.ide_duplicate(&mut doc);
        assert_eq!(doc.text(), "aab");
        assert_eq!(engine.copy_text(&doc), "a");
        assert_eq!(engine.cursor().col, 2);
    }

    #[test]
    fn move_lines_carries_the_caret() {
        let (mut engine, mut doc) = modeless("one\ntwo\nthree\n");
        at(&mut engine, &mut doc, 1, 1);
        engine.ide_move_lines(&mut doc, true);
        assert_eq!(doc.text(), "two\none\nthree\n");
        assert_eq!(engine.cursor(), Cursor { line: 0, col: 1 });
        engine.ide_move_lines(&mut doc, false);
        engine.ide_move_lines(&mut doc, false);
        assert_eq!(doc.text(), "one\nthree\ntwo\n");
        // The final newline is not a line to swap with.
        engine.ide_move_lines(&mut doc, false);
        assert_eq!(doc.text(), "one\nthree\ntwo\n");
    }

    #[test]
    fn delete_lines_removes_the_newline_too() {
        let (mut engine, mut doc) = modeless("a\nb\nc");
        at(&mut engine, &mut doc, 1, 0);
        engine.ide_delete_lines(&mut doc);
        assert_eq!(doc.text(), "a\nc");
        at(&mut engine, &mut doc, 1, 0);
        engine.ide_delete_lines(&mut doc);
        assert_eq!(doc.text(), "a");
    }

    #[test]
    fn toggle_comment_round_trips_at_common_indent() {
        let (mut engine, mut doc) = modeless("fn a() {\n    x();\n}\n");
        engine.ide_move(&mut doc, IdeMotion::Down, true);
        engine.ide_move(&mut doc, IdeMotion::Down, true);
        engine.ide_toggle_comment(&mut doc, "//");
        assert_eq!(doc.text(), "// fn a() {\n//     x();\n}\n");
        engine.ide_toggle_comment(&mut doc, "//");
        assert_eq!(doc.text(), "fn a() {\n    x();\n}\n");
    }

    #[test]
    fn toggle_comment_without_selection_moves_down() {
        let (mut engine, mut doc) = modeless("a\nb\n");
        engine.ide_toggle_comment(&mut doc, "#");
        assert_eq!(doc.text(), "# a\nb\n");
        assert_eq!(engine.cursor().line, 1);
    }

    #[test]
    fn enter_keeps_indent_and_splits_braces() {
        let (mut engine, mut doc) = modeless("    if x {}");
        at(&mut engine, &mut doc, 0, 10);
        engine.modeless_key(&mut doc, Key::Enter);
        assert_eq!(doc.text(), "    if x {\n        \n    }");
        assert_eq!(engine.cursor(), Cursor { line: 1, col: 8 });
        engine.modeless_key(&mut doc, Key::Char('y'));
        engine.modeless_key(&mut doc, Key::Enter);
        assert_eq!(doc.text(), "    if x {\n        y\n        \n    }");
    }

    #[test]
    fn brackets_pair_and_type_over_their_closer() {
        let (mut engine, mut doc) = modeless("");
        engine.modeless_key(&mut doc, Key::Char('f'));
        engine.type_text(&mut doc, "(");
        assert_eq!(doc.text(), "f()");
        assert_eq!(engine.cursor().col, 2);
        engine.type_text(&mut doc, ")");
        assert_eq!(doc.text(), "f()");
        assert_eq!(engine.cursor().col, 3);
    }

    #[test]
    fn opener_before_a_word_does_not_pair() {
        let (mut engine, mut doc) = modeless("x");
        engine.modeless_key(&mut doc, Key::Char('('));
        assert_eq!(doc.text(), "(x");
    }

    #[test]
    fn backspace_removes_an_empty_pair_and_whole_indent_levels() {
        let (mut engine, mut doc) = modeless("");
        engine.modeless_key(&mut doc, Key::Char('['));
        engine.modeless_key(&mut doc, Key::Backspace);
        assert_eq!(doc.text(), "");
        let (mut engine, mut doc) = modeless("        x");
        at(&mut engine, &mut doc, 0, 8);
        engine.modeless_key(&mut doc, Key::Backspace);
        assert_eq!(doc.text(), "    x");
    }

    #[test]
    fn typing_replaces_the_selection() {
        let (mut engine, mut doc) = modeless("hello world");
        engine.ide_move(&mut doc, IdeMotion::WordRight, true);
        engine.modeless_key(&mut doc, Key::Char('J'));
        assert_eq!(doc.text(), "J world");
        engine.ide_move(&mut doc, IdeMotion::LineEnd, true);
        engine.modeless_key(&mut doc, Key::Backspace);
        assert_eq!(doc.text(), "J");
    }

    #[test]
    fn modeless_undo_works_without_escape() {
        let (mut engine, mut doc) = modeless("x");
        at(&mut engine, &mut doc, 0, 1);
        engine.modeless_key(&mut doc, Key::Char('y'));
        engine.type_text(&mut doc, "z");
        engine.undo(&mut doc);
        assert_eq!(doc.text(), "x");
    }

    #[test]
    fn tab_inserts_spaces_to_the_next_stop() {
        let (mut engine, mut doc) = modeless("ab");
        at(&mut engine, &mut doc, 0, 1);
        engine.modeless_key(&mut doc, Key::Tab);
        assert_eq!(doc.text(), "a   b");
        let (mut engine, mut doc) = modeless("\tx\n");
        at(&mut engine, &mut doc, 1, 0);
        engine.modeless_key(&mut doc, Key::Tab);
        assert_eq!(doc.text(), "\tx\n\t");
    }

    #[test]
    fn join_lines_glues_with_one_space() {
        let (mut engine, mut doc) = modeless("let x =\n    1;\nfoo(\n  a)");
        engine.ide_join_lines(&mut doc);
        assert_eq!(doc.text(), "let x = 1;\nfoo(\n  a)");
        at(&mut engine, &mut doc, 1, 0);
        engine.ide_join_lines(&mut doc);
        assert_eq!(doc.text(), "let x = 1;\nfoo(a)");
    }

    #[test]
    fn delete_word_back_stops_at_the_word_start() {
        let (mut engine, mut doc) = modeless("let value");
        engine.ide_move(&mut doc, IdeMotion::LineEnd, false);
        engine.ide_delete_word_back(&mut doc);
        assert_eq!(doc.text(), "let ");
    }

    #[test]
    fn matching_bracket_and_caret_word() {
        let (mut engine, mut doc) = modeless("f(a[0], {b})\n");
        at(&mut engine, &mut doc, 0, 1);
        assert_eq!(
            engine.matching_bracket(&doc),
            Some((Cursor { line: 0, col: 1 }, Cursor { line: 0, col: 11 }))
        );
        at(&mut engine, &mut doc, 0, 10);
        assert_eq!(
            engine.matching_bracket(&doc),
            Some((Cursor { line: 0, col: 8 }, Cursor { line: 0, col: 10 }))
        );
        at(&mut engine, &mut doc, 0, 2);
        assert_eq!(engine.word_at_caret(&doc).as_deref(), Some("a"));
    }

    #[test]
    fn tab_indents_only_lines_the_selection_covers() {
        let (mut engine, mut doc) = modeless("a\nb\nc");
        engine.ide_move(&mut doc, IdeMotion::Down, true);
        engine.modeless_key(&mut doc, Key::Tab);
        assert_eq!(doc.text(), "    a\nb\nc");
    }

    #[test]
    fn vim_mode_keeps_inclusive_selections() {
        let mut engine = VimEngine::new();
        let mut doc = Doc::new("hello");
        engine.ide_move(&mut doc, IdeMotion::Right, true);
        assert_eq!(engine.copy_text(&doc), "he");
    }
}
