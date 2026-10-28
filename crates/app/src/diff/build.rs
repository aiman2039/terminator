//! Build a diff document from two texts: hunks, gaps and split rows.
use super::*;

#[cfg(test)]
pub(super) fn build(path: &Path, left: &str, right: &str, staged: bool) -> DiffDocument {
    build_with_context(BuildWithContextInput {
        path,
        left,
        right,
        staged,
        context: CONTEXT,
    })
}

pub(crate) struct BuildWithContextInput<'a> {
    pub(crate) path: &'a Path,
    pub(crate) left: &'a str,
    pub(crate) right: &'a str,
    pub(crate) staged: bool,
    pub(crate) context: usize,
}

pub(crate) fn build_with_context(input: BuildWithContextInput<'_>) -> DiffDocument {
    let BuildWithContextInput {
        path,
        left,
        right,
        staged,
        context,
    } = input;
    build_with_options(BuildWithOptionsInput {
        path,
        left,
        right,
        staged,
        view: &DiffView {
            context,
            ..DiffView::default()
        },
    })
}

/// `text` with every whitespace char removed from each line (line breaks
/// kept), so the line count, and every line index, matches the original.
fn strip_whitespace(text: &str) -> String {
    text.split_inclusive('\n')
        .map(|line| {
            let body = line.trim_end_matches(['\n', '\r']);
            let mut out: String = body.chars().filter(|c| !c.is_whitespace()).collect();
            if line.ends_with('\n') {
                out.push('\n');
            }
            out
        })
        .collect()
}

/// Original lines by index, for whitespace-insensitive diffs that compare
/// stripped text but must display what is really in the file.
struct Originals<'a> {
    old: Vec<&'a str>,
    new: Vec<&'a str>,
}

pub(crate) struct BuildWithOptionsInput<'a> {
    pub(crate) path: &'a Path,
    pub(crate) left: &'a str,
    pub(crate) right: &'a str,
    pub(crate) staged: bool,
    pub(crate) view: &'a DiffView,
}

pub(crate) fn build_with_options(input: BuildWithOptionsInput<'_>) -> DiffDocument {
    let BuildWithOptionsInput {
        path,
        left,
        right,
        staged,
        view,
    } = input;
    let old_hl = highlight_file(path, left);
    let new_hl = highlight_file(path, right);
    let (compare_left, compare_right) = if view.whitespace {
        (strip_whitespace(left), strip_whitespace(right))
    } else {
        (String::new(), String::new())
    };
    let originals = view.whitespace.then(|| Originals {
        old: left.split_inclusive('\n').collect(),
        new: right.split_inclusive('\n').collect(),
    });
    let diff = if view.whitespace {
        TextDiff::configure()
            .timeout(DIFF_TIMEOUT)
            .diff_lines(compare_left.as_str(), compare_right.as_str())
    } else {
        TextDiff::configure()
            .timeout(DIFF_TIMEOUT)
            .diff_lines(left, right)
    };
    let mut doc = DiffDocument {
        left_label: if staged { "HEAD" } else { "Index" }.into(),
        right_label: if staged { "Index" } else { "Working tree" }.into(),
        left_text: left.into(),
        right_text: right.into(),
        context: view.context,
        whitespace: view.whitespace,
        revealed: view.revealed.clone(),
        ..Default::default()
    };
    let sides = Sides {
        diff: &diff,
        old_hl: &old_hl,
        new_hl: &new_hl,
        originals: originals.as_ref(),
    };
    let (old_end, new_end) = sides.emit_groups(&mut doc, view.context);
    if !doc.hunks.is_empty() {
        sides.emit_gap(
            &mut doc,
            old_end..left.split_inclusive('\n').count(),
            new_end..right.split_inclusive('\n').count(),
        );
    }
    doc
}

/// The line diff and both sides' highlighting, shared by every row.
#[derive(Clone, Copy)]
struct Sides<'a, 'b> {
    diff: &'a TextDiff<'b, 'b, str>,
    old_hl: &'a [Vec<DiffSpan>],
    new_hl: &'a [Vec<DiffSpan>],
    originals: Option<&'a Originals<'b>>,
}

