//! Multiple carets for modeless editing (`IntelliJ`): Alt+Shift+click
//! adds or removes a caret, and Add Selection for Next Occurrence selects
//! the next match of the selection. Every caret edit replays the single-
//! caret engine once per caret, bottom-up, so one caret's edit never moves
//! a caret that has not run yet; the ones already run are shifted by the
//! length change. One key across all carets is one undo step.

use super::mode::{Cursor, Effect, Mode};
use super::state::VimEngine;
use crate::doc::Buffer;

/// One caret: its mode (a selection is Visual), position, selection
/// anchor, and the column vertical moves aim for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caret {
    pub mode: Mode,
    pub cursor: Cursor,
    pub anchor: Cursor,
    pub preferred_col: usize,
}

impl Caret {
    /// The selected span `(start, end)`, end-exclusive, when selecting.
    #[must_use]
    pub fn selection(&self) -> Option<(Cursor, Cursor)> {
        if !self.mode.is_visual() {
            return None;
        }
        let (a, b) = (self.anchor, self.cursor);
        Some(if (a.line, a.col) <= (b.line, b.col) {
            (a, b)
        } else {
            (b, a)
        })
    }
}

/// A caret as char offsets while edits run (offsets shift cleanly).
#[derive(Clone, Copy)]
struct Spot {
    mode: Mode,
    cursor: usize,
    anchor: usize,
    preferred_col: usize,
}

impl Spot {
    fn of<B: Buffer>(doc: &B, caret: &Caret) -> Self {
        Self {
            mode: caret.mode,
            cursor: offset(doc, caret.cursor),
            anchor: offset(doc, caret.anchor),
            preferred_col: caret.preferred_col,
        }
    }

    fn caret<B: Buffer>(&self, doc: &B) -> Caret {
        Caret {
            mode: self.mode,
            cursor: cursor_at(doc, self.cursor),
            anchor: cursor_at(doc, self.anchor),
            preferred_col: self.preferred_col,
        }
    }

    /// Follow a buffer length change from `before` to `after` chars.
    fn shift(&mut self, before: usize, after: usize) {
        let shift = |at: usize| {
            if after >= before {
                at.saturating_add(after.saturating_sub(before))
            } else {
                at.saturating_sub(before.saturating_sub(after))
            }
        };
        self.cursor = shift(self.cursor);
        self.anchor = shift(self.anchor);
    }
}

fn offset<B: Buffer>(doc: &B, at: Cursor) -> usize {
    doc.char_at_line_col(at.line, at.col)
}

fn cursor_at<B: Buffer>(doc: &B, offset: usize) -> Cursor {
    let (line, col) = doc.line_col_at(offset.min(doc.len_chars()));
    Cursor { line, col }
}

/// The buffer as chars plus the selection to match, for scanning ahead.
struct OccurrenceSearch {
    text: Vec<char>,
    needle: Vec<char>,
    /// Start offsets that already hold a selection.
    taken: std::collections::HashSet<usize>,
    /// Where the next scan starts.
    next: usize,
    /// Positions already scanned (stops after one wrap).
    scanned: usize,
}

impl OccurrenceSearch {
    fn new<B: Buffer>(engine: &VimEngine, doc: &B, from: usize, to: usize) -> Option<Self> {
        if from >= to {
            return None;
        }
        let count = doc.line_count();
        let mut text: Vec<char> = Vec::with_capacity(doc.len_chars());
        for line in 0..count {
            text.extend(doc.line_text(line).chars());
            if line.saturating_add(1) < count {
                text.push('\n');
            }
        }
        let needle = text.get(from..to).unwrap_or_default().to_vec();
        if needle.is_empty() {
            return None;
        }
        let mut taken: std::collections::HashSet<usize> = engine
            .carets()
            .iter()
            .filter_map(Caret::selection)
            .map(|(start, _)| offset(doc, start))
            .collect();
        taken.insert(from);
        Some(Self {
            text,
            needle,
            taken,
            next: to,
            scanned: 0,
        })
    }

    /// The next untaken match after the last one, wrapping to the top once.
    fn next_hit(&mut self) -> Option<usize> {
        let total = self.text.len();
        let len = self.needle.len();
        while self.scanned < total {
            let at = self.next.checked_rem(total).unwrap_or(0);
            self.next = at.saturating_add(1);
            self.scanned = self.scanned.saturating_add(1);
            if !self.taken.contains(&at)
                && self.text.get(at..at.saturating_add(len)) == Some(self.needle.as_slice())
            {
                self.taken.insert(at);
                return Some(at);
            }
        }
        None
    }
}

impl VimEngine {
    fn current_caret(&self) -> Caret {
        Caret {
            mode: self.mode,
            cursor: self.cursor,
            anchor: self.anchor,
            preferred_col: self.preferred_col,
        }
    }

