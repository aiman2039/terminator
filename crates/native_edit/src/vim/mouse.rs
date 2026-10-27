//! Mouse-driven selection: drag and double-click word select, mapped onto
//! the same anchor/cursor pair the `v` visual modes use, so copy, cut,
//! indent, and typing over the selection all work unchanged.
use super::super::doc::Buffer;
use super::mode::{Cursor, Mode};
use super::state::VimEngine;
use super::words::{WordClass, word_class};

impl VimEngine {
    /// Start a drag selection at a buffer position: the press point is both
    /// anchor and cursor, replacing any earlier selection.
    pub fn begin_mouse_selection<B: Buffer>(&mut self, doc: &mut B, line: usize, col: usize) {
        doc.end_run();
        self.clear_pending();
        self.rec_discard();
        self.mode = Mode::Visual;
        self.cursor = Cursor { line, col };
        self.clamp(doc);
        self.anchor = self.cursor;
        self.preferred_col = self.cursor.col;
    }

    /// Extend a drag to a new buffer position, keeping the anchor. Ignored
    /// outside a visual mode, so a stray move cannot invent a selection.
    pub fn extend_mouse_selection<B: Buffer>(&mut self, doc: &mut B, line: usize, col: usize) {
        if !self.mode.is_visual() {
            return;
        }
        self.cursor = Cursor { line, col };
        self.clamp(doc);
        self.preferred_col = self.cursor.col;
    }

    /// Double-click select: the run of same-class characters under the
    /// pointer (word characters, or a punctuation run). Blank space parks
    /// the cursor there instead.
    pub fn select_word_at<B: Buffer>(&mut self, doc: &mut B, line: usize, col: usize) {
        let chars: Vec<char> = doc.line_text(line).chars().collect();
        let class = chars.get(col).map_or(WordClass::Blank, |c| word_class(*c));
        if class == WordClass::Blank {
            // Parking is not a selection: a double-click in the margin or
            // past the line end drops whatever was selected before.
            doc.end_run();
            self.clear_pending();
            self.rec_discard();
            self.mode = Mode::Normal;
            self.cursor = Cursor { line, col };
            self.anchor = self.cursor;
            self.clamp(doc);
            self.preferred_col = self.cursor.col;
            return;
        }
        let mut start = col;
        while let Some(previous) = start.checked_sub(1)
            && chars.get(previous).is_some_and(|c| word_class(*c) == class)
        {
            start = previous;
        }
        let mut end = col;
        while chars.get(end).is_some_and(|c| word_class(*c) == class) {
            end = end.saturating_add(1);
        }
        self.begin_mouse_selection(doc, line, start);
        // The selection ends on the last char of the run (end-exclusive
        // `end`), which is what the visual range paints.
        // Modeless selections end-exclusive, so the caret sits past it.
        self.cursor = Cursor {
            line,
            col: if self.modeless {
                end
            } else {
                end.saturating_sub(1).max(start)
            },
        };
        self.preferred_col = self.cursor.col;
    }

    /// Drop a mouse selection without moving the cursor: a plain click
    /// collapses the drag selection instead of extending it like `v` does.
    pub fn clear_mouse_selection<B: Buffer>(&mut self, doc: &mut B) {
        if !self.mode.is_visual() {
            return;
        }
        self.mode = Mode::Normal;
        self.clamp(doc);
        self.anchor = self.cursor;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Doc;
    use crate::vim::ModalEngine;

    #[test]
    fn drag_selection_covers_both_ends() {
        let mut doc = Doc::new("alpha beta\ngamma\n");
        let mut engine = VimEngine::new();
        engine.begin_mouse_selection(&mut doc, 0, 2);
        engine.extend_mouse_selection(&mut doc, 1, 3);
        let (start, end) = engine.selection().expect("drag selects");
        assert_eq!((start.line, start.col), (0, 2));
        assert_eq!((end.line, end.col), (1, 3));
        assert_eq!(engine.copy_text(&doc), "pha beta\ngamm");

        engine.clear_mouse_selection(&mut doc);
        assert!(engine.selection().is_none());
        assert_eq!(engine.mode(), Mode::Normal);
    }

    #[test]
    fn extending_without_a_selection_is_ignored() {
        let mut doc = Doc::new("alpha\n");
        let mut engine = VimEngine::new();
        engine.extend_mouse_selection(&mut doc, 0, 3);
        assert!(engine.selection().is_none());
        assert_eq!(engine.cursor(), Cursor { line: 0, col: 0 });
    }

    #[test]
    fn double_click_selects_the_word_run_under_the_pointer() {
        let mut doc = Doc::new("let value_1 = other;\n");
        let mut engine = VimEngine::new();
        engine.select_word_at(&mut doc, 0, 5);
        let (start, end) = engine.selection().expect("word selected");
        assert_eq!((start.col, end.col), (4, 10));
        assert_eq!(engine.copy_text(&doc), "value_1");

        // Punctuation selects its own run, vim-style.
        engine.select_word_at(&mut doc, 0, 12);
        let (start, end) = engine.selection().expect("punct selected");
        assert_eq!((start.col, end.col), (12, 12));

        // Blank space just parks the cursor.
        engine.select_word_at(&mut doc, 0, 11);
        assert!(engine.selection().is_none());
        assert_eq!(engine.mode(), Mode::Normal);
        assert_eq!(engine.cursor().col, 11);
    }
}
