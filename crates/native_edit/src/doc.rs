//! Native editing core: document buffer plus undo.
//!
//! [`Doc`] is backed by a `ropey` rope. Every consumer programs against the
//! [`Buffer`] trait (`char` indexing, cheap line access), so the backing
//! store stays private. Undo uses typing-run coalescing.

use crate::indent::IndentStyle;
pub use crate::undo::UndoChange;
use crate::undo::{ChangeSpan, EditRun, UndoEntry, UndoOp, UndoStep};
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};

/// Process-wide revision source. Revisions are unique across documents, so
/// a buffer reloaded from disk never reuses a revision that a cache keyed
/// on the previous buffer still holds.
static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);

fn next_revision() -> u64 {
    NEXT_REVISION.fetch_add(1, Ordering::Relaxed)
}

pub use crate::journal::Change;
use crate::journal::Journal;

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
    /// Undo one step and report what it changed; `None` when there is
    /// nothing to undo.
    fn undo_change(&mut self) -> Option<UndoChange>;
    /// Redo one step and report what it changed; `None` when there is
    /// nothing to redo.
    fn redo_change(&mut self) -> Option<UndoChange>;
    fn undo(&mut self) -> bool {
        self.undo_change().is_some()
    }
    fn redo(&mut self) -> bool {
        self.redo_change().is_some()
    }
    /// Note the caret and selection (`anchor`, `head` char offsets) just
    /// before an edit: the next new undo step keeps it, and undo reports
    /// it back. `None` drops the note. Backends may keep nothing.
    fn note_selection(&mut self, _selection: Option<(usize, usize)>) {}
    /// Monotonic edit counter: every text mutation (insert, delete,
    /// undo, redo) bumps it. Views use it to invalidate cached
    /// derivations (highlighting) without rehashing the buffer.
    /// Backends without a counter report `0` (always re-derive).
    fn revision(&self) -> u64 {
        0
    }
    /// The edits since `revision`, oldest first, so positions taken then
    /// can follow them. `None` when the buffer no longer knows them.
    fn changes_since(&self, _revision: u64) -> Option<Vec<Change>> {
        None
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
    /// How this text indents: typing, indent commands, tab stops and
    /// indent guides follow it. Backends without a setting use four
    /// spaces.
    fn indent_style(&self) -> IndentStyle {
        IndentStyle::default()
    }
}

/// Rope-backed [`Buffer`]: `ropey` owns storage and line indexing, so
/// edits stay cheap on large files. Char indices are the unit everywhere,
/// exactly as before the migration.
#[derive(Clone, Debug)]
pub struct Doc {
    rope: ropey::Rope,
    revision: u64,
    undo: Vec<UndoStep>,
    redo: Vec<UndoStep>,
    run: EditRun,
    group_depth: usize,
    group: Vec<UndoOp>,
    journal: Journal,
    /// Caret and selection noted for the next new undo step.
    noted: Option<(usize, usize)>,
    /// The open unit's note, taken when the unit opened.
    group_before: Option<(usize, usize)>,
    /// Detected from the text at creation; hosts may set it.
    indent: IndentStyle,
}

impl Default for Doc {
    fn default() -> Self {
        Self::new("")
    }
}

impl Doc {
    /// A buffer holding `text`, indenting the way the text already does
    /// (four spaces when nothing is indented).
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let indent = IndentStyle::detect(text.lines()).unwrap_or_default();
        Self {
            rope: ropey::Rope::from(text),
            revision: next_revision(),
            undo: Vec::new(),
            redo: Vec::new(),
            run: EditRun::default(),
            group_depth: 0,
            group: Vec::new(),
            journal: Journal::default(),
            noted: None,
            group_before: None,
            indent,
        }
    }

    /// Indent with `style` from now on (`.editorconfig`, language rules).
    pub fn set_indent_style(&mut self, style: IndentStyle) {
        self.indent = style;
    }

    /// Whether an undone step waits to be redone (any new edit drops it).
    #[must_use]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// An undo unit is open (a vim Insert session): edits now join it.
    #[must_use]
    pub fn undo_group_open(&self) -> bool {
        self.group_depth > 0
    }

    #[must_use]
    pub fn text(&self) -> String {
        self.rope.to_string()
    }

    /// A new revision for `change`, which the journal keeps.
    fn bump(&mut self, change: Change) {
        self.journal.record(self.revision, change);
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
        // Only a new step keeps the note; a merge or a unit drops it.
        let before = self.noted.take();
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
                && let Some(UndoStep {
                    entry: UndoEntry::Op(slot),
                    ..
                }) = self.undo.get_mut(last)
            {
                *slot = stored;
            }
        } else {
            self.undo.push(UndoStep {
                entry: UndoEntry::Op(op),
                before,
            });
        }
        self.redo.clear();
    }

    /// Bank the edits of the open unit as one entry. Undo and redo call
    /// this first, so the open unit never holds positions that a replay
    /// made stale; edits after it start a new entry in the same unit.
    fn flush_group(&mut self) {
        let before = self.group_before.take();
        if !self.group.is_empty() {
            self.redo.clear();
            self.undo.push(UndoStep {
                entry: UndoEntry::Group(std::mem::take(&mut self.group)),
                before,
            });
        }
    }

    /// Execute one undo entry (single op or whole group), returning the
    /// entry that reverses it and the span it changed. Groups replay in
    /// reverse chronological order, so the banked group redoes forward.
    fn execute_entry(&mut self, entry: UndoEntry) -> (UndoEntry, Range<usize>) {
        let mut span = ChangeSpan::default();
        let inverse = match entry {
            UndoEntry::Op(op) => {
                let inverse = self.execute(op);
                span.add(&inverse);
                UndoEntry::Op(inverse)
            }
            UndoEntry::Group(ops) => {
                let mut inverses = Vec::with_capacity(ops.len());
                for op in ops.into_iter().rev() {
                    let inverse = self.execute(op);
                    span.add(&inverse);
                    inverses.push(inverse);
                }
                UndoEntry::Group(inverses)
            }
        };
        (inverse, span.range())
    }

    /// Run one stack step: what it changed, and the step that reverses
    /// it (keeping the note from before the original edit).
    fn replay(&mut self, step: UndoStep) -> (UndoChange, UndoStep) {
        let UndoStep { entry, before } = step;
        let (entry, range) = self.execute_entry(entry);
        let change = UndoChange {
            range,
            selection: before,
        };
        (change, UndoStep { entry, before })
    }

    /// Execute an inverse op, returning the inverse that reverses it.
    fn execute(&mut self, op: UndoOp) -> UndoOp {
        match op {
            UndoOp::Insert { at, text } => {
                let at = at.min(self.len_chars());
                let len = text.chars().count();
                self.rope.insert(at, &text);
                self.bump(Change::insert(at, len));
                UndoOp::Delete { at, len }
            }
            UndoOp::Delete { at, len } => {
                let end = at
                    .checked_add(len)
                    .map_or(self.len_chars(), |sum| sum.min(self.len_chars()));
                let removed = self.rope.slice(at.min(end)..end).to_string();
                self.rope.remove(at.min(end)..end);
                self.bump(Change::delete(at.min(end), end.saturating_sub(at.min(end))));
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
        self.undo.push(inverse);
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

#[cfg(test)]
#[path = "doc_tests.rs"]
mod tests;
