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
    let originals = originals.as_ref();
    let diff = if view.whitespace {
        TextDiff::configure()
            .timeout(DIFF_TIMEOUT)
            .diff_lines(compare_left.as_str(), compare_right.as_str())
    } else {
        TextDiff::configure()
            .timeout(DIFF_TIMEOUT)
            .diff_lines(left, right)
    };
    let context = view.context;
    let mut doc = DiffDocument {
        left_label: if staged { "HEAD" } else { "Index" }.into(),
        right_label: if staged { "Index" } else { "Working tree" }.into(),
        left_text: left.into(),
        right_text: right.into(),
        context,
        whitespace: view.whitespace,
        revealed: view.revealed.clone(),
        ..Default::default()
    };
    let mut old_end = 0;
    let mut new_end = 0;
    for group in diff.grouped_ops(context) {
        if let Some(first) = group.first() {
            emit_gap(EmitGapInput {
                doc: &mut doc,
                diff: &diff,
                old: old_end..first.old_range().start,
                new: new_end..first.new_range().start,
                old_hl: &old_hl,
                new_hl: &new_hl,
                originals,
            });
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
            emit_op(EmitOpInput {
                diff: &diff,
                op,
                old_hl: &old_hl,
                new_hl: &new_hl,
                unified: &mut doc.unified,
                split: &mut doc.split,
                originals,
            });
            old_end = op.old_range().end;
            new_end = op.new_range().end;
        }
    }
    if !doc.hunks.is_empty() {
        emit_gap(EmitGapInput {
            doc: &mut doc,
            diff: &diff,
            old: old_end..left.split_inclusive('\n').count(),
            new: new_end..right.split_inclusive('\n').count(),
            old_hl: &old_hl,
            new_hl: &new_hl,
            originals,
        });
    }
    doc
}

/// Unchanged lines between changes: runs the user revealed are shown, and
/// every remaining run collapses into its own expandable gap row.
struct EmitGapInput<'a, 'b> {
    doc: &'a mut DiffDocument,
    diff: &'a TextDiff<'b, 'b, str>,
    old: std::ops::Range<usize>,
    new: std::ops::Range<usize>,
    old_hl: &'a [Vec<DiffSpan>],
    new_hl: &'a [Vec<DiffSpan>],
    originals: Option<&'a Originals<'b>>,
}

fn emit_gap(input: EmitGapInput<'_, '_>) {
    let EmitGapInput {
        doc,
        diff,
        old,
        new,
        old_hl,
        new_hl,
        originals,
    } = input;
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
            emit_op(EmitOpInput {
                diff,
                op: &op,
                old_hl,
                new_hl,
                unified: &mut doc.unified,
                split: &mut doc.split,
                originals,
            });
        } else {
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
                old: start..end,
                new: new_start..new_start.saturating_add(len),
                unified_row: doc.unified.len(),
                split_row: doc.split.len(),
            });
            doc.unified.push(line.clone());
            doc.split.push(SplitRow {
                left: Some(line.clone()),
                right: Some(line),
            });
        }
        start = end;
    }
}

struct EmitOpInput<'a, 'b> {
    diff: &'a TextDiff<'b, 'b, str>,
    op: &'a similar::DiffOp,
    old_hl: &'a [Vec<DiffSpan>],
    new_hl: &'a [Vec<DiffSpan>],
    unified: &'a mut Vec<DiffLine>,
    split: &'a mut Vec<SplitRow>,
    originals: Option<&'a Originals<'b>>,
}

fn emit_op(input: EmitOpInput<'_, '_>) {
    let EmitOpInput {
        diff,
        op,
        old_hl,
        new_hl,
        unified,
        split,
        originals,
    } = input;
    match op.tag() {
        DiffTag::Equal => {
            for change in diff.iter_changes(op) {
                let line = equal_line(change, old_hl, originals.map(|o| o.old.as_slice()), true);
                // Whitespace-insensitive equal lines may still differ in
                // whitespace: each side shows its own text.
                let right = match originals {
                    Some(originals) => {
                        equal_line(change, new_hl, Some(originals.new.as_slice()), false)
                    }
                    None => line.clone(),
                };
                unified.push(right.clone());
                split.push(SplitRow {
                    left: Some(line),
                    right: Some(right),
                });
            }
        }
        DiffTag::Delete => {
            for change in diff.iter_inline_changes(op) {
                let line = change_line(ChangeLineInput {
                    change,
                    kind: LineKind::Delete,
                    highlighted: old_hl,
                    old_side: true,
                    originals,
                });
                unified.push(line.clone());
                split.push(SplitRow {
                    left: Some(line),
                    right: None,
                });
            }
        }
        DiffTag::Insert => {
            for change in diff.iter_inline_changes(op) {
                let line = change_line(ChangeLineInput {
                    change,
                    kind: LineKind::Insert,
                    highlighted: new_hl,
                    old_side: false,
                    originals,
                });
                unified.push(line.clone());
                split.push(SplitRow {
                    left: None,
                    right: Some(line),
                });
            }
        }
        DiffTag::Replace => {
            let mut deletes = Vec::new();
            let mut inserts = Vec::new();
            for change in diff.iter_inline_changes(op) {
                match change.tag() {
                    ChangeTag::Delete => {
                        deletes.push(change_line(ChangeLineInput {
                            change,
                            kind: LineKind::Delete,
                            highlighted: old_hl,
                            old_side: true,
                            originals,
                        }));
                    }
                    ChangeTag::Insert => {
                        inserts.push(change_line(ChangeLineInput {
                            change,
                            kind: LineKind::Insert,
                            highlighted: new_hl,
                            old_side: false,
                            originals,
                        }));
                    }
                    ChangeTag::Equal => {}
                }
            }
            for line in &deletes {
                unified.push(line.clone());
            }
            for line in &inserts {
                unified.push(line.clone());
            }
            let rows = deletes.len().max(inserts.len());
            for i in 0..rows {
                split.push(SplitRow {
                    left: deletes.get(i).cloned(),
                    right: inserts.get(i).cloned(),
                });
            }
        }
    }
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
