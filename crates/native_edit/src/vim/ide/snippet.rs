//! Snippet stops after a completion insert, like `IntelliJ` live
//! templates: the first stop is selected, Tab and Shift+Tab move between
//! the stops, and `$0` (else the snippet end) comes last. Every copy of
//! a stop gets a caret. Enter finishes; Escape, a caret outside the
//! snippet, or (vim) leaving Insert mode ends the session. Modeless
//! editing selects a stop; vim Insert mode puts the caret at its end and
//! lets the first typed key replace it.
use super::super::super::doc::Buffer;
use super::super::carets::Caret;
use super::super::mode::{Cursor, Mode};
use super::super::state::VimEngine;
use super::tracked::{SnippetSession, Span};
use crate::lsp::Snippet;

/// A stop range for the view to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnippetMark {
    pub start: Cursor,
    pub end: Cursor,
    /// A copy of the stop Tab selected.
    pub active: bool,
    /// Draw it as selected: vim Insert mode has no selection of its own.
    pub selected: bool,
}

impl VimEngine {
    /// Place the caret after `snippet` was inserted at char `at`. With a
    /// numbered stop (and, in vim, in Insert mode) a session starts on
    /// the first stop. Otherwise the caret goes to [`Snippet::caret`],
    /// and a session already running goes on.
    pub fn start_snippet<B: Buffer>(&mut self, doc: &mut B, at: usize, snippet: &Snippet) {
        let editing = self.modeless || self.mode == Mode::Insert;
        let session = SnippetSession::new(snippet, at, doc.revision()).filter(|_| editing);
        if let Some(session) = session {
            self.end_snippet();
            self.snippet = Some(session);
            self.select_stop(doc, 0);
        } else {
            let (line, col) = doc.line_col_at(at.saturating_add(snippet.caret()));
            let _ = self.place_cursor(doc, line, col);
        }
    }

    /// A snippet session is running.
    #[must_use]
    pub fn snippet_active(&self) -> bool {
        self.snippet.is_some()
    }

    /// Tab (`forward`) or Shift+Tab: select the next or the previous
    /// stop. Past the last stop the snippet finishes; before the first,
    /// nothing moves. False without a session: the key keeps its usual
    /// meaning.
    pub fn snippet_jump<B: Buffer>(&mut self, doc: &mut B, forward: bool) -> bool {
        self.snippet_upkeep(doc);
        let Some(session) = self.snippet.as_ref() else {
            return false;
        };
        match session.step(forward) {
            Some(k) => self.select_stop(doc, k),
            None if forward => {
                self.finish_snippet(doc);
            }
            None => {}
        }
        true
    }

    /// Enter, or Tab on the last stop: the caret goes to `$0` (else the
    /// snippet end) and the session ends. False without a session.
    pub fn finish_snippet<B: Buffer>(&mut self, doc: &mut B) -> bool {
        self.snippet_upkeep(doc);
        let Some(last) = self.snippet.as_ref().map(|session| session.last) else {
            return false;
        };
        self.end_snippet();
        self.put_carets(doc, &[last]);
        true
    }

    /// End the session (Escape). Carets it put on copies go away.
    pub fn end_snippet(&mut self) {
        if self.snippet.take().is_some_and(|session| session.mirrored) {
            self.extra_carets.clear();
        }
    }

    /// Follow the edits since the last look, and end the session when
    /// its text is gone, the caret left it, or vim left Insert mode.
    pub fn snippet_upkeep<B: Buffer>(&mut self, doc: &B) {
        if self.snippet.is_none() {
            return;
        }
        let caret = self.char_idx(doc, self.cursor);
        let editing = if self.modeless {
            self.mode != Mode::Command
        } else {
            self.mode == Mode::Insert
        };
        let Some(session) = self.snippet.as_mut() else {
            return;
        };
        if !(session.follow(doc) && editing && session.holds(caret)) {
            self.end_snippet();
            return;
        }
        if session.fresh != Some((doc.revision(), caret)) {
            session.fresh = None;
        }
    }

