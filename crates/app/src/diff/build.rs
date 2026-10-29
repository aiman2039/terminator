//! Build a diff document from two texts: hunks, gaps and split rows.
use super::*;

#[cfg(test)]
pub(super) struct BuildInput<'a> {
    pub(super) path: &'a Path,
    pub(super) left: &'a str,
    pub(super) right: &'a str,
    pub(super) staged: bool,
}

#[cfg(test)]
pub(super) fn build(input: BuildInput<'_>) -> DiffDocument {
    let BuildInput {
        path,
        left,
        right,
        staged,
    } = input;
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

/// Original lines by index: the diff compares normalized text when compare
/// options are on, but rows always display what is really in the file.
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
    let options = view.options;
    let old_hl = highlight_file(path, left);
    let new_hl = highlight_file(path, right);
    let (compare_left, compare_right) = if options.exact() {
        (String::new(), String::new())
    } else {
        (options.normalize(left), options.normalize(right))
    };
    let originals = Originals {
        old: left.split_inclusive('\n').collect(),
        new: right.split_inclusive('\n').collect(),
    };
    let diff = if options.exact() {
        TextDiff::configure()
            .timeout(DIFF_TIMEOUT)
            .diff_lines(left, right)
    } else {
        TextDiff::configure()
            .timeout(DIFF_TIMEOUT)
            .diff_lines(compare_left.as_str(), compare_right.as_str())
    };
    let mut doc = DiffDocument {
        left_label: if staged { "HEAD" } else { "Index" }.into(),
        right_label: if staged { "Index" } else { "Working tree" }.into(),
        left_text: left.into(),
        right_text: right.into(),
        context: view.context,
        options,
        revealed: view.revealed.clone(),
        ..Default::default()
    };
    let sides = Sides {
        diff: &diff,
        old_hl: &old_hl,
        new_hl: &new_hl,
        originals: &originals,
        options,
    };
    let (old_end, new_end) = sides.emit_groups(&mut doc, view.context);
    if !doc.hunks.is_empty() {
        sides.emit_gap(
            &mut doc,
            old_end..originals.old.len(),
            new_end..originals.new.len(),
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
    originals: &'a Originals<'b>,
    options: DiffOptions,
}

impl Sides<'_, '_> {
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
        let old = op.old_range();
        let new = op.new_range();
        if op.tag() != DiffTag::Equal {
            self.emit_change(old, new, doc);
            return;
        }
        for offset in 0..old.len() {
            // Equal under the compare options; each side still shows its
            // own text.
            let old_index = Some(old.start.saturating_add(offset));
            let new_index = Some(new.start.saturating_add(offset));
            let left = self.line(LineKind::Equal, (old_index, new_index), true);
            let right = self.line(LineKind::Equal, (old_index, new_index), false);
            doc.unified.push(right.clone());
            doc.split.push(SplitRow {
                left: Some(left),
                right: Some(right),
            });
        }
    }

    /// Removed lines, then added lines; split rows pair them by position,
    /// and each pair gets in-line change marks for the highlight mode.
    fn emit_change(
        self,
        old: std::ops::Range<usize>,
        new: std::ops::Range<usize>,
        doc: &mut DiffDocument,
    ) {
        let mut deletes: Vec<DiffLine> = old
            .clone()
            .map(|index| self.line(LineKind::Delete, (Some(index), None), true))
            .collect();
        let mut inserts: Vec<DiffLine> = new
            .clone()
            .map(|index| self.line(LineKind::Insert, (None, Some(index)), false))
            .collect();
        for ((delete, insert), (old_index, new_index)) in
            deletes.iter_mut().zip(inserts.iter_mut()).zip(old.zip(new))
        {
            let old_text = line_text(&self.originals.old, old_index);
            let new_text = line_text(&self.originals.new, new_index);
            if let Some(fragments) = self.options.intra(old_text, new_text) {
                delete.spans = overlay(std::mem::take(&mut delete.spans), &fragments.old);
                insert.spans = overlay(std::mem::take(&mut insert.spans), &fragments.new);
            }
        }
        doc.unified.extend(deletes.iter().cloned());
        doc.unified.extend(inserts.iter().cloned());
        let rows = deletes.len().max(inserts.len());
        for i in 0..rows {
            doc.split.push(SplitRow {
                left: deletes.get(i).cloned(),
                right: inserts.get(i).cloned(),
            });
        }
    }

    /// A row with the original text of one side's line (`old_side`).
    fn line(
        self,
        kind: LineKind,
        (old, new): (Option<usize>, Option<usize>),
        old_side: bool,
    ) -> DiffLine {
        let (lines, highlighted, index) = if old_side {
            (&self.originals.old, self.old_hl, old)
        } else {
            (&self.originals.new, self.new_hl, new)
        };
        let text = index.map_or("", |index| line_text(lines, index));
        let number = |index: Option<usize>| {
            index.map(|n| u32::try_from(n).unwrap_or(u32::MAX).saturating_add(1))
        };
        DiffLine {
            kind,
            old_no: number(old),
            new_no: number(new),
            spans: line_spans(highlighted, index, text),
        }
    }
}

/// One original line without its line break.
fn line_text<'a>(lines: &[&'a str], index: usize) -> &'a str {
    lines
        .get(index)
        .map_or("", |line| line.trim_end_matches(['\n', '\r']))
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