    fn load_caret(&mut self, caret: Caret) {
        self.mode = caret.mode;
        self.cursor = caret.cursor;
        self.anchor = caret.anchor;
        self.preferred_col = caret.preferred_col;
    }

    /// Every caret, the primary first.
    #[must_use]
    pub fn carets(&self) -> Vec<Caret> {
        std::iter::once(self.current_caret())
            .chain(self.extra_carets.iter().copied())
            .collect()
    }

    /// Carets besides the primary one.
    #[must_use]
    pub fn extra_carets(&self) -> &[Caret] {
        &self.extra_carets
    }

    /// Back to the primary caret alone (Escape, a plain click, undo).
    pub fn clear_extra_carets(&mut self) {
        self.extra_carets.clear();
    }

    /// Alt+Shift+click: add a caret at `line`/`col`, or remove the one
    /// already there (the primary moves to the newest when removed).
    pub fn toggle_caret_at<B: Buffer>(&mut self, doc: &B, line: usize, col: usize) {
        if !self.modeless {
            return;
        }
        let line = line.min(doc.line_count().saturating_sub(1));
        let col = col.min(doc.line_text(line).chars().count());
        let at = Cursor { line, col };
        if let Some(index) = self
            .extra_carets
            .iter()
            .position(|caret| caret.cursor == at)
        {
            self.extra_carets.remove(index);
            return;
        }
        if self.cursor == at {
            if let Some(next) = self.extra_carets.pop() {
                self.load_caret(next);
            }
            return;
        }
        let previous = self.current_caret();
        self.extra_carets.push(previous);
        if self.mode.is_visual() {
            self.mode = Mode::Normal;
        }
        self.cursor = at;
        self.anchor = at;
        self.preferred_col = col;
    }

    /// Alt+drag column selection: one caret on each line from `anchor` to
    /// `to`, each selecting the same columns (clamped to its line). The
    /// caret on `to`'s line is the primary one.
    pub fn column_select<B: Buffer>(&mut self, doc: &B, anchor: Cursor, to: Cursor) {
        if !self.modeless {
            return;
        }
        let last = doc.line_count().saturating_sub(1);
        let (from_line, to_line) = (anchor.line.min(last), to.line.min(last));
        let caret_on = |line: usize| {
            let len = doc.line_text(line).chars().count();
            let (start, end) = (anchor.col.min(len), to.col.min(len));
            Caret {
                mode: if start == end {
                    Mode::Normal
                } else {
                    Mode::Visual
                },
                anchor: Cursor { line, col: start },
                cursor: Cursor { line, col: end },
                preferred_col: to.col,
            }
        };
        self.extra_carets = (from_line.min(to_line)..=from_line.max(to_line))
            .filter(|line| *line != to_line)
            .map(caret_on)
            .collect();
        self.load_caret(caret_on(to_line));
    }

    /// Add Selection for Next Occurrence: with no selection, select the
    /// word at the caret; otherwise select the next match of the newest
    /// selection's text (wrapping) as a new caret. Returns false when
    /// there is nothing more to add.
    pub fn add_next_occurrence<B: Buffer>(&mut self, doc: &B) -> bool {
        if !self.modeless {
            return false;
        }
        let Some((start, end)) = self.current_caret().selection() else {
            let Some((from, to)) = self.word_bounds_at_caret(doc) else {
                return false;
            };
            self.mode = Mode::Visual;
            self.anchor = Cursor {
                line: self.cursor.line,
                col: from,
            };
            self.cursor = Cursor {
                line: self.cursor.line,
                col: to,
            };
            self.preferred_col = to;
            return true;
        };
        let Some(mut search) =
            OccurrenceSearch::new(self, doc, offset(doc, start), offset(doc, end))
        else {
            return false;
        };
        let Some(at) = search.next_hit() else {
            return false;
        };
        self.push_occurrence(doc, at, search.needle.len());
        self.request_scroll();
        true
    }

    /// Make the match at char offset `at` the new primary selection.
    fn push_occurrence<B: Buffer>(&mut self, doc: &B, at: usize, len: usize) {
        let previous = self.current_caret();
        self.extra_carets.push(previous);
        self.mode = Mode::Visual;
        self.anchor = cursor_at(doc, at);
        self.cursor = cursor_at(doc, at.saturating_add(len));
        self.preferred_col = self.cursor.col;
    }

    /// Unselect Occurrence: drop the newest caret (the primary), making
    /// the previous one primary again.
    pub fn remove_last_occurrence(&mut self) {
        if let Some(previous) = self.extra_carets.pop() {
            self.load_caret(previous);
        }
    }

    /// Select All Occurrences of the selection (or the word at the caret).
    pub fn add_all_occurrences<B: Buffer>(&mut self, doc: &B) {
        if self.current_caret().selection().is_none() && !self.add_next_occurrence(doc) {
            return;
        }
        let Some((start, end)) = self.current_caret().selection() else {
            return;
        };
        // One buffer copy and one forward scan for every match.
        let Some(mut search) =
            OccurrenceSearch::new(self, doc, offset(doc, start), offset(doc, end))
        else {
            return;
        };
        let len = search.needle.len();
        let mut added = false;
        while let Some(at) = search.next_hit() {
            self.push_occurrence(doc, at, len);
            added = true;
        }
        if added {
            self.request_scroll();
        }
    }