impl<'b> Sides<'_, 'b> {
    /// Emits each group of changes with its context and the gap before it.
    /// Returns where the last group ended on each side.
    fn emit_groups(self, doc: &mut DiffDocument, context: usize) -> (usize, usize) {
        let mut old_end = 0;
        let mut new_end = 0;
        for group in self.diff.grouped_ops(context) {
            if let Some(first) = group.first() {
                self.emit_gap(
                    doc,
                    old_end..first.old_range().start,
                    new_end..first.new_range().start,
                );
            }
            for op in &group {
                if op.tag() != DiffTag::Equal {
                    doc.hunks.push(review::Hunk {
                        old: op.old_range(),
                        new: op.new_range(),
                        unified_row: doc.unified.len(),
                        split_row: doc.split.len(),
                    });
                }
                self.emit_op(op, doc);
                old_end = op.old_range().end;
                new_end = op.new_range().end;
            }
        }
        (old_end, new_end)
    }

    /// Unchanged lines between changes: runs the user revealed are shown, and
    /// every remaining run collapses into its own expandable gap row.
    fn emit_gap(
        self,
        doc: &mut DiffDocument,
        old: std::ops::Range<usize>,
        new: std::ops::Range<usize>,
    ) {
        let mut start = old.start;
        while start < old.end {
            let shown = |line: usize| doc.revealed.iter().any(|range| range.contains(&line));
            let visible = shown(start);
            let mut end = start.saturating_add(1);
            while end < old.end && shown(end) == visible {
                end = end.saturating_add(1);
            }
            let len = end.saturating_sub(start);
            let new_start = new.start.saturating_add(start.saturating_sub(old.start));
            if visible {
                let op = similar::DiffOp::Equal {
                    old_index: start,
                    new_index: new_start,
                    len,
                };
                self.emit_op(&op, doc);
            } else {
                push_gap_row(doc, start..end, new_start..new_start.saturating_add(len));
            }
            start = end;
        }
    }

    fn emit_op(self, op: &similar::DiffOp, doc: &mut DiffDocument) {
        match op.tag() {
            DiffTag::Equal => self.emit_equal(op, doc),
            DiffTag::Delete => self.emit_one_side(op, doc, true),
            DiffTag::Insert => self.emit_one_side(op, doc, false),
            DiffTag::Replace => self.emit_replace(op, doc),
        }
    }

    fn emit_equal(self, op: &similar::DiffOp, doc: &mut DiffDocument) {
        for change in self.diff.iter_changes(op) {
            let line = equal_line(
                change,
                self.old_hl,
                self.originals.map(|o| o.old.as_slice()),
                true,
            );
            // Whitespace-insensitive equal lines may still differ in
            // whitespace: each side shows its own text.
            let right = match self.originals {
                Some(originals) => {
                    equal_line(change, self.new_hl, Some(originals.new.as_slice()), false)
                }
                None => line.clone(),
            };
            doc.unified.push(right.clone());
            doc.split.push(SplitRow {
                left: Some(line),
                right: Some(right),
            });
        }
    }

    /// A pure deletion (`old_side`) or insertion: rows on one side only.
    fn emit_one_side(self, op: &similar::DiffOp, doc: &mut DiffDocument, old_side: bool) {
        for change in self.diff.iter_inline_changes(op) {
            let line = self.change(change, old_side);
            doc.unified.push(line.clone());
            doc.split.push(if old_side {
                SplitRow {
                    left: Some(line),
                    right: None,
                }
            } else {
                SplitRow {
                    left: None,
                    right: Some(line),
                }
            });
        }
    }

    fn emit_replace(self, op: &similar::DiffOp, doc: &mut DiffDocument) {
        let mut deletes = Vec::new();
        let mut inserts = Vec::new();
        for change in self.diff.iter_inline_changes(op) {
            match change.tag() {
                ChangeTag::Delete => deletes.push(self.change(change, true)),
                ChangeTag::Insert => inserts.push(self.change(change, false)),
                ChangeTag::Equal => {}
            }
        }
        for line in &deletes {
            doc.unified.push(line.clone());
        }
        for line in &inserts {
            doc.unified.push(line.clone());
        }
        let rows = deletes.len().max(inserts.len());
        for i in 0..rows {
            doc.split.push(SplitRow {
                left: deletes.get(i).cloned(),
                right: inserts.get(i).cloned(),
            });
        }
    }

    /// A deleted (`old_side`) or inserted line.
    fn change(self, change: similar::InlineChange<'b, str>, old_side: bool) -> DiffLine {
        change_line(ChangeLineInput {
            change,
            kind: if old_side {
                LineKind::Delete
            } else {
                LineKind::Insert
            },
            highlighted: if old_side { self.old_hl } else { self.new_hl },
            old_side,
            originals: self.originals,
        })
    }
}

