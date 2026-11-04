//! Regex search: `/`, `?`, `n`, `N`, `*`, `#` over vim patterns, with
//! `ignorecase` / `smartcase` and `\c` / `\C`. Matches stay within a line.
mod pattern;

use super::super::doc::Buffer;
use super::mode::Cursor;
use super::motion::{MKind, Motion};
use super::state::VimEngine;
use super::words::{WordClass, word_class};
pub(crate) use pattern::compile;
use regex::Regex;

/// Char columns of every match start in `text`.
pub(crate) fn match_cols(re: &Regex, text: &str) -> Vec<(usize, usize)> {
    re.find_iter(text)
        .map(|m| {
            let col = text.get(..m.start()).map_or(0, |p| p.chars().count());
            (col, m.as_str().chars().count())
        })
        .collect()
}

impl VimEngine {
    /// Compile a vim pattern with the current case options.
    pub(crate) fn search_regex(&self, pattern: &str) -> Option<Regex> {
        compile(pattern, self.options.ignorecase, self.options.smartcase)
    }

    /// The first match after (or before) `from`, wrapping around the
    /// buffer, scanning line by line from the cursor.
    pub(crate) fn find_match<B: Buffer>(
        &self,
        doc: &B,
        re: &Regex,
        (from, forward): (Cursor, bool),
    ) -> Option<Cursor> {
        let lines = Self::last_addressable(doc).saturating_add(1);
        let start = from.line.min(lines.saturating_sub(1));
        for step in 0..=lines {
            let line = if forward {
                start.saturating_add(step).checked_rem(lines).unwrap_or(0)
            } else {
                start
                    .saturating_add(lines.saturating_mul(2))
                    .saturating_sub(step)
                    .checked_rem(lines)
                    .unwrap_or(0)
            };
            let cols = match_cols(re, &doc.line_text(line));
            let first_pass = step == 0;
            let wrapped = step == lines;
            let hit = if forward {
                cols.iter()
                    .map(|(col, _)| *col)
                    .find(|col| !first_pass || *col > from.col)
                    .filter(|col| !wrapped || *col <= from.col)
            } else {
                cols.iter()
                    .rev()
                    .map(|(col, _)| *col)
                    .find(|col| !first_pass || *col < from.col)
                    .filter(|col| !wrapped || *col >= from.col)
            };
            if let Some(col) = hit {
                return Some(Cursor { line, col });
            }
        }
        None
    }

    /// Run a search for `pattern` from the cursor (`/`, `?`): remembers
    /// the pattern and direction for `n`/`N`; `None` when nothing matches.
    pub(crate) fn search_from_cursor<B: Buffer>(
        &mut self,
        doc: &B,
        pattern: &str,
        backward: bool,
    ) -> Option<Cursor> {
        self.search_last = pattern.to_string();
        self.x.search_backward = backward;
        let re = self.search_regex(pattern)?;
        self.find_match(doc, &re, (self.cursor, !backward))
    }

    /// `n` / `N`: the `count`th match in the last search's direction
    /// (`reverse` flips it).
    pub(crate) fn search_motion<B: Buffer>(
        &mut self,
        doc: &B,
        reverse: bool,
        count: usize,
    ) -> Option<Motion> {
        if self.search_last.is_empty() {
            return None;
        }
        let re = self.search_regex(&self.search_last)?;
        let forward = self.x.search_backward == reverse;
        let mut at = self.cursor;
        for _ in 0..count.max(1) {
            at = self.find_match(doc, &re, (at, forward))?;
        }
        Some(Motion {
            target: at,
            kind: MKind::Exclusive,
            jump: true,
            keep_col: false,
        })
    }

    /// `*` / `#` (`whole`: as a whole word): search the keyword under or
    /// after the cursor.
    pub(crate) fn star_motion<B: Buffer>(
        &mut self,
        doc: &B,
        (forward, whole): (bool, bool),
        count: usize,
    ) -> Option<Motion> {
        let chars: Vec<char> = doc.line_text(self.cursor.line).chars().collect();
        let is_word = |i: usize| {
            chars
                .get(i)
                .is_some_and(|c| word_class(*c) == WordClass::Word)
        };
        let mut start = (self.cursor.col..chars.len()).find(|i| is_word(*i))?;
        while start > 0 && is_word(start.saturating_sub(1)) {
            start = start.saturating_sub(1);
        }
        let mut end = start;
        while is_word(end) {
            end = end.saturating_add(1);
        }
        let word: String = chars.get(start..end)?.iter().collect();
        let escaped = pattern::escape_vim(&word);
        let pattern = if whole {
            format!("\\<{escaped}\\>")
        } else {
            escaped
        };
        self.search_last = pattern;
        self.x.search_backward = !forward;
        // Start at the word itself, so `#` skips the word under the cursor.
        let saved = self.cursor;
        self.cursor.col = start;
        let motion = self.search_motion(doc, false, count);
        self.cursor = saved;
        motion
    }
}
