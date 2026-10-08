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
    /// Monotonic edit counter: every text mutation (insert, delete,
    /// undo, redo) bumps it. Views use it to invalidate cached
    /// derivations (highlighting) without rehashing the buffer.
    /// Backends without a counter report `0` (always re-derive).
    fn revision(&self) -> u64 {
        0
    }
    /// End the coalescing run: the next edit starts a fresh undo unit.
    /// Call after cursor moves, mode changes, saves, and remote updates.
    fn end_run(&mut self);
    /// Begin an explicit undo unit (one vim change = one undo step):
    /// every edit until [`Buffer::end_undo_group`] lands on a single
    /// undo entry. Nesting closes out at the outermost end; the default
    /// is no grouping (each edit stays its own step).
    fn begin_undo_group(&mut self) {}
    /// Close the unit opened by [`Buffer::begin_undo_group`]. An empty
    /// unit records nothing.
    fn end_undo_group(&mut self) {}
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
                if a.checked_add(la) == Some(b) =>
            {
                match la.checked_add(lb) {
                    Some(len) => (true, Some(UndoOp::Delete { at: a, len })),
                    None => (false, Some(UndoOp::Delete { at: b, len: lb })),
                }
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
            ) if b.checked_add(tb.chars().count()) == Some(a) => {
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

/// One undo-stack entry: a single edit or an explicit group of edits
/// (one vim change) that undoes and redoes atomically.
#[derive(Clone, Debug)]
enum UndoEntry {
    Op(UndoOp),
    Group(Vec<UndoOp>),
}

/// Rope-backed [`Buffer`]: `ropey` owns storage and line indexing, so
/// edits stay cheap on large files. Char indices are the unit everywhere,
/// exactly as before the migration.
#[derive(Clone, Debug, Default)]
pub struct Doc {
    rope: ropey::Rope,
    revision: u64,
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
    run: EditRun,
    group_depth: usize,
    group: Vec<UndoOp>,
}

impl Doc {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            rope: ropey::Rope::from(text.into()),
            revision: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            run: EditRun::default(),
            group_depth: 0,
            group: Vec::new(),
        }
    }

    #[must_use]
    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    fn bump(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }

    /// Line `line` without its terminator (a lone `\n` is stripped; a
    /// `\r` is kept, matching the old `String` backend).
    fn raw_line(&self, line: usize) -> String {
        if line >= self.rope.len_lines() {
            return String::new();
        }
        let mut text = self.rope.line(line).to_string();
        if text.ends_with('\n') {
            text.pop();
        }
        text
    }

    fn commit(&mut self, op: UndoOp) {
        if self.group_depth > 0 {
            // Inside an explicit unit every edit is banked verbatim; the
            // run merger stays closed so nothing leaks into neighbors.
            self.run.op = None;
            self.group.push(op);
            self.redo.clear();
            return;
        }
        if self.run.push(op.clone()) {
            let last = self.undo.len().saturating_sub(1);
            if let Some(stored) = self.run.op.clone()
                && let Some(UndoEntry::Op(slot)) = self.undo.get_mut(last)
            {
                *slot = stored;
            }
        } else {
            self.undo.push(UndoEntry::Op(op));
        }
        self.redo.clear();
    }

    /// Execute one undo entry (single op or whole group), returning the
    /// entry that reverses it. Groups replay in reverse chronological
    /// order, so the banked group redoes forward.
    fn execute_entry(&mut self, entry: UndoEntry) -> UndoEntry {
        match entry {
            UndoEntry::Op(op) => UndoEntry::Op(self.execute(op)),
            UndoEntry::Group(ops) => {
                let mut inverses = Vec::with_capacity(ops.len());
                for op in ops.into_iter().rev() {
                    inverses.push(self.execute(op));
                }
                UndoEntry::Group(inverses)
            }
        }
    }

    /// Execute an inverse op, returning the inverse that reverses it.
    fn execute(&mut self, op: UndoOp) -> UndoOp {
        match op {
            UndoOp::Insert { at, text } => {
                let at = at.min(self.len_chars());
                let len = text.chars().count();
                self.rope.insert(at, &text);
                self.bump();
                UndoOp::Delete { at, len }
            }
            UndoOp::Delete { at, len } => {
                let end = at
                    .checked_add(len)
                    .map_or(self.len_chars(), |sum| sum.min(self.len_chars()));
                let removed = self.rope.slice(at.min(end)..end).to_string();
                self.rope.remove(at.min(end)..end);
                self.bump();
                UndoOp::Insert { at, text: removed }
            }
        }
    }
}

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

    fn char_at_line_col(&self, line: usize, col: usize) -> usize {
        let line = line.min(self.line_count().saturating_sub(1));
        let len = self.raw_line(line).chars().count();
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
        self.bump();
        let len = text.chars().count();
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
        self.bump();
        self.commit(UndoOp::Insert {
            at: start,
            text: removed,
        });
    }

    fn revision(&self) -> u64 {
        self.revision
    }

    /// Stored ops are already inverses: executing one undoes the edit, and
    /// banking the new inverse makes redo exact. A group replays in
    /// reverse chronological order, so the banked group redoes forward.
    fn undo(&mut self) -> bool {
        self.end_run();
        let Some(entry) = self.undo.pop() else {
            return false;
        };
        let inverse = self.execute_entry(entry);
        self.redo.push(inverse);
        true
    }

    fn redo(&mut self) -> bool {
        self.end_run();
        let Some(entry) = self.redo.pop() else {
            return false;
        };
        let inverse = self.execute_entry(entry);
        self.undo.push(inverse);
        true
    }

    fn end_run(&mut self) {
        self.run.op = None;
    }

    fn begin_undo_group(&mut self) {
        self.end_run();
        self.group_depth = self.group_depth.saturating_add(1);
    }

    fn end_undo_group(&mut self) {
        if self.group_depth == 0 {
            return;
        }
        // One unit, one entry: close out nesting so a change spanning
        // delete-plus-insert (e.g. `cw`) never splits across steps.
        self.group_depth = 0;
        self.end_run();
        if !self.group.is_empty() {
            self.redo.clear();
            self.undo
                .push(UndoEntry::Group(std::mem::take(&mut self.group)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_delete_round_trip() {
        let mut doc = Doc::new("hello");
        doc.insert(5, " world");
        assert_eq!(doc.text().as_str(), "hello world");
        doc.delete(5..11);
        assert_eq!(doc.text().as_str(), "hello");
    }

    #[test]
    fn unicode_indices_are_char_based() {
        let mut doc = Doc::new("日本語");
        doc.insert(3, "テスト");
        assert_eq!(doc.text().as_str(), "日本語テスト");
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
        assert_eq!(doc.text().as_str(), "");
    }

    #[test]
    fn undo_group_is_one_step_and_redoes_forward() {
        let mut doc = Doc::new(String::new());
        doc.begin_undo_group();
        doc.insert(0, "a");
        doc.insert(1, "b");
        doc.delete(0..1);
        doc.end_undo_group();
        assert_eq!(doc.text().as_str(), "b");
        assert!(doc.undo());
        assert_eq!(doc.text().as_str(), "");
        assert!(doc.redo());
        assert_eq!(doc.text().as_str(), "b");
    }

    #[test]
    fn empty_group_records_nothing() {
        let mut doc = Doc::new("x");
        doc.begin_undo_group();
        doc.end_undo_group();
        assert!(!doc.undo());
        assert_eq!(doc.text().as_str(), "x");
    }

    #[test]
    fn cursor_move_ends_the_run() {
        let mut doc = Doc::new(String::new());
        doc.insert(0, "a");
        doc.end_run();
        doc.insert(1, "b");
        assert!(doc.undo());
        assert_eq!(doc.text().as_str(), "a");
        assert!(doc.undo());
        assert_eq!(doc.text().as_str(), "");
    }

    #[test]
    fn undo_redo_delete_restores_text() {
        let mut doc = Doc::new("hello");
        doc.delete(1..4);
        assert_eq!(doc.text().as_str(), "ho");
        assert!(doc.undo());
        assert_eq!(doc.text().as_str(), "hello");
        assert!(doc.redo());
        assert_eq!(doc.text().as_str(), "ho");
    }

    #[test]
    fn revision_bumps_on_every_mutation() {
        let mut doc = Doc::new("hello");
        assert_eq!(doc.revision(), 0);
        doc.insert(5, "!");
        let after_insert = doc.revision();
        assert!(after_insert > 0);
        doc.delete(0..1);
        assert!(doc.revision() > after_insert);
        let before_undo = doc.revision();
        assert!(doc.undo());
        assert!(doc.revision() > before_undo);
    }

    #[test]
    fn trailing_newline_keeps_empty_last_line() {
        // Rope line tables must agree with the old backend: `a\nbb\n` is
        // three lines, and out-of-range rows read empty.
        let doc = Doc::new("a\nbb\n");
        assert_eq!(doc.line_count(), 3);
        assert_eq!(doc.line_text(2).as_str(), "");
        assert_eq!(doc.line_text(9).as_str(), "");
        assert_eq!(doc.char_at_line_col(9, 9), doc.len_chars());
    }

    #[test]
    fn out_of_range_edits_clamp() {
        let mut doc = Doc::new("hi");
        doc.insert(99, "!");
        assert_eq!(doc.text().as_str(), "hi!");
        doc.delete(5..50);
        assert_eq!(doc.text().as_str(), "hi!");
        assert!(!Doc::new("x").undo());
    }
}
