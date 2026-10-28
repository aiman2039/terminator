//! Bracket matching, caret word, and extend/shrink selection.
use super::super::super::doc::Buffer;
use super::super::mode::{Cursor, Mode};
use super::super::state::VimEngine;
use super::super::words::{WordClass, word_class};
use super::text::IdeText;

/// The bracket match scan is bounded so a huge unbalanced file costs at
/// most a few thousand lines per frame.
const MATCH_SCAN_LINES: usize = 2000;

impl VimEngine {
    /// The bracket at (or just before) the caret and its partner, for
    /// matching-brace highlighting. The scan is bounded so a huge
    /// unbalanced file costs at most a few thousand lines per frame.
    #[must_use]
    pub fn matching_bracket<B: Buffer>(&self, doc: &B) -> Option<(Cursor, Cursor)> {
        let Cursor { line, col } = self.cursor;
        let chars: Vec<char> = doc.line_text(line).chars().collect();
        let bracket = |c: char| matches!(c, '(' | ')' | '[' | ']' | '{' | '}');
        let at = [Some(col), col.checked_sub(1)]
            .into_iter()
            .flatten()
            .find(|i| chars.get(*i).copied().is_some_and(bracket))?;
        let open = *chars.get(at)?;
        let start = Cursor { line, col: at };
        if let Some(close) = IdeText::closer_for(open) {
            return Self::closer_forward(doc, start, (open, close)).map(|end| (start, end));
        }
        let close = open;
        let open = match close {
            ')' => '(',
            ']' => '[',
            _ => '{',
        };
        Self::opener_backward(doc, start, (open, close)).map(|begin| (begin, start))
    }

    /// The closer matching the opener at `start`, scanning forward.
    fn closer_forward<B: Buffer>(
        doc: &B,
        start: Cursor,
        (open, close): (char, char),
    ) -> Option<Cursor> {
        let mut depth = 0usize;
        let last = doc
            .line_count()
            .min(start.line.saturating_add(MATCH_SCAN_LINES));
        for l in start.line..last {
            let text: Vec<char> = doc.line_text(l).chars().collect();
            let from = if l == start.line { start.col } else { 0 };
            for (i, c) in text.iter().enumerate().skip(from) {
                if *c == open {
                    depth = depth.saturating_add(1);
                    continue;
                }
                if *c != close {
                    continue;
                }
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(Cursor { line: l, col: i });
                }
            }
        }
        None
    }

    /// The opener matching the closer at `start`, scanning backward.
    fn opener_backward<B: Buffer>(
        doc: &B,
        start: Cursor,
        (open, close): (char, char),
    ) -> Option<Cursor> {
        let mut depth = 0usize;
        let first = start.line.saturating_sub(MATCH_SCAN_LINES);
        for l in (first..=start.line).rev() {
            let text: Vec<char> = doc.line_text(l).chars().collect();
            let upto = if l == start.line {
                start.col.saturating_add(1)
            } else {
                text.len()
            };
            for i in (0..upto.min(text.len())).rev() {
                let c = text.get(i).copied();
                if c == Some(close) {
                    depth = depth.saturating_add(1);
                    continue;
                }
                if c != Some(open) {
                    continue;
                }
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(Cursor { line: l, col: i });
                }
            }
        }
        None
    }

    /// Current selection (or caret) as a flat char range, end-exclusive.
    pub(super) fn selection_chars<B: Buffer>(&self, doc: &B) -> (usize, usize) {
        use super::super::mode::ModalEngine;
        if self.selection().is_some() {
            self.selection_range(doc, self.mode == Mode::VisualLine)
        } else {
            let at = self.char_idx(doc, self.cursor);
            (at, at)
        }
    }

    pub(super) fn select_chars<B: Buffer>(&mut self, doc: &mut B, from: usize, to: usize) {
        self.clear_pending();
        doc.end_run();
        if self.mode == Mode::Insert {
            self.enter_normal(doc);
        }
        let (line, col) = doc.line_col_at(from);
        self.anchor = Cursor { line, col };
        // Vim selections include the cursor cell; modeless ones end at it.
        let end = if self.modeless {
            to
        } else {
            to.saturating_sub(1).max(from)
        };
        let (line, col) = doc.line_col_at(end);
        self.cursor = Cursor { line, col };
        self.mode = Mode::Visual;
        self.preferred_col = col;
        self.scroll_request = true;
    }

    /// Extend selection (`IntelliJ` Ctrl+W / Alt+Up): grow to the smallest
    /// enclosing unit: word, string contents, bracket contents, brackets,
    /// whole lines, … up to the whole buffer.
    pub fn ide_extend_selection<B: Buffer>(&mut self, doc: &mut B) {
        let text = super::super::words::flat_text(doc);
        let current = self.selection_chars(doc);
        if let Some((from, to)) = IdeText::extend_candidate(&text, current) {
            if self.extend_stack.last() != Some(&current) {
                self.extend_stack.push(current);
            }
            self.select_chars(doc, from, to);
        }
    }

    /// Shrink selection: back to what extend grew from.
    pub fn ide_shrink_selection<B: Buffer>(&mut self, doc: &mut B) {
        let current = self.selection_chars(doc);
        while let Some((from, to)) = self.extend_stack.pop() {
            if from >= current.0 && to <= current.1 && (from, to) != current {
                if from == to {
                    let (line, col) = doc.line_col_at(from);
                    self.mode = Mode::Normal;
                    self.cursor = Cursor { line, col };
                    self.anchor = self.cursor;
                    self.clamp(doc);
                } else {
                    self.select_chars(doc, from, to);
                }
                return;
            }
        }
    }

    /// Identifier under (or just before) the caret, for occurrence
    /// highlighting. Punctuation and blanks yield nothing.
    #[must_use]
    pub fn word_at_caret<B: Buffer>(&self, doc: &B) -> Option<String> {
        let chars: Vec<char> = doc.line_text(self.cursor.line).chars().collect();
        let col = self.cursor.col.min(chars.len());
        let on = |i: usize| {
            chars
                .get(i)
                .is_some_and(|c| word_class(*c) == WordClass::Word)
        };
        let hit = if on(col) {
            col
        } else {
            col.checked_sub(1).filter(|p| on(*p))?
        };
        let mut start = hit;
        while start > 0 && on(start.saturating_sub(1)) {
            start = start.saturating_sub(1);
        }
        let mut end = hit;
        while on(end) {
            end = end.saturating_add(1);
        }
        let word: String = chars.get(start..end)?.iter().collect();
        (!word.chars().all(|c| c.is_ascii_digit())).then_some(word)
    }
}
