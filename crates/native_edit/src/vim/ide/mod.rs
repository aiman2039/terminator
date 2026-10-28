//! IDE-style (`IntelliJ`) editing on top of the modal engine: caret motions
//! with Shift-selection, line operations, and the modeless typing helpers
//! (auto-indent, bracket pairs, smart Backspace).
//!
//! Modeless selections are end-exclusive: the caret sits between chars,
//! so `anchor..cursor` is exactly the selected text. Vim keeps its
//! inclusive cell semantics; [`VimEngine::set_modeless`] picks the model.
mod lines;
mod selection;
#[cfg(test)]
mod tests;
mod text;
mod typing;

use super::super::doc::Buffer;
use super::mode::{Cursor, Mode};
use super::state::VimEngine;
use super::words::{line_len, word_class};

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

    /// Ask the view to bring the caret into sight (centered) next frame.
    pub fn request_scroll(&mut self) {
        self.scroll_request = true;
    }

    /// A fresh engine for another view of the same buffer: same options,
    /// mode model, and caret, with no selection or pending command.
    #[must_use]
    pub fn fork_view(&self) -> Self {
        let mut engine = Self::new();
        engine.options = self.options;
        engine.modeless = self.modeless;
        engine.cursor = self.cursor;
        engine.anchor = self.cursor;
        engine.preferred_col = self.preferred_col;
        engine
    }

    /// Selected line span, inclusive. A modeless selection ending at column
    /// 0 does not claim that last line (`IntelliJ` line operations agree).
    pub(in crate::vim) fn ide_line_span(&self) -> (usize, usize) {
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
            IdeMotion::Left => self.ide_left(doc),
            IdeMotion::Right => self.ide_right(doc),
            IdeMotion::Up => vertical(line.saturating_sub(1)),
            IdeMotion::Down => vertical(line.saturating_add(1).min(last)),
            IdeMotion::PageUp(rows) => vertical(line.saturating_sub(rows.max(1))),
            IdeMotion::PageDown(rows) => vertical(line.saturating_add(rows.max(1)).min(last)),
            IdeMotion::WordLeft => self.ide_word_left(doc),
            IdeMotion::WordRight => self.ide_word_right(doc),
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

    /// One char left, wrapping to the end of the previous line.
    fn ide_left<B: Buffer>(&self, doc: &B) -> Cursor {
        let Cursor { line, col } = self.cursor;
        match col.checked_sub(1) {
            Some(col) => Cursor { line, col },
            None if line > 0 => {
                let line = line.saturating_sub(1);
                Cursor {
                    line,
                    col: line_len(doc, line),
                }
            }
            None => self.cursor,
        }
    }

    /// One char right, wrapping to the start of the next line.
    fn ide_right<B: Buffer>(&self, doc: &B) -> Cursor {
        let Cursor { line, col } = self.cursor;
        if col < line_len(doc, line) {
            Cursor {
                line,
                col: col.saturating_add(1),
            }
        } else if line < doc.line_count().saturating_sub(1) {
            Cursor {
                line: line.saturating_add(1),
                col: 0,
            }
        } else {
            self.cursor
        }
    }

    /// Back over blanks, then over one word class; at column 0, one char.
    fn ide_word_left<B: Buffer>(&self, doc: &B) -> Cursor {
        let Cursor { line, col } = self.cursor;
        let chars: Vec<char> = doc.line_text(line).chars().collect();
        if col == 0 {
            return self.ide_left(doc);
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

    /// Over blanks, then over one word class; at line end, one char.
    fn ide_word_right<B: Buffer>(&self, doc: &B) -> Cursor {
        let Cursor { line, col } = self.cursor;
        let chars: Vec<char> = doc.line_text(line).chars().collect();
        if col >= chars.len() {
            return self.ide_right(doc);
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
}