/// A collapsed run of unchanged lines, expandable from its row.
fn push_gap_row(doc: &mut DiffDocument, old: std::ops::Range<usize>, new: std::ops::Range<usize>) {
    let len = old.end.saturating_sub(old.start);
    let line = DiffLine {
        kind: LineKind::Hunk,
        old_no: None,
        new_no: None,
        spans: vec![DiffSpan {
            text: format!("⋯ {len} unchanged lines ⋯"),
            rgb: fallback_rgb(),
            intra: Intra::None,
        }],
    };
    doc.gaps.push(review::Gap {
        old,
        new,
        unified_row: doc.unified.len(),
        split_row: doc.split.len(),
    });
    doc.unified.push(line.clone());
    doc.split.push(SplitRow {
        left: Some(line.clone()),
        right: Some(line),
    });
}

fn equal_line(
    change: similar::Change<&str>,
    highlighted: &[Vec<DiffSpan>],
    originals: Option<&[&str]>,
    old_side: bool,
) -> DiffLine {
    let index = if old_side {
        change.old_index()
    } else {
        change.new_index()
    };
    let text = originals
        .zip(index)
        .and_then(|(lines, index)| lines.get(index).copied())
        .unwrap_or_else(|| change.value());
    DiffLine {
        kind: LineKind::Equal,
        old_no: change
            .old_index()
            .map(|n| u32::try_from(n).unwrap_or(u32::MAX).saturating_add(1)),
        new_no: change
            .new_index()
            .map(|n| u32::try_from(n).unwrap_or(u32::MAX).saturating_add(1)),
        spans: line_spans(highlighted, index, text),
    }
}

struct ChangeLineInput<'a, 'b> {
    change: similar::InlineChange<'b, str>,
    kind: LineKind,
    highlighted: &'a [Vec<DiffSpan>],
    old_side: bool,
    originals: Option<&'a Originals<'b>>,
}

fn change_line(input: ChangeLineInput<'_, '_>) -> DiffLine {
    let ChangeLineInput {
        change,
        kind,
        highlighted,
        old_side,
        originals,
    } = input;
    let side_index = if old_side {
        change.old_index()
    } else {
        change.new_index()
    };
    // Whitespace-insensitive: show the real line, without word marks that
    // would point into the stripped comparison text.
    if let Some(original) = originals.zip(side_index).and_then(|(originals, index)| {
        if old_side {
            originals.old.get(index)
        } else {
            originals.new.get(index)
        }
    }) {
        return DiffLine {
            kind,
            old_no: change
                .old_index()
                .map(|n| u32::try_from(n).unwrap_or(u32::MAX).saturating_add(1)),
            new_no: change
                .new_index()
                .map(|n| u32::try_from(n).unwrap_or(u32::MAX).saturating_add(1)),
            spans: line_spans(highlighted, side_index, original),
        };
    }
    let fragments: Vec<(bool, String)> = change
        .iter_strings_lossy()
        .map(|(changed, text)| (changed, text.trim_end_matches(['\n', '\r']).to_string()))
        .collect();
    let joined: String = fragments.iter().map(|(_, t)| t.as_str()).collect();
    let index = if old_side {
        change.old_index()
    } else {
        change.new_index()
    };
    let spans = overlay(line_spans(highlighted, index, &joined), &fragments);
    DiffLine {
        kind,
        old_no: change
            .old_index()
            .map(|n| u32::try_from(n).unwrap_or(u32::MAX).saturating_add(1)),
        new_no: change
            .new_index()
            .map(|n| u32::try_from(n).unwrap_or(u32::MAX).saturating_add(1)),
        spans,
    }
}
