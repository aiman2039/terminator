//! Undo history behind [`crate::doc::Doc`]: inverse ops, typing-run
//! coalescing, explicit units, and the span one undo or redo step changed.

use std::ops::Range;

/// What one undo or redo step changed, as char offsets in the text after
/// the step ran.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndoChange {
    /// One span over every edit of the step: the text it put in, or the
    /// empty spot where it took text out.
    pub range: Range<usize>,
    /// Caret and selection (`anchor`, `head`) noted before the edit that
    /// undo reverted ([`crate::doc::Buffer::note_selection`]). Always
    /// `None` after a redo: the text no longer matches it.
    pub selection: Option<(usize, usize)>,
}

#[derive(Clone, Debug)]
pub(crate) enum UndoOp {
    /// Insert `text` at `at` to undo (i.e. a delete happened).
    Insert { at: usize, text: String },
    /// Delete `len` chars at `at` to undo (i.e. an insert happened).
    Delete { at: usize, len: usize },
}

#[derive(Clone, Debug, Default)]
pub(crate) struct EditRun {
    pub(crate) op: Option<UndoOp>,
}

impl EditRun {
    /// Merge `op` into the open run when it directly continues it:
    /// contiguous typing appends, contiguous forward deletes append, and
    /// contiguous backward deletes prepend (with the stored position moved).
    pub(crate) fn push(&mut self, op: UndoOp) -> bool {
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
pub(crate) enum UndoEntry {
    Op(UndoOp),
    Group(Vec<UndoOp>),
}

/// An undo or redo stack slot: the entry, plus the caret and selection
/// noted before the original edit. Undo and redo carry it along.
#[derive(Clone, Debug)]
pub(crate) struct UndoStep {
    pub(crate) entry: UndoEntry,
    pub(crate) before: Option<(usize, usize)>,
}

/// The span an undo or redo step changed, grown one executed op at a
/// time. Each op is the inverse that execution returned, so it tells what
/// happened: `Delete` means text went in, `Insert` means text came out.
#[derive(Default)]
pub(crate) struct ChangeSpan(Option<Range<usize>>);

impl ChangeSpan {
    pub(crate) fn add(&mut self, done: &UndoOp) {
        let (at, inserted, removed) = match done {
            UndoOp::Delete { at, len } => (*at, *len, 0),
            UndoOp::Insert { at, text } => (*at, 0, text.chars().count()),
        };
        // The span so far follows this edit: text after it shifts by the
        // length change, and removed text collapses onto `at`.
        let follow = |pos: usize| {
            if pos <= at {
                pos
            } else if pos >= at.saturating_add(removed) {
                pos.saturating_sub(removed).saturating_add(inserted)
            } else {
                at
            }
        };
        let end = at.saturating_add(inserted);
        self.0 = Some(match self.0.take() {
            None => at..end,
            Some(span) => follow(span.start).min(at)..follow(span.end).max(end),
        });
    }

    pub(crate) fn range(self) -> Range<usize> {
        self.0.unwrap_or_default()
    }
}