    /// vim Insert mode: delete the stop Tab selected, at every copy, so
    /// the typed text replaces it. False (nothing deleted) when it does
    /// not read as selected any more.
    pub fn snippet_take_selection<B: Buffer>(&mut self, doc: &mut B) -> bool {
        self.snippet_upkeep(doc);
        let Some(session) = self
            .snippet
            .as_mut()
            .filter(|session| session.fresh.is_some())
        else {
            return false;
        };
        session.fresh = None;
        let ranges = session.active_ranges().to_vec();
        doc.begin_undo_group();
        for (start, end) in ranges.iter().rev() {
            doc.delete(*start..*end);
        }
        doc.end_undo_group();
        // The carets still name the old text: follow the stops instead.
        let emptied = self.snippet.as_mut().and_then(|session| {
            session
                .follow(doc)
                .then(|| session.active_ranges().to_vec())
        });
        match emptied {
            Some(emptied) => self.put_carets(doc, &emptied),
            None => self.end_snippet(),
        }
        true
    }

    /// The stop ranges to draw; empty ones are left out.
    #[must_use]
    pub fn snippet_marks<B: Buffer>(&self, doc: &B) -> Vec<SnippetMark> {
        let Some(session) = self
            .snippet
            .as_ref()
            .filter(|session| session.revision == doc.revision())
        else {
            return Vec::new();
        };
        let at = |offset: usize| Self::cursor_at(doc, offset);
        let mut marks = Vec::new();
        for (k, ranges) in session.stops.iter().enumerate() {
            let active = k == session.active;
            for (start, end) in ranges.iter().filter(|(start, end)| start < end) {
                marks.push(SnippetMark {
                    start: at(*start),
                    end: at(*end),
                    active,
                    selected: active && session.fresh.is_some(),
                });
            }
        }
        marks
    }

    /// Select stop `k`: a caret on each copy, the first one primary.
    fn select_stop<B: Buffer>(&mut self, doc: &mut B, k: usize) {
        let Some(session) = self.snippet.as_mut() else {
            return;
        };
        session.active = k;
        let ranges = session.active_ranges().to_vec();
        self.put_carets(doc, &ranges);
        let selected = !self.modeless && ranges.first().is_some_and(|(start, end)| start < end);
        let fresh = selected.then(|| (doc.revision(), self.char_idx(doc, self.cursor)));
        if let Some(session) = self.snippet.as_mut() {
            session.mirrored = ranges.len() > 1;
            session.fresh = fresh;
        }
    }

    /// One caret per range, the first primary: modeless editing selects
    /// each range, vim Insert mode sits at its end.
    fn put_carets<B: Buffer>(&mut self, doc: &mut B, ranges: &[Span]) {
        doc.end_run();
        self.clear_pending();
        let selecting = self.modeless && ranges.first().is_some_and(|(start, end)| start < end);
        if selecting && self.mode == Mode::Insert {
            // A selection ends the modeless insert, as Shift+arrows do.
            self.enter_normal(doc);
        }
        let carets: Vec<Caret> = ranges
            .iter()
            .map(|span| self.stop_caret(doc, *span))
            .collect();
        let mut carets = carets.into_iter();
        let Some(primary) = carets.next() else {
            return;
        };
        self.extra_carets = carets.collect();
        self.load_caret_state(primary);
        if !self.modeless {
            // Text typed after a jump is not one run of the Insert session.
            self.typed_exact = false;
        }
        self.clamp(doc);
    }

    /// The caret for one copy of a stop.
    fn stop_caret<B: Buffer>(&self, doc: &B, (start, end): Span) -> Caret {
        let cursor = Self::cursor_at(doc, end);
        let (mode, anchor) = if self.modeless && start < end {
            (Mode::Visual, Self::cursor_at(doc, start))
        } else if self.modeless && self.mode != Mode::Insert {
            (Mode::Normal, cursor)
        } else {
            (Mode::Insert, cursor)
        };
        Caret {
            mode,
            cursor,
            anchor,
            preferred_col: cursor.col,
        }
    }
}
