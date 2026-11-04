//! Bracket matching (`%`) and the bracket, quote and tag text objects.
use super::super::super::doc::Buffer;
use super::super::state::VimEngine;
use super::super::words::CharSource;
use super::stream::Stream;
use super::{Count, MKind, Motion};

/// Bracket pairs for `%`, the bracket objects, and IDE pair typing.
pub(crate) const PAIRS: [(char, char); 3] = [('(', ')'), ('[', ']'), ('{', '}')];

/// `inner` limited to chars `lo..hi`: scans stop at the window edges.
pub(crate) struct Window<S> {
    pub(crate) inner: S,
    pub(crate) lo: usize,
    pub(crate) hi: usize,
}

impl<S: CharSource> CharSource for Window<S> {
    fn at(&mut self, i: usize) -> Option<char> {
        (self.lo..self.hi)
            .contains(&i)
            .then(|| self.inner.at(i))
            .flatten()
    }
}

/// The partner of `idx` (an open or close bracket), honoring nesting.
pub(crate) fn partner<S: CharSource>(s: &mut S, idx: usize) -> Option<usize> {
    let c = s.at(idx)?;
    if let Some((open, close)) = PAIRS.iter().find(|(open, _)| *open == c) {
        return close_after(s, idx.saturating_add(1), *open, *close);
    }
    let (open, close) = PAIRS.iter().find(|(_, close)| *close == c)?;
    open_before(s, idx, *open, *close)
}

/// The close bracket matching an open one just before `from`.
fn close_after<S: CharSource>(s: &mut S, from: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = from;
    while let Some(c) = s.at(i) {
        if c == open {
            depth = depth.saturating_add(1);
        } else if c == close {
            if depth == 0 {
                return Some(i);
            }
            depth = depth.saturating_sub(1);
        }
        i = i.saturating_add(1);
    }
    None
}

/// The unmatched open bracket before `before` (exclusive).
fn open_before<S: CharSource>(s: &mut S, before: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = before;
    while let Some(prev) = i.checked_sub(1) {
        i = prev;
        let c = s.at(i)?;
        if c == close {
            depth = depth.saturating_add(1);
        } else if c == open {
            if depth == 0 {
                return Some(i);
            }
            depth = depth.saturating_sub(1);
        }
    }
    None
}

/// The `count`th bracket pair around `idx` (a bracket under `idx` is its
/// own pair): open and close char indices.
pub(crate) fn enclosing<S: CharSource>(
    s: &mut S,
    idx: usize,
    (open, close): (char, char),
    count: usize,
) -> Option<(usize, usize)> {
    let mut first = match s.at(idx) {
        Some(c) if c == open => idx,
        Some(c) if c == close => open_before(s, idx, open, close)?,
        _ => open_before(s, idx, open, close)?,
    };
    for _ in 1..count.max(1) {
        first = open_before(s, first, open, close)?;
    }
    let last = close_after(s, first.saturating_add(1), open, close)?;
    Some((first, last))
}

/// The pair a text-object char names (`b`, `(`, `)` → parens, …).
pub(crate) fn pair_for(c: char) -> Option<(char, char)> {
    Some(match c {
        'b' | '(' | ')' => ('(', ')'),
        'B' | '{' | '}' => ('{', '}'),
        '[' | ']' => ('[', ']'),
        '<' | '>' => ('<', '>'),
        _ => return None,
    })
}

/// Quote object on one line: the quoted span around (or after) `col`, as
/// (open quote, close quote) columns. Backslash-escaped quotes are text.
pub(crate) fn quote_span(line: &[char], col: usize, quote: char) -> Option<(usize, usize)> {
    let quotes: Vec<usize> = (0..line.len())
        .filter(|i| {
            line.get(*i) == Some(&quote)
                && !i
                    .checked_sub(1)
                    .is_some_and(|prev| line.get(prev) == Some(&'\\'))
        })
        .collect();
    // Pair quotes left to right; the cursor's pair, else the next one.
    let pairs: Vec<(usize, usize)> = quotes
        .chunks(2)
        .filter_map(|pair| match pair {
            [a, b] => Some((*a, *b)),
            _ => None,
        })
        .collect();
    pairs
        .iter()
        .find(|(a, b)| *a <= col && col <= *b)
        .or_else(|| pairs.iter().find(|(a, _)| *a > col))
        .copied()
}

impl VimEngine {
    /// `%`: the partner of the first bracket at or after the cursor on
    /// its line; `N%` goes to `N` percent of the file.
    pub(super) fn match_pair_motion<B: Buffer>(&self, doc: &B, count: Count) -> Option<Motion> {
        if count.typed {
            let lines = Self::last_addressable(doc).saturating_add(1);
            let line = count.n.min(100).saturating_mul(lines).saturating_add(99) / 100;
            let line = line.saturating_sub(1);
            let col = Self::first_non_blank(doc, line);
            return Some(Motion::new(super::Cursor { line, col }, MKind::Linewise).jump());
        }
        let text: Vec<char> = doc.line_text(self.cursor.line).chars().collect();
        let col = (self.cursor.col..text.len()).find(|i| {
            text.get(*i)
                .is_some_and(|c| PAIRS.iter().any(|(o, cl)| o == c || cl == c))
        })?;
        let mut s = Stream::new(doc);
        let idx = doc.char_at_line_col(self.cursor.line, col);
        let target = partner(&mut s, idx)?;
        Some(Motion::new(Self::cursor_at(doc, target), MKind::Inclusive).jump())
    }
}
