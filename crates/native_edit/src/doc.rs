//! Native editing core: document buffer plus undo.
//!
//! Sandbox note: `ropey` could not be vendored here (no crate downloads), so
//! [`Doc`] is backed by a `String` with a cached line index. Every consumer
//! programs against the [`Buffer`] trait, which mirrors the `ropey` surface we
//! will adopt later (`char` indexing, cheap line access); swapping the backing
//! store must not touch callers. Undo uses typing-run coalescing.

use std::ops::Range;

/// Char-indexed text buffer. Indices are Unicode scalar values, matching the
/// `ropey` convention, so callers never deal in bytes.
pub trait Buffer {
    fn len_chars(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len_chars() == 0
    }
    fn line_count(&self) -> usize;
    fn line_text(&self, line: usize) -> String;
    fn char_at_line_col(&self, line: usize, col: usize) -> usize;
    fn line_col_at(&self, char_idx: usize) -> (usize, usize);
    fn insert(&mut self, char_idx: usize, text: &str);
    fn delete(&mut self, range: Range<usize>);
    fn undo(&mut self) -> bool;
    fn redo(&mut self) -> bool;
    /// End the coalescing run: the next edit starts a fresh undo unit.
    /// Call after cursor moves, mode changes, saves, and remote updates.
    fn end_run(&mut self);
}

#[derive(Clone, Debug)]
enum UndoOp {
    /// Insert `text` at `at` to undo (i.e. a delete happened).
    Insert { at: usize, text: String },
    /// Delete `len` chars at `at` to undo (i.e. an insert happened).
    Delete { at: usize, len: usize },
}

#[derive(Clone, Debug, Default)]
struct EditRun {
    op: Option<UndoOp>,
}

impl EditRun {
    /// Merge `op` into the open run when it directly continues it:
    /// contiguous typing appends, contiguous forward deletes append, and
    /// contiguous backward deletes prepend (with the stored position moved).
    fn push(&mut self, op: UndoOp) -> bool {
        let slot = self.op.take();
        let (merged, next) = match (slot, op) {
            (Some(UndoOp::Delete { at: a, len: la }), UndoOp::Delete { at: b, len: lb })
                if a + la == b =>
            {
                (
                    true,
                    Some(UndoOp::Delete {
                        at: a,
                        len: la + lb,
                    }),
                )
            }
            (
                Some(UndoOp::Insert {
                    at: a,
                    text: mut ta,
                }),
                UndoOp::Insert { at: b, text: tb },
            ) if b == a => {
                ta.push_str(&tb);
                (true, Some(UndoOp::Insert { at: a, text: ta }))
            }
            (
                Some(UndoOp::Insert {
                    at: a,
                    text: mut ta,
                }),
                UndoOp::Insert { at: b, text: tb },
            ) if b + tb.chars().count() == a => {
                ta.insert_str(0, &tb);
                (true, Some(UndoOp::Insert { at: b, text: ta }))
            }
            (slot, op) => (false, {
                let _ = slot;
                Some(op)
            }),
        };
        // `push` is only called from `commit`, which immediately stores `next`
        // back into the undo stack when unmerged; keep the merged run here.
        self.op = next;
        merged
    }
}

/// `String`-backed [`Buffer`] with a cached line-start table.
#[derive(Clone, Debug, Default)]
pub struct Doc {
    text: String,
    line_starts: Vec<usize>,
    undo: Vec<UndoOp>,
    redo: Vec<UndoOp>,
    run: EditRun,
}

impl Doc {
    pub fn new(text: impl Into<String>) -> Self {
        let mut doc = Self {
            text: text.into(),
            line_starts: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            run: EditRun::default(),
        };
        doc.reindex();
        doc
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    fn reindex(&mut self) {
        self.line_starts.clear();
        self.line_starts.push(0);
        for (i, c) in self.text.char_indices() {
            if c == '\n' {
                self.line_starts.push(i + 1);
            }
        }
    }

    fn commit(&mut self, op: UndoOp) {
        if self.run.push(op.clone()) {
            let last = self.undo.len().saturating_sub(1);
            self.undo[last] = self.run.op.clone().expect("run");
        } else {
            self.undo.push(op);
        }
        self.redo.clear();
    }

    /// Execute an inverse op, returning the inverse that reverses it.
    fn execute(&mut self, op: UndoOp) -> UndoOp {
        match op {
            UndoOp::Insert { at, text } => {
                let at = at.min(self.len_chars());
                let len = text.chars().count();
                self.text.insert_str(byte_idx(&self.text, at), &text);
                self.reindex();
                UndoOp::Delete { at, len }
            }
            UndoOp::Delete { at, len } => {
                let end = (at + len).min(self.len_chars());
                let removed: String = self.text.chars().skip(at).take(end - at).collect();
                self.text.replace_range(byte_range(&self.text, at, end), "");
                self.reindex();
                UndoOp::Insert { at, text: removed }
            }
        }
    }
}

fn byte_idx(text: &str, char_idx: usize) -> usize {
    text.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(text.len())
}

fn byte_range(text: &str, start: usize, end: usize) -> std::ops::Range<usize> {
    byte_idx(text, start)..byte_idx(text, end.min(text.chars().count()))
}

impl Buffer for Doc {
    fn len_chars(&self) -> usize {
        self.text.chars().count()
    }

    fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    fn line_text(&self, line: usize) -> String {
        let Some(&start_byte) = self.line_starts.get(line) else {
            return String::new();
        };
        let start_char = char_idx_of_byte(&self.text, start_byte);
        let end_char = self
            .line_starts
            .get(line + 1)
            .map(|b| char_idx_of_byte(&self.text, *b).saturating_sub(1))
            .unwrap_or_else(|| self.text.chars().count());
        self.text
            .chars()
            .skip(start_char)
            .take(end_char.saturating_sub(start_char))
            .collect()
    }

    fn char_at_line_col(&self, line: usize, col: usize) -> usize {
        let line = line.min(self.line_count().saturating_sub(1));
        let start = char_idx_of_byte(&self.text, self.line_starts[line]);
        let len = self.line_text(line).chars().count();
        start + col.min(len)
    }

    fn line_col_at(&self, char_idx: usize) -> (usize, usize) {
        let char_idx = char_idx.min(self.len_chars());
        let byte = byte_idx(&self.text, char_idx);
        let line = self
            .line_starts
            .iter()
            .rposition(|s| *s <= byte)
            .unwrap_or(0);
        (
            line,
            char_idx - char_idx_of_byte(&self.text, self.line_starts[line]),
        )
    }

    fn insert(&mut self, char_idx: usize, text: &str) {
        if text.is_empty() {
            return;
        }
        let at = char_idx.min(self.len_chars());
        self.text.insert_str(byte_idx(&self.text, at), text);
        self.reindex();
        let len = text.chars().count();
        self.commit(UndoOp::Delete { at, len });
    }

    fn delete(&mut self, range: Range<usize>) {
        let end = range.end.min(self.len_chars());
        let start = range.start.min(end);
        if start == end {
            return;
        }
        let removed: String = self.text.chars().skip(start).take(end - start).collect();
        self.text
            .replace_range(byte_range(&self.text, start, end), "");
        self.reindex();
        self.commit(UndoOp::Insert {
            at: start,
            text: removed,
        });
    }

    /// Stored ops are already inverses: executing one undoes the edit, and
    /// banking the new inverse makes redo exact.
    fn undo(&mut self) -> bool {
        self.end_run();
        let Some(op) = self.undo.pop() else {
            return false;
        };
        let inverse = self.execute(op);
        self.redo.push(inverse);
        true
    }

    fn redo(&mut self) -> bool {
        self.end_run();
        let Some(op) = self.redo.pop() else {
            return false;
        };
        let inverse = self.execute(op);
        self.undo.push(inverse);
        true
    }

    fn end_run(&mut self) {
        self.run.op = None;
    }
}

fn char_idx_of_byte(text: &str, byte: usize) -> usize {
    text.char_indices().take_while(|(b, _)| *b < byte).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_delete_round_trip() {
        let mut doc = Doc::new("hello");
        doc.insert(5, " world");
        assert_eq!(doc.text(), "hello world");
        doc.delete(5..11);
        assert_eq!(doc.text(), "hello");
    }

    #[test]
    fn unicode_indices_are_char_based() {
        let mut doc = Doc::new("日本語");
        doc.insert(3, "テスト");
        assert_eq!(doc.text(), "日本語テスト");
        assert_eq!(doc.line_col_at(4), (0, 4));
        assert_eq!(doc.char_at_line_col(0, 4), 4);
    }

    #[test]
    fn lines_split_on_newlines() {
        let doc = Doc::new("a\nbb\n");
        assert_eq!(doc.line_count(), 3);
        assert_eq!(doc.line_text(1), "bb");
        assert_eq!(doc.char_at_line_col(1, 1), 3);
        assert_eq!(doc.line_col_at(3), (1, 1));
    }

    #[test]
    fn typing_run_coalesces_into_one_undo() {
        let mut doc = Doc::new(String::new());
        doc.insert(0, "a");
        doc.insert(1, "b");
        doc.insert(2, "c");
        assert!(doc.undo());
        assert_eq!(doc.text(), "");
    }

    #[test]
    fn cursor_move_ends_the_run() {
        let mut doc = Doc::new(String::new());
        doc.insert(0, "a");
        doc.end_run();
        doc.insert(1, "b");
        assert!(doc.undo());
        assert_eq!(doc.text(), "a");
        assert!(doc.undo());
        assert_eq!(doc.text(), "");
    }

    #[test]
    fn undo_redo_delete_restores_text() {
        let mut doc = Doc::new("hello");
        doc.delete(1..4);
        assert_eq!(doc.text(), "ho");
        assert!(doc.undo());
        assert_eq!(doc.text(), "hello");
        assert!(doc.redo());
        assert_eq!(doc.text(), "ho");
    }

    #[test]
    fn out_of_range_edits_clamp() {
        let mut doc = Doc::new("hi");
        doc.insert(99, "!");
        assert_eq!(doc.text(), "hi!");
        doc.delete(5..50);
        assert_eq!(doc.text(), "hi!");
        assert!(!Doc::new("x").undo());
    }
}
