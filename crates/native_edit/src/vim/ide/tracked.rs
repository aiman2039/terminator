//! Buffer positions that follow later edits through the buffer's change
//! journal ([`Buffer::changes_since`]): a snippet's stops while Tab moves
//! between them, and carets across edits made outside the engine.
use super::super::super::doc::{Buffer, Change};
use super::super::carets::Caret;
use super::super::state::VimEngine;
use crate::lsp::Snippet;

/// A char range `start..end` of the buffer.
pub(super) type Span = (usize, usize);

/// The stops of one inserted snippet while Tab moves between them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SnippetSession {
    /// Numbered stops in Tab order (`$1`, `$2`, …), each the ranges of
    /// its copies in text order. A stop deleted with the text around it
    /// (nested in a replaced placeholder) loses its range.
    pub(super) stops: Vec<Vec<Span>>,
    /// Where the caret goes after the last stop: `$0`, else the end.
    pub(super) last: Span,
    /// The whole snippet.
    pub(super) bounds: Span,
    /// The stop Tab selected last.
    pub(super) active: usize,
    /// The buffer revision the ranges are for.
    pub(super) revision: u64,
    /// vim Insert mode: the selected stop still reads as selected while
    /// the buffer stays at this revision and the caret at this offset.
    pub(super) fresh: Option<(u64, usize)>,
    /// The copies of the selected stop hold extra carets.
    pub(super) mirrored: bool,
}

impl SnippetSession {
    /// The stops of `snippet`, inserted at char `at`; `None` without a
    /// numbered stop.
    pub(super) fn new(snippet: &Snippet, at: usize, revision: u64) -> Option<Self> {
        let shift = |start: usize, end: usize| (at.saturating_add(start), at.saturating_add(end));
        let mut indices: Vec<u32> = snippet
            .stops
            .iter()
            .map(|stop| stop.index)
            .filter(|index| *index > 0)
            .collect();
        indices.sort_unstable();
        indices.dedup();
        if indices.is_empty() {
            return None;
        }
        let stops = indices
            .iter()
            .map(|index| {
                let copies = snippet.stops.iter().filter(|stop| stop.index == *index);
                let mut ranges: Vec<Span> =
                    copies.map(|stop| shift(stop.start, stop.end)).collect();
                ranges.sort_unstable();
                ranges.dedup();
                ranges
            })
            .collect();
        let end = at.saturating_add(snippet.text.chars().count());
        let last = snippet
            .final_stop()
            .map_or((end, end), |stop| shift(stop.start, stop.end));
        Some(Self {
            stops,
            last,
            bounds: (at, end),
            active: 0,
            revision,
            fresh: None,
            mirrored: false,
        })
    }

    /// Follow the edits since the last look. False when they are not
    /// known any more, or one deleted the whole snippet: the session is
    /// over.
    pub(super) fn follow<B: Buffer>(&mut self, doc: &B) -> bool {
        let revision = doc.revision();
        if revision == self.revision {
            return true;
        }
        let Some(changes) = doc.changes_since(self.revision) else {
            return false;
        };
        self.revision = revision;
        changes.into_iter().all(|change| self.apply(change))
    }

    fn apply(&mut self, change: Change) -> bool {
        let Change {
            at,
            removed,
            inserted,
        } = change;
        if removed > 0 && !self.delete((at, at.saturating_add(removed))) {
            return false;
        }
        if inserted > 0 {
            self.insert(at, inserted);
        }
        true
    }

    /// Text `deleted` went away. False when it took the whole snippet
    /// (not just the selected stop typed over).
    fn delete(&mut self, deleted: Span) -> bool {
        let (start, end) = self.bounds;
        let typed_over = self.active_ranges().contains(&deleted);
        if start < end && deleted.0 <= start && end <= deleted.1 && !typed_over {
            return false;
        }
        let moved = |(start, end): Span| (after_delete(start, deleted), after_delete(end, deleted));
        for ranges in &mut self.stops {
            ranges.retain(|span| !swallowed(*span, deleted));
            for span in ranges.iter_mut() {
                *span = moved(*span);
            }
        }
        self.last = moved(self.last);
        self.bounds = moved(self.bounds);
        true
    }

    /// `len` chars went in at `at`. The selected stop and the snippet
    /// take text typed at their edges; the other stops do not.
    fn insert(&mut self, at: usize, len: usize) {
        let active = self.active;
        for (k, ranges) in self.stops.iter_mut().enumerate() {
            for span in ranges.iter_mut() {
                *span = after_insert(*span, (at, len), k == active);
            }
        }
        self.last = after_insert(self.last, (at, len), false);
        self.bounds = after_insert(self.bounds, (at, len), true);
    }

