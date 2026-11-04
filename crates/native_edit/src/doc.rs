//! Native editing core: document buffer plus undo.
//!
//! [`Doc`] is backed by a `ropey` rope. Every consumer programs against the
//! [`Buffer`] trait (`char` indexing, cheap line access), so the backing
//! store stays private. Undo uses typing-run coalescing.

use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

/// Process-wide revision source. Revisions are unique across documents, so
/// a buffer reloaded from disk never reuses a revision that a cache keyed
/// on the previous buffer still holds.
static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);

fn next_revision() -> u64 {
    NEXT_REVISION.fetch_add(1, Ordering::Relaxed)
}

/// Char-indexed text buffer. Indices are Unicode scalar values, matching the
/// `ropey` convention, so callers never deal in bytes.
pub trait Buffer {
    fn len_chars(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len_chars() == 0
    }
    fn line_count(&self) -> usize;
    fn line_text(&self, line: usize) -> String;
    /// UTF-8 size of the text: the unit of the large-file cap.
    fn len_bytes(&self) -> usize {
        (0..self.line_count())
            .map(|line| self.line_text(line).len().saturating_add(1))
            .fold(0, usize::saturating_add)
    }
    /// The whole text with `\n` after every line (CRLF and lone CR read
    /// as LF), numbered like [`Buffer::line_text`]: the input of
    /// whole-text derivations such as highlighting.
    fn lf_text(&self) -> String {
        let mut text = String::new();
        for line in 0..self.line_count() {
            text.push_str(&self.line_text(line));
            text.push('\n');
        }
        text
    }
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
#[derive(Clone, Debug)]
pub struct Doc {
    rope: ropey::Rope,
    revision: u64,
    undo: Vec<UndoEntry>,
    redo: Vec<UndoEntry>,
    run: EditRun,
    group_depth: usize,
    group: Vec<UndoOp>,
}

impl Default for Doc {
    fn default() -> Self {
        Self::new("")
    }
}

impl Doc {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            rope: ropey::Rope::from(text.into()),
            revision: next_revision(),
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
        self.revision = next_revision();
    }

    /// Line `line` without its terminator (`\n`, `\r\n` or a lone `\r`,
    /// the breaks the rope counts).
    fn raw_line(&self, line: usize) -> String {
        if line >= self.rope.len_lines() {
            return String::new();
        }
        let slice = self.rope.line(line);
        let len = content_len(slice);
        slice.slice(..len).to_string()
    }

    /// Hash of the whole text, streamed from the rope chunks without
    /// copying it. Equals [`hash_text`] of [`Doc::text`].
    #[must_use]
    pub fn content_hash(&self) -> u64 {
        use std::hash::Hasher as _;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for chunk in self.rope.chunks() {
            hasher.write(chunk.as_bytes());
        }
        hasher.finish()
    }

    /// Hash of line `line` without its terminator, streamed from the
    /// rope: equals [`hash_text`] of [`Buffer::line_text`].
    #[must_use]
    pub fn line_hash(&self, line: usize) -> u64 {
        use std::hash::Hasher as _;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        if line < self.rope.len_lines() {
            let slice = self.rope.line(line);
            for chunk in slice.slice(..content_len(slice)).chunks() {
                hasher.write(chunk.as_bytes());
            }
        }
        hasher.finish()
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

    /// Bank the edits of the open unit as one entry. Undo and redo call
    /// this first, so the open unit never holds positions that a replay
    /// made stale; edits after it start a new entry in the same unit.
    fn flush_group(&mut self) {
        if !self.group.is_empty() {
            self.redo.clear();
            self.undo
                .push(UndoEntry::Group(std::mem::take(&mut self.group)));
        }
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

/// Chars of a rope line before its terminator: a `\r\n` pair counts as
/// one break, so a caret never lands between `\r` and `\n`.
fn content_len(slice: ropey::RopeSlice<'_>) -> usize {
    let total = slice.len_chars();
    let last = total.checked_sub(1).map(|at| slice.char(at));
    match last {
        Some('\n') => {
            let before = total.checked_sub(2).map(|at| slice.char(at));
            total.saturating_sub(if before == Some('\r') { 2 } else { 1 })
        }
        Some('\r') => total.saturating_sub(1),
        _ => total,
    }
}

/// Hash of `text`, matching [`Doc::content_hash`] of a buffer holding it.
#[must_use]
pub fn hash_text(text: &str) -> u64 {
    hash_bytes(text.as_bytes())
}

/// `\r\n` and lone `\r` as `\n` (the rope's `cr_lines` numbering);
/// borrows when there is no `\r`.
#[must_use]
pub fn normalize_breaks(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains('\r') {
        std::borrow::Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

/// The byte hash behind [`hash_text`].
#[must_use]
pub fn hash_bytes(bytes: &[u8]) -> u64 {
    use std::hash::Hasher as _;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write(bytes);
    hasher.finish()
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
        self.flush_group();
        let Some(entry) = self.undo.pop() else {
            return false;
        };
        let inverse = self.execute_entry(entry);
        self.redo.push(inverse);
        true
    }

    fn redo(&mut self) -> bool {
        self.end_run();
        self.flush_group();
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
        // Nested units (on-type formatting inside an insert session) join
        // the outer one: only the outermost end banks the entry.
        self.group_depth = self.group_depth.saturating_sub(1);
        self.end_run();
        if self.group_depth == 0 {
            self.flush_group();
        }
    }
}

#[cfg(test)]
#[path = "doc_tests.rs"]
mod tests;
