//! [`Buffer`] implementation for [`Doc`].

use super::{Buffer, Change, Doc, IndentStyle, UndoChange, UndoOp, content_len, normalize_breaks};
use std::ops::Range;

impl Buffer for Doc {
    fn len_chars(&self) -> usize {
        self.rope.len_chars()
    }

    fn line_count(&self) -> usize {
        self.rope.len_lines()
    }

    fn line_text(&self, line: usize) -> String {
        self.raw_line(line)
    }

    fn len_bytes(&self) -> usize {
        self.rope.len_bytes()
    }

    fn lf_text(&self) -> String {
        let mut text = String::with_capacity(self.rope.len_bytes().saturating_add(1));
        for chunk in self.rope.chunks() {
            text.push_str(chunk);
        }
        text.push('\n');
        match normalize_breaks(&text) {
            std::borrow::Cow::Borrowed(_) => text,
            std::borrow::Cow::Owned(lf) => lf,
        }
    }

    fn char_at_line_col(&self, line: usize, col: usize) -> usize {
        let line = line.min(self.line_count().saturating_sub(1));
        // Count chars in place: copying the line is O(line) per call.
        let len = content_len(self.rope.line(line));
        self.rope
            .line_to_char(line)
            .checked_add(col.min(len))
            .unwrap_or(0)
    }

    fn line_col_at(&self, char_idx: usize) -> (usize, usize) {
        let char_idx = char_idx.min(self.len_chars());
        let line = self.rope.char_to_line(char_idx);
        (line, char_idx.saturating_sub(self.rope.line_to_char(line)))
    }

    fn insert(&mut self, char_idx: usize, text: &str) {
        if text.is_empty() {
            return;
        }
        let at = char_idx.min(self.len_chars());
        self.rope.insert(at, text);
        let len = text.chars().count();
        self.bump(Change::insert(at, len));
        self.commit(UndoOp::Delete { at, len });
    }

    fn delete(&mut self, range: Range<usize>) {
        let end = range.end.min(self.len_chars());
        let start = range.start.min(end);
        if start == end {
            return;
        }
        let removed = self.rope.slice(start..end).to_string();
        self.rope.remove(start..end);
        self.bump(Change::delete(start, end.saturating_sub(start)));
        self.commit(UndoOp::Insert {
            at: start,
            text: removed,
        });
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    fn changes_since(&self, revision: u64) -> Option<Vec<Change>> {
        self.journal.since(revision, self.revision)
    }

    fn indent_style(&self) -> IndentStyle {
        self.indent
    }

    /// Stored ops are already inverses: executing one undoes the edit, and
    /// banking the new inverse makes redo exact. A group replays in
    /// reverse chronological order, so the banked group redoes forward.
    fn undo_change(&mut self) -> Option<UndoChange> {
        self.end_run();
        self.flush_group();
        let step = self.undo.pop()?;
        let (change, inverse) = self.replay(step);
        self.redo.push(inverse);
        Some(change)
    }

    fn redo_change(&mut self) -> Option<UndoChange> {
        self.end_run();
        self.flush_group();
        let step = self.redo.pop()?;
        let (change, inverse) = self.replay(step);
        self.push_undo(inverse);
        Some(UndoChange {
            selection: None,
            ..change
        })
    }

    fn note_selection(&mut self, selection: Option<(usize, usize)>) {
        self.noted = selection;
    }

    fn end_run(&mut self) {
        self.run.op = None;
    }

    fn begin_undo_group(&mut self) {
        self.end_run();
        if self.group_depth == 0 {
            // The unit is one undo step: it keeps the note from before it.
            self.group_before = self.noted.take();
        }
        self.group_depth = self.group_depth.saturating_add(1);
    }

    fn end_undo_group(&mut self) {
        if self.group_depth == 0 {
            return;
        }
        // Nested units (on-type formatting inside an insert session) join
        // the outer one: only the outermost end banks the entry.
        self.group_depth = self.group_depth.saturating_sub(1);
        self.end_run();
        if self.group_depth == 0 {
            self.flush_group();
        }
    }
}