    /// The word around the caret as columns `(start, end)`.
    fn word_bounds_at_caret<B: Buffer>(&self, doc: &B) -> Option<(usize, usize)> {
        let word = self.word_at_caret(doc)?;
        let chars: Vec<char> = doc.line_text(self.cursor.line).chars().collect();
        let len = word.chars().count();
        let col = self.cursor.col.min(chars.len());
        // The word's start is the nearest match covering the caret.
        let word: Vec<char> = word.chars().collect();
        (col.saturating_sub(len)..=col)
            .find(|start| chars.get(*start..start.saturating_add(len)) == Some(word.as_slice()))
            .map(|start| (start, start.saturating_add(len)))
    }

    /// Selected text of every caret, top to bottom (multi-caret copy).
    #[must_use]
    pub fn caret_texts<B: Buffer>(&self, doc: &B) -> Vec<String> {
        let mut spans: Vec<(usize, usize)> = self
            .carets()
            .iter()
            .filter_map(Caret::selection)
            .map(|(start, end)| (offset(doc, start), offset(doc, end)))
            .collect();
        spans.sort_unstable();
        spans
            .into_iter()
            .map(|(from, to)| {
                let (start, end) = (cursor_at(doc, from), cursor_at(doc, to));
                (start.line..=end.line)
                    .map(|line| {
                        let chars: Vec<char> = doc.line_text(line).chars().collect();
                        let first = if line == start.line { start.col } else { 0 };
                        let last = if line == end.line {
                            end.col
                        } else {
                            chars.len()
                        };
                        chars
                            .get(first.min(last)..last.min(chars.len()))
                            .unwrap_or_default()
                            .iter()
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect()
    }

    /// Top-to-bottom rank of each caret, in the order
    /// [`Self::for_each_caret`] visits them (bottom-up).
    #[must_use]
    pub fn caret_order<B: Buffer>(&self, _doc: &B) -> Vec<usize> {
        (0..self.extra_carets.len().saturating_add(1))
            .rev()
            .collect()
    }

    /// Run `apply` once per caret (just once without extra carets): the
    /// carets bottom-up, each loaded as the engine's own, edits grouped
    /// into one undo step. Carets that end up on the same spot merge.
    /// A lone caret is noted first, so undo can put it back.
    pub fn for_each_caret<B: Buffer>(
        &mut self,
        doc: &mut B,
        mut apply: impl FnMut(&mut Self, &mut B) -> Vec<Effect>,
    ) -> Vec<Effect> {
        if self.extra_carets.is_empty() {
            self.note_selection(doc);
            let effects = apply(self, doc);
            doc.note_selection(None);
            return effects;
        }
        let mut spots: Vec<Spot> = self
            .carets()
            .iter()
            .map(|caret| Spot::of(doc, caret))
            .collect();
        let mut order: Vec<usize> = (0..spots.len()).collect();
        order.sort_by_key(|index| {
            std::cmp::Reverse(
                spots
                    .get(*index)
                    .map_or(0, |spot| spot.cursor.min(spot.anchor)),
            )
        });
        let mut effects = Vec::new();
        doc.begin_undo_group();
        let mut done: Vec<usize> = Vec::new();
        for index in order {
            let Some(spot) = spots.get(index).copied() else {
                continue;
            };
            self.load_caret(spot.caret(doc));
            let before = doc.len_chars();
            effects.extend(apply(self, doc));
            let after = doc.len_chars();
            // Carets already run sit after this one: shift them by the
            // length change (deletions shrink, insertions grow).
            for other in &done {
                if let Some(moved) = spots.get_mut(*other) {
                    moved.shift(before, after);
                }
            }
            let now = self.current_caret();
            if let Some(slot) = spots.get_mut(index) {
                *slot = Spot::of(doc, &now);
            }
            done.push(index);
        }
        doc.end_undo_group();
        self.load_merged_carets(doc, &spots);
        effects
    }

    /// Carets that ended up on the same spot merge; the first becomes the
    /// engine's own.
    fn load_merged_carets<B: Buffer>(&mut self, doc: &B, spots: &[Spot]) {
        let mut merged: Vec<Caret> = Vec::new();
        for spot in spots {
            let caret = spot.caret(doc);
            if !merged.iter().any(|kept| kept.cursor == caret.cursor) {
                merged.push(caret);
            }
        }
        let mut merged = merged.into_iter();
        if let Some(primary) = merged.next() {
            self.load_caret(primary);
        }
        self.extra_carets = merged.collect();
    }
}

#[cfg(test)]
#[path = "carets_tests.rs"]
mod tests;
