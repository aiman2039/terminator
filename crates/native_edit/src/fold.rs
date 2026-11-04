//! Code folding: foldable regions (multi-line bracket blocks, and indent
//! blocks for indentation-scoped languages) and which ones are folded. A
//! folded region keeps its first line visible and hides the rest.

use crate::doc::Buffer;
use crate::highlight::Language;
use std::collections::BTreeSet;

/// Regions recompute on edits; `folded` holds the start lines the user
/// folded and survives edits that keep those regions.
#[derive(Default)]
pub struct Folds {
    revision: Option<u64>,
    /// Foldable `(first, last)` line ranges, by first line.
    ranges: Vec<(usize, usize)>,
    folded: BTreeSet<usize>,
    /// Line count at `revision`, to shift folds by the lines an edit added.
    lines: usize,
    /// Bumped on every change, so row layouts know to rebuild.
    generation: u64,
    /// Lines hidden by folds, as merged `(first, last)` spans sorted by
    /// first line (the folded region's own first line stays visible).
    hidden_spans: Vec<(usize, usize)>,
}

impl Folds {
    /// Recompute regions for a new buffer revision. A folded region
    /// follows lines added or removed above or inside it; folds whose
    /// region cannot be found again are dropped.
    pub fn ensure<B: Buffer>(&mut self, doc: &B, language: Language) {
        self.ensure_enabled(doc, language, true);
    }

    /// [`Folds::ensure`]; `enabled == false` (large-file mode) finds no
    /// regions at all and skips the scan.
    pub fn ensure_enabled<B: Buffer>(&mut self, doc: &B, language: Language, enabled: bool) {
        if self.revision == Some(doc.revision()) {
            return;
        }
        self.revision = Some(doc.revision());
        let lines = doc.line_count();
        let delta = lines.cast_signed().saturating_sub(self.lines.cast_signed());
        self.lines = lines;
        let ranges = if enabled {
            fold_ranges(doc, language)
        } else {
            Vec::new()
        };
        let folded: BTreeSet<usize> = self
            .folded
            .iter()
            .filter_map(|start| {
                let old = self.region_at(*start)?;
                moved_region(old, delta, &ranges)
            })
            .collect();
        if ranges != self.ranges || folded != self.folded {
            self.ranges = ranges;
            self.folded = folded;
            self.bump();
        }
    }

    /// Record a change: rebuild the hidden spans once, so per-line
    /// lookups stay a binary search instead of a scan over every fold.
    fn bump(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        let mut spans: Vec<(usize, usize)> = Vec::new();
        for start in &self.folded {
            let Some((first, last)) = self.region_at(*start) else {
                continue;
            };
            match spans.last_mut() {
                Some((_, end)) if first <= *end => *end = (*end).max(last),
                _ => spans.push((first, last)),
            }
        }
        self.hidden_spans = spans;
    }

    /// The merged hidden span whose first line is the last one before
    /// `line`, when it reaches `line`.
    fn span_hiding(&self, line: usize) -> Option<(usize, usize)> {
        let index = self
            .hidden_spans
            .partition_point(|(first, _)| *first < line)
            .checked_sub(1)?;
        self.hidden_spans
            .get(index)
            .copied()
            .filter(|(_, last)| line <= *last)
    }

    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The foldable region starting on `line`.
    #[must_use]
    pub fn region_at(&self, line: usize) -> Option<(usize, usize)> {
        let index = self
            .ranges
            .binary_search_by_key(&line, |(first, _)| *first)
            .ok()?;
        self.ranges.get(index).copied()
    }

    #[must_use]
    pub fn is_folded(&self, line: usize) -> bool {
        self.folded.contains(&line)
    }

    /// Whether a folded region hides `line`.
    #[must_use]
    pub fn hidden(&self, line: usize) -> bool {
        self.span_hiding(line).is_some()
    }

    /// Regions whose first line is above `line` and that still enclose
    /// it, outermost first (sticky scope headers).
    #[must_use]
    pub fn enclosing(&self, line: usize) -> Vec<(usize, usize)> {
        let mut found: Vec<(usize, usize)> = self
            .ranges
            .iter()
            .copied()
            .filter(|(first, last)| *first < line && line <= *last)
            .collect();
        found.sort_by_key(|(first, _)| *first);
        found
    }

    /// First line of the folded region hiding `line`, if one does.
    #[must_use]
    pub fn hiding(&self, line: usize) -> Option<usize> {
        self.span_hiding(line).map(|(first, _)| first)
    }

    pub fn toggle(&mut self, line: usize) {
        if self.region_at(line).is_none() {
            return;
        }
        if !self.folded.remove(&line) {
            self.folded.insert(line);
        }
        self.bump();
    }

    /// Fold the innermost region holding `line` that is still open.
    pub fn fold_at(&mut self, line: usize) {
        let target = self
            .ranges
            .iter()
            .filter(|(first, last)| *first <= line && line <= *last && !self.folded.contains(first))
            .min_by_key(|(first, last)| last.saturating_sub(*first))
            .map(|(first, _)| *first);
        if let Some(first) = target {
            self.folded.insert(first);
            self.bump();
        }
    }

    /// Unfold every folded region holding `line` (or starting on it).
    pub fn unfold_at(&mut self, line: usize) {
        let ranges = &self.ranges;
        let before = self.folded.len();
        self.folded.retain(|start| {
            !ranges
                .iter()
                .any(|(first, last)| first == start && *first <= line && line <= *last)
        });
        if before != self.folded.len() {
            self.bump();
        }
    }

    pub fn fold_all(&mut self) {
        self.folded = self.ranges.iter().map(|(first, _)| *first).collect();
        self.bump();
    }

