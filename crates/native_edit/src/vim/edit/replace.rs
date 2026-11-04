//! `r` (replace chars) and `R` (Replace mode: typing overwrites).
use super::super::super::doc::Buffer;
use super::super::helpers::grapheme_right;
use super::super::mode::{Cursor, Effect};
use super::super::state::VimEngine;

impl VimEngine {
    /// `r{c}`: replace `count` chars with `c` (Enter splits the line).
    pub(crate) fn replace_chars<B: Buffer>(
        &mut self,
        doc: &mut B,
        ch: char,
        count: usize,
    ) -> Vec<Effect> {
        let text = doc.line_text(self.cursor.line);
        let len = text.chars().count();
        // Fails (like vim) when fewer than `count` chars remain.
        let mut end = self.cursor.col;
        let mut taken = 0usize;
        while end < len && taken < count {
            end = grapheme_right(&text, end, 1);
            taken = taken.saturating_add(1);
        }
        if taken < count.max(1) {
            return self.finish_simple(false);
        }
        self.x.keys_change = true;
        let from = self.char_idx(doc, self.cursor);
        let to = doc.char_at_line_col(self.cursor.line, end);
        let replacement = if ch == '\n' {
            "\n".to_string()
        } else {
            std::iter::repeat_n(ch, count).collect()
        };
        self.replace_text(doc, (from, to), &replacement);
        self.cursor = if ch == '\n' {
            Cursor {
                line: self.cursor.line.saturating_add(1),
                col: 0,
            }
        } else {
            Cursor {
                line: self.cursor.line,
                col: self.cursor.col.saturating_add(count.saturating_sub(1)),
            }
        };
        self.preferred_col = self.cursor.col;
        self.clear_pending();
        self.clamp(doc);
        self.rec_commit();
        Vec::new()
    }

    /// `R`: Insert mode where typed chars overwrite the line.
    pub(crate) fn start_replace_mode<B: Buffer>(&mut self, doc: &mut B) -> Vec<Effect> {
        self.x.keys_change = true;
        self.begin_change_group(doc);
        self.begin_insert_session(doc);
        self.x.replaced = Some(Vec::new());
        Vec::new()
    }

    /// In `R` mode, remove the char each typed char overwrites (none at
    /// the line end) and remember it for Backspace.
    pub(crate) fn overwrite_for<B: Buffer>(&mut self, doc: &mut B, text: &str) {
        let Some(mut stack) = self.x.replaced.take() else {
            return;
        };
        for c in text.chars() {
            let len = doc.line_text(self.cursor.line).chars().count();
            let at = self.char_idx(doc, self.cursor);
            let col = self.cursor.col;
            if c != '\n' && col < len {
                let old = doc.line_text(self.cursor.line).chars().nth(col);
                doc.delete(at..at.saturating_add(1));
                stack.push(old);
            } else {
                stack.push(None);
            }
        }
        self.x.replaced = Some(stack);
    }

    /// Backspace in `R` mode: step left, putting back what was overwritten.
    /// `false` when not in Replace mode.
    pub(crate) fn replace_backspace<B: Buffer>(&mut self, doc: &mut B) -> bool {
        let Some(stack) = self.x.replaced.as_mut() else {
            return false;
        };
        let at = doc.char_at_line_col(self.cursor.line, self.cursor.col);
        let Some(prev) = at.checked_sub(1) else {
            return true;
        };
        match stack.pop() {
            Some(Some(old)) => {
                doc.delete(prev..at);
                doc.insert(prev, &old.to_string());
            }
            Some(None) => doc.delete(prev..at),
            None => {}
        }
        let (line, col) = doc.line_col_at(prev);
        self.cursor = Cursor { line, col };
        self.preferred_col = col;
        true
    }
}