    /// The next (`forward`) or previous stop that still has a range.
    pub(super) fn step(&self, forward: bool) -> Option<usize> {
        let has = |k: &usize| self.stops.get(*k).is_some_and(|ranges| !ranges.is_empty());
        if forward {
            (self.active.saturating_add(1)..self.stops.len()).find(has)
        } else {
            (0..self.active).rev().find(has)
        }
    }

    /// The copies of the selected stop.
    pub(super) fn active_ranges(&self) -> &[Span] {
        self.stops.get(self.active).map_or(&[], Vec::as_slice)
    }

    /// Whether char `offset` is inside the snippet (edges included).
    pub(super) fn holds(&self, offset: usize) -> bool {
        self.bounds.0 <= offset && offset <= self.bounds.1
    }
}

/// `pos` after the text `deleted` went away.
fn after_delete(pos: usize, (at, end): Span) -> usize {
    if pos <= at {
        pos
    } else {
        pos.saturating_sub(end.saturating_sub(at)).max(at)
    }
}

/// A range the deletion of `at..end` removed: one inside it that is not
/// the deleted text itself, such as a stop nested in a replaced
/// placeholder. An empty range on an edge stays.
fn swallowed((start, end): Span, (at, stop): Span) -> bool {
    if start == end {
        at < start && start < stop
    } else {
        at <= start && end <= stop && (start, end) != (at, stop)
    }
}

/// `span` after `len` chars went in at `at`; `grow` takes text typed at
/// its edges in.
fn after_insert((start, end): Span, (at, len): (usize, usize), grow: bool) -> Span {
    if at < start || (at == start && !grow) {
        (start.saturating_add(len), end.saturating_add(len))
    } else if at < end || (at == end && grow) {
        (start, end.saturating_add(len))
    } else {
        (start, end)
    }
}

/// Every caret as char offsets at one buffer revision
/// ([`VimEngine::caret_offsets`]).
#[derive(Clone, Debug)]
pub struct CaretOffsets {
    revision: u64,
    /// Each caret with its cursor and anchor offsets, the primary first.
    carets: Vec<(Caret, usize, usize)>,
}

impl VimEngine {
    /// Note where every caret is, so [`Self::follow_carets`] can keep
    /// them on their text across edits made outside the engine.
    #[must_use]
    pub fn caret_offsets<B: Buffer>(&self, doc: &B) -> CaretOffsets {
        let offset = |at: super::super::mode::Cursor| doc.char_at_line_col(at.line, at.col);
        CaretOffsets {
            revision: doc.revision(),
            carets: self
                .carets()
                .into_iter()
                .map(|caret| (caret, offset(caret.cursor), offset(caret.anchor)))
                .collect(),
        }
    }

    /// Move the carets noted in `before` through the edits since then:
    /// an edit before a caret (or at it) shifts it, a deleted caret goes
    /// to the deletion. Selections and extra carets stay, and so does a
    /// vim snippet stop that read as selected. Without those edits on
    /// record the carets stay as they are.
    pub fn follow_carets<B: Buffer>(&mut self, doc: &B, before: CaretOffsets) {
        let Some(changes) = doc.changes_since(before.revision) else {
            return;
        };
        let primary = before.carets.first().map(|(_, cursor, _)| *cursor);
        let was_fresh = self
            .snippet
            .as_ref()
            .and_then(|session| session.fresh)
            .is_some_and(|fresh| primary == Some(fresh.1) && fresh.0 == before.revision);
        let follow = |offset: usize| {
            changes.iter().fold(offset, |offset, change| {
                let offset = after_delete(
                    offset,
                    (change.at, change.at.saturating_add(change.removed)),
                );
                if change.inserted > 0 && change.at <= offset {
                    offset.saturating_add(change.inserted)
                } else {
                    offset
                }
            })
        };
        let mut carets = before
            .carets
            .into_iter()
            .map(|(caret, cursor, anchor)| Caret {
                cursor: Self::cursor_at(doc, follow(cursor)),
                anchor: Self::cursor_at(doc, follow(anchor)),
                ..caret
            });
        let Some(primary) = carets.next() else {
            return;
        };
        self.extra_carets = carets.collect();
        self.load_caret_state(primary);
        self.clamp(doc);
        if was_fresh {
            // The edits only moved the stop: typing still replaces it.
            let caret = self.char_idx(doc, self.cursor);
            if let Some(session) = self.snippet.as_mut() {
                session.fresh = Some((doc.revision(), caret));
            }
        }
    }

    /// Make `caret` the primary one.
    pub(super) fn load_caret_state(&mut self, caret: Caret) {
        self.mode = caret.mode;
        self.cursor = caret.cursor;
        self.anchor = caret.anchor;
        self.preferred_col = caret.preferred_col;
    }
}