    pub fn unfold_all(&mut self) {
        if !self.folded.is_empty() {
            self.folded.clear();
            self.bump();
        }
    }
}

/// Where the folded region `old` is after an edit that changed the line
/// count by `delta`: unchanged (edit below), grown or shrunk (edit
/// inside), or shifted (edit above). Falls back to any region on the
/// same first line.
fn moved_region(old: (usize, usize), delta: isize, ranges: &[(usize, usize)]) -> Option<usize> {
    let (first, last) = old;
    let shift = |line: usize| line.checked_add_signed(delta);
    let candidates = [
        Some((first, last)),
        shift(last).map(|last| (first, last)),
        shift(first).zip(shift(last)),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|candidate| ranges.contains(candidate))
        .map(|(first, _)| first)
        .or_else(|| ranges.iter().any(|(f, _)| *f == first).then_some(first))
}

/// Foldable regions: brackets opened on one line and closed on a later
/// one, or (for indentation-scoped languages) a line followed by deeper
/// indented lines. One region per first line: the longest.
#[must_use]
pub fn fold_ranges<B: Buffer>(doc: &B, language: Language) -> Vec<(usize, usize)> {
    let mut regions = Regions::default();
    if matches!(language, Language::Python | Language::Yaml) {
        regions.indent_regions(doc);
    } else {
        regions.bracket_regions(doc);
    }
    regions.found.into_iter().collect()
}

/// Regions found so far, by first line.
#[derive(Default)]
struct Regions {
    found: std::collections::BTreeMap<usize, usize>,
}

impl Regions {
    /// Keep the longest region per first line.
    fn record(&mut self, first: usize, last: usize) {
        if last > first {
            let end = self.found.entry(first).or_insert(last);
            *end = (*end).max(last);
        }
    }

    /// A line followed by deeper indented lines (blank lines included).
    /// One pass with a stack of open blocks: a line closes every open
    /// block at its depth or deeper, each ending on the last non-blank
    /// line before it.
    fn indent_regions<B: Buffer>(&mut self, doc: &B) {
        let mut open: Vec<(usize, usize)> = Vec::new();
        let mut last_text = 0usize;
        for line in 0..doc.line_count() {
            let Some(depth) = Self::indent(&doc.line_text(line)) else {
                continue;
            };
            while let Some(&(first, open_depth)) = open.last() {
                if open_depth < depth {
                    break;
                }
                open.pop();
                self.record(first, last_text);
            }
            open.push((line, depth));
            last_text = line;
        }
        for (first, _) in open {
            self.record(first, last_text);
        }
    }

    /// Leading whitespace of a non-blank line.
    fn indent(text: &str) -> Option<usize> {
        (!text.trim().is_empty()).then(|| text.chars().take_while(|c| c.is_whitespace()).count())
    }

    /// Brackets opened on one line and closed on a later one.
    fn bracket_regions<B: Buffer>(&mut self, doc: &B) {
        let mut scan = BracketScan::default();
        for line in 0..doc.line_count() {
            let text: Vec<char> = doc.line_text(line).chars().collect();
            scan.scan_line(self, line, &text);
        }
    }
}

/// Open brackets across lines. Brackets inside strings and comments do
/// not open or close blocks.
#[derive(Default)]
struct BracketScan {
    stack: Vec<(char, usize)>,
    lexer: BracketLexer,
}

/// Finds bracket characters outside `"` strings, `//` comments and
/// `/* */` comments. Block comments continue across lines.
#[derive(Default)]
pub(crate) struct BracketLexer {
    block_comment: bool,
}

impl BracketLexer {
    /// Calls `found(column, bracket)` for each `{[()]}` in code on this line.
    pub(crate) fn line(&mut self, text: &[char], mut found: impl FnMut(usize, char)) {
        let mut in_string = false;
        let mut index = 0usize;
        while let Some(&c) = text.get(index) {
            let column = index;
            let next = text.get(index.saturating_add(1)).copied();
            index = index.saturating_add(1);
            if self.block_comment {
                if c == '*' && next == Some('/') {
                    self.block_comment = false;
                    index = index.saturating_add(1);
                }
                continue;
            }
            if in_string {
                match c {
                    '\\' => index = index.saturating_add(1),
                    '"' => in_string = false,
                    _ => {}
                }
                continue;
            }
            match (c, next) {
                ('/', Some('/')) => break,
                ('/', Some('*')) => {
                    self.block_comment = true;
                    index = index.saturating_add(1);
                    continue;
                }
                ('"', _) => {
                    in_string = true;
                    continue;
                }
                ('{' | '[' | '(' | '}' | ']' | ')', _) => found(column, c),
                _ => {}
            }
        }
    }
}

impl BracketScan {
    fn scan_line(&mut self, regions: &mut Regions, line: usize, text: &[char]) {
        let mut brackets = Vec::new();
        self.lexer.line(text, |_, c| brackets.push(c));
        for c in brackets {
            self.bracket(regions, c, line);
        }
    }

    /// Open on `{[(`; close the nearest matching opener on `}])`.
    fn bracket(&mut self, regions: &mut Regions, c: char, line: usize) {
        let open = match c {
            '{' | '[' | '(' => {
                self.stack.push((c, line));
                return;
            }
            '}' => '{',
            ']' => '[',
            ')' => '(',
            _ => return,
        };
        if let Some(at) = self.stack.iter().rposition(|(kind, _)| *kind == open) {
            let (_, first) = self.stack.remove(at);
            self.stack.truncate(at);
            regions.record(first, line);
        }
    }
}

#[cfg(test)]
#[path = "fold_tests.rs"]
mod tests;
