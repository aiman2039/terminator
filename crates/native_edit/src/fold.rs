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
    /// Bumped on every change, so row layouts know to rebuild.
    generation: u64,
}

impl Folds {
    /// Recompute regions for a new buffer revision, dropping folds whose
    /// region no longer starts on the same line.
    pub fn ensure<B: Buffer>(&mut self, doc: &B, language: Language) {
        if self.revision == Some(doc.revision()) {
            return;
        }
        self.revision = Some(doc.revision());
        let ranges = fold_ranges(doc, language);
        let before = self.folded.len();
        self.folded
            .retain(|start| ranges.iter().any(|(first, _)| first == start));
        if ranges != self.ranges || before != self.folded.len() {
            self.ranges = ranges;
            self.bump();
        }
    }

    fn bump(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The foldable region starting on `line`.
    #[must_use]
    pub fn region_at(&self, line: usize) -> Option<(usize, usize)> {
        self.ranges
            .iter()
            .copied()
            .find(|(first, _)| *first == line)
    }

    #[must_use]
    pub fn is_folded(&self, line: usize) -> bool {
        self.folded.contains(&line)
    }

    /// Whether a folded region hides `line`.
    #[must_use]
    pub fn hidden(&self, line: usize) -> bool {
        self.folded.iter().any(|start| {
            self.region_at(*start)
                .is_some_and(|(first, last)| first < line && line <= last)
        })
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
        self.folded.iter().copied().find(|start| {
            self.region_at(*start)
                .is_some_and(|(first, last)| first < line && line <= last)
        })
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

/// Foldable regions: brackets opened on one line and closed on a later
/// one, or (for indentation-scoped languages) a line followed by deeper
/// indented lines. One region per first line: the longest.
#[must_use]
pub fn fold_ranges<B: Buffer>(doc: &B, language: Language) -> Vec<(usize, usize)> {
    let lines = doc.line_count();
    let mut found: std::collections::BTreeMap<usize, usize> = std::collections::BTreeMap::new();
    let mut record = |first: usize, last: usize| {
        if last > first {
            let end = found.entry(first).or_insert(last);
            *end = (*end).max(last);
        }
    };
    if matches!(language, Language::Python | Language::Yaml) {
        let indent = |line: usize| -> Option<usize> {
            let text = doc.line_text(line);
            (!text.trim().is_empty())
                .then(|| text.chars().take_while(|c| c.is_whitespace()).count())
        };
        for line in 0..lines {
            let Some(depth) = indent(line) else {
                continue;
            };
            let mut last = line;
            let mut next = line.saturating_add(1);
            while next < lines {
                match indent(next) {
                    Some(inner) if inner <= depth => break,
                    Some(_) => last = next,
                    None => {}
                }
                next = next.saturating_add(1);
            }
            record(line, last);
        }
    } else {
        let mut stack: Vec<(char, usize)> = Vec::new();
        for line in 0..lines {
            for c in doc.line_text(line).chars() {
                match c {
                    '{' | '[' | '(' => stack.push((c, line)),
                    '}' | ']' | ')' => {
                        let open = match c {
                            '}' => '{',
                            ']' => '[',
                            _ => '(',
                        };
                        if let Some(at) = stack.iter().rposition(|(kind, _)| *kind == open) {
                            let (_, first) = stack.remove(at);
                            stack.truncate(at);
                            record(first, line);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    found.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Doc;

    #[test]
    fn bracket_regions_span_lines_and_fold_hides_their_body() {
        let doc = Doc::new("fn a() {\n    b(\n        1,\n    );\n}\nfn c() {}\n");
        let mut folds = Folds::default();
        folds.ensure(&doc, Language::Rust);
        assert_eq!(folds.region_at(0), Some((0, 4)));
        assert_eq!(folds.region_at(1), Some((1, 3)));
        assert_eq!(folds.region_at(5), None);
        folds.fold_at(2);
        assert!(folds.is_folded(1));
        assert!(folds.hidden(2) && folds.hidden(3) && !folds.hidden(1));
        folds.fold_at(2);
        assert!(folds.is_folded(0));
        folds.unfold_at(2);
        assert!(!folds.is_folded(0) && !folds.is_folded(1));
        folds.fold_all();
        assert!(folds.hidden(4));
        folds.unfold_all();
        assert!(!folds.hidden(4));
    }

    #[test]
    fn enclosing_lists_open_blocks_outermost_first() {
        let doc = Doc::new("impl S {\n    fn f() {\n        x();\n    }\n}\n");
        let mut folds = Folds::default();
        folds.ensure(&doc, Language::Rust);
        assert_eq!(folds.enclosing(2), [(0, 4), (1, 3)]);
        // A block's own first line is not "enclosed" by it.
        assert_eq!(folds.enclosing(1), [(0, 4)]);
        assert!(folds.enclosing(0).is_empty());
    }

    #[test]
    fn indentation_regions_for_python() {
        let doc = Doc::new("def f():\n    x = 1\n\n    return x\nprint(f())\n");
        assert_eq!(fold_ranges(&doc, Language::Python), [(0, 3)]);
    }

    #[test]
    fn folds_survive_edits_that_keep_their_region() {
        let mut doc = Doc::new("if a {\n    b\n}\n");
        let mut folds = Folds::default();
        folds.ensure(&doc, Language::Rust);
        folds.toggle(0);
        doc.insert(doc.len_chars(), "x\n");
        folds.ensure(&doc, Language::Rust);
        assert!(folds.is_folded(0));
        // Deleting the block's opener drops the fold with its region.
        doc.delete(0..6);
        folds.ensure(&doc, Language::Rust);
        assert!(!folds.is_folded(0));
    }
}
