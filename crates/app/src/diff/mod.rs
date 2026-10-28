//! Read-only Git snapshots rendered with similar + syntect. Never runs on the GUI thread.
use anyhow::{Context, Result, ensure};
use similar::{ChangeTag, DiffTag, TextDiff};
use std::{
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

pub(crate) struct ExpandGapInput<'a> {
    pub(crate) path: &'a Path,
    pub(crate) gap: usize,
    pub(crate) part: GapPart,
    pub(crate) staged: bool,
}

use syntect::{
    easy::HighlightLines,
    highlighting::Theme,
    parsing::{SyntaxReference, SyntaxSet},
    util::LinesWithEndings,
};
use terminator_core::CommandOptions;

pub(crate) mod actions;
pub(crate) mod review;

const CONTEXT: usize = 3;

/// Buffers above this size render plain, like the native editor's cap:
/// syntect over a multi-megabyte side would stall a CPU worker.
const MAX_HIGHLIGHT_BYTES: usize = 256 * 1024;

/// Wall-clock budget for one line diff; past it `similar` falls back to a
/// coarser (still correct) diff instead of stalling on pathological input.
const DIFF_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

/// Theme the next diff build highlights with. The GUI mirrors its
/// visuals here every frame; builds run on workers that have no `Ui`.
static DARK: AtomicBool = AtomicBool::new(true);

/// Record whether the GUI is in dark mode, for later diff builds.
pub(crate) fn set_dark(dark: bool) {
    DARK.store(dark, Ordering::Relaxed);
}

/// Identity of one built document. Views key derived caches (content
/// width) on it instead of hashing both full texts every frame. Every
/// value, the default included, is fresh.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DiffGeneration(u64);

impl Default for DiffGeneration {
    fn default() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

#[cfg(test)]
pub struct DiffRequest<'a> {
    pub cwd: &'a Path,
    pub path: &'a Path,
    pub staged: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Intra {
    None,
    Change,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LineKind {
    Hunk,
    Equal,
    Delete,
    Insert,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DiffSpan {
    pub text: String,
    pub rgb: [u8; 3],
    pub intra: Intra,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub spans: Vec<DiffSpan>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SplitRow {
    pub left: Option<DiffLine>,
    pub right: Option<DiffLine>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct DiffDocument {
    pub read_only: bool,
    pub left_label: String,
    pub right_label: String,
    pub left_text: String,
    pub right_text: String,
    pub unified: Vec<DiffLine>,
    pub split: Vec<SplitRow>,
    pub(crate) hunks: Vec<review::Hunk>,
    pub(crate) gaps: Vec<review::Gap>,
    pub(crate) revealed: Vec<std::ops::Range<usize>>,
    pub(crate) context: usize,
    /// Lines were compared ignoring whitespace (display keeps originals).
    pub(crate) whitespace: bool,
    pub(crate) source: Option<actions::Source>,
    pub(crate) generation: DiffGeneration,
}

/// Which part of a collapsed gap to reveal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GapPart {
    All,
    /// The first lines, continuing down from the change above.
    Top,
    /// The last lines, continuing up from the change below.
    Bottom,
}

/// Lines one partial gap expansion reveals (`IntelliJ` shows 20 more).
pub(crate) const GAP_STEP: usize = 20;

/// How a refreshed diff should look: the context size, the unchanged
/// ranges the user expanded, and the whitespace mode, so a refresh keeps
/// the view the user built up.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DiffView {
    pub context: usize,
    pub revealed: Vec<std::ops::Range<usize>>,
    pub whitespace: bool,
}

impl DiffDocument {
    pub(crate) fn change_at(&self, row: usize, split: bool) -> Option<&review::Hunk> {
        let index = self
            .hunks
            .partition_point(|hunk| hunk.row(split) <= row)
            .checked_sub(1)?;
        self.hunks
            .get(index)
            .filter(|hunk| hunk.rows(split).contains(&row))
    }

    pub(crate) fn expand_gap(&self, input: ExpandGapInput<'_>) -> Result<Self> {
        let ExpandGapInput {
            path,
            gap,
            part,
            staged,
        } = input;
        let gap = self
            .gaps
            .get(gap)
            .context("The hidden lines changed; refresh the diff")?;
        let range = match part {
            GapPart::All => gap.old.clone(),
            GapPart::Top => gap.old.start..gap.old.end.min(gap.old.start.saturating_add(GAP_STEP)),
            GapPart::Bottom => gap.old.start.max(gap.old.end.saturating_sub(GAP_STEP))..gap.old.end,
        };
        let mut revealed = self.revealed.clone();
        revealed.push(range);
        Ok(self.rebuild(
            path,
            staged,
            &DiffView {
                context: self.context,
                revealed,
                whitespace: self.whitespace,
            },
        ))
    }

    /// The same two texts laid out again under `view`, keeping the labels,
    /// source, and read-only flag of this document.
    pub(crate) fn rebuild(&self, path: &Path, staged: bool, view: &DiffView) -> Self {
        let mut doc = build_with_options(BuildWithOptionsInput {
            path,
            left: &self.left_text,
            right: &self.right_text,
            staged,
            view,
        });
        doc.read_only = self.read_only;
        doc.source.clone_from(&self.source);
        doc.left_label.clone_from(&self.left_label);
        doc.right_label.clone_from(&self.right_label);
        doc
    }
}

#[cfg(test)]
pub fn document(DiffRequest { cwd, path, staged }: DiffRequest<'_>) -> Result<DiffDocument> {
    let root = git_root(cwd)?;
    let relative = terminator_git::relative_path(&root, path)?;
    let options = CommandOptions {
        stdout_limit: 4 * 1024 * 1024,
        ..Default::default()
    };
    let (left, right) = terminator_git::snapshots(&root, &relative, staged, &options)?;
    let (left, right) = decode_sides(&left, &right)?;
    Ok(build(&relative, &left, &right, staged))
}

#[cfg(test)]
fn git_root(cwd: &Path) -> Result<PathBuf> {
    let options = CommandOptions {
        stdout_limit: 4 * 1024 * 1024,
        ..Default::default()
    };
    let bytes = terminator_git::run_blocking(cwd, &["rev-parse", "--show-toplevel"], options)?;
    let text = std::str::from_utf8(&bytes)?.trim();
    PathBuf::from(text)
        .canonicalize()
        .context("Resolve Git root")
}

/// Both sides as text, or an error naming the sizes when either side is
/// binary (or not UTF-8): a size change is all a text diff can say there.
fn decode_sides(left: &[u8], right: &[u8]) -> Result<(String, String)> {
    ensure!(
        terminator_git::check_text(left) && terminator_git::check_text(right),
        "Binary file · {} → {}. Binary or non-UTF-8 files cannot be reviewed natively.",
        human_size(left.len()),
        human_size(right.len())
    );
    Ok((
        String::from_utf8_lossy(left).into_owned(),
        String::from_utf8_lossy(right).into_owned(),
    ))
}

fn human_size(bytes: usize) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit.saturating_add(1) < UNITS.len() {
        value /= 1024.0;
        unit = unit.saturating_add(1);
    }
    let name = UNITS.get(unit).copied().unwrap_or("B");
    if unit == 0 {
        format!("{bytes} {name}")
    } else {
        format!("{value:.1} {name}")
    }
}

/// Left pane for the editable split view: one row per line of `right`
/// (the live buffer), so both panes share row positions. Equal and
/// replaced lines show the original beside their buffer line, inserted
/// lines get a blank filler, and deleted lines (which have no buffer row)
/// are returned as markers: `(row, lines)` deleted just above `row`.
pub(crate) struct AlignedLeft {
    pub doc: DiffDocument,
    pub deleted: Vec<(usize, Vec<String>)>,
}

pub(crate) fn align_left(left: &str, right: &str, left_spans: &[Vec<DiffSpan>]) -> AlignedLeft {
    let diff = TextDiff::configure()
        .timeout(DIFF_TIMEOUT)
        .diff_lines(left, right);
    let lines: Vec<&str> = left.split_inclusive('\n').collect();
    let number = |index: usize| Some(u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1));
    let row = |kind: LineKind, old: usize, new: usize| {
        let text = lines.get(old).copied().unwrap_or_default();
        SplitRow {
            left: Some(DiffLine {
                kind,
                old_no: number(old),
                new_no: number(new),
                spans: line_spans(left_spans, Some(old), text),
            }),
            right: None,
        }
    };
    let deleted_text = |range: std::ops::Range<usize>| -> Vec<String> {
        range
            .filter_map(|index| lines.get(index))
            .map(|line| line.trim_end_matches(['\n', '\r']).to_owned())
            .collect()
    };
    let mut out = AlignedLeft {
        doc: DiffDocument::default(),
        deleted: Vec::new(),
    };
    for op in diff.ops() {
        let (old, new) = (op.old_range(), op.new_range());
        match op.tag() {
            DiffTag::Equal => {
                for offset in 0..old.len() {
                    out.doc.split.push(row(
                        LineKind::Equal,
                        old.start.saturating_add(offset),
                        new.start.saturating_add(offset),
                    ));
                }
            }
            DiffTag::Insert => {
                for _ in new {
                    out.doc.split.push(SplitRow {
                        left: None,
                        right: None,
                    });
                }
            }
            DiffTag::Delete => out.deleted.push((new.start, deleted_text(old))),
            DiffTag::Replace => {
                let paired = old.len().min(new.len());
                for offset in 0..paired {
                    out.doc.split.push(row(
                        LineKind::Delete,
                        old.start.saturating_add(offset),
                        new.start.saturating_add(offset),
                    ));
                }
                for _ in paired..new.len() {
                    out.doc.split.push(SplitRow {
                        left: None,
                        right: None,
                    });
                }
                if old.len() > paired {
                    out.deleted.push((
                        new.start.saturating_add(paired),
                        deleted_text(old.start.saturating_add(paired)..old.end),
                    ));
                }
            }
        }
    }
    out
}

/// Syntax-highlighted spans per line, for the in-diff editor.
pub(crate) fn highlight_spans(path: &Path, text: &str) -> Vec<Vec<DiffSpan>> {
    highlight_file(path, text)
}

fn syntaxes() -> &'static SyntaxSet {
    terminator_native_edit::highlight::syntax_set()
}

fn theme() -> &'static Theme {
    static FALLBACK: OnceLock<Theme> = OnceLock::new();
    terminator_native_edit::highlight::theme(DARK.load(Ordering::Relaxed))
        .unwrap_or_else(|| FALLBACK.get_or_init(Theme::default))
}

fn highlight_file(path: &Path, text: &str) -> Vec<Vec<DiffSpan>> {
    if text.len() > MAX_HIGHLIGHT_BYTES {
        // Plain rows: `line_spans` falls back to the theme foreground.
        return Vec::new();
    }
    let set = syntaxes();
    let syntax = syntax_for(path, set);
    let mut highlighter = HighlightLines::new(syntax, theme());
    let fallback = fallback_rgb();
    let mut lines = Vec::new();
    for line in LinesWithEndings::from(text) {
        let ranges = highlighter
            .highlight_line(line, set)
            .unwrap_or_else(|_| vec![(syntect::highlighting::Style::default(), line)]);
        let mut spans = Vec::new();
        for (style, piece) in ranges {
            let piece = piece.trim_end_matches(['\n', '\r']);
            if piece.is_empty() {
                continue;
            }
            spans.push(DiffSpan {
                text: piece.to_owned(),
                rgb: [style.foreground.r, style.foreground.g, style.foreground.b],
                intra: Intra::None,
            });
        }
        if spans.is_empty() {
            spans.push(DiffSpan {
                text: String::new(),
                rgb: fallback,
                intra: Intra::None,
            });
        }
        lines.push(spans);
    }
    if text.is_empty() {
        lines.clear();
    }
    lines
}

fn syntax_for<'a>(path: &Path, _set: &'a SyntaxSet) -> &'a SyntaxReference {
    terminator_native_edit::highlight::syntax_for_path(&path.to_string_lossy())
}

fn fallback_rgb() -> [u8; 3] {
    let color = theme()
        .settings
        .foreground
        .unwrap_or(syntect::highlighting::Color {
            r: 192,
            g: 197,
            b: 206,
            a: 255,
        });
    [color.r, color.g, color.b]
}

fn line_spans(highlighted: &[Vec<DiffSpan>], index: Option<usize>, text: &str) -> Vec<DiffSpan> {
    let trimmed = text.trim_end_matches(['\n', '\r']);
    index
        .and_then(|i| highlighted.get(i).cloned())
        .filter(|spans| spans.iter().map(|s| s.text.as_str()).collect::<String>() == trimmed)
        .unwrap_or_else(|| {
            vec![DiffSpan {
                text: trimmed.to_owned(),
                rgb: fallback_rgb(),
                intra: Intra::None,
            }]
        })
}

fn overlay(spans: Vec<DiffSpan>, fragments: &[(bool, String)]) -> Vec<DiffSpan> {
    let joined: String = fragments.iter().map(|(_, t)| t.as_str()).collect();
    let original: String = spans.iter().map(|s| s.text.as_str()).collect();
    if joined != original {
        return spans;
    }
    let mut colors = Vec::with_capacity(original.len());
    for span in &spans {
        for _ in 0..span.text.len() {
            colors.push(span.rgb);
        }
    }
    let mut out = Vec::new();
    let mut offset = 0;
    for (changed, text) in fragments {
        if text.is_empty() {
            continue;
        }
        let rgb = colors.get(offset).copied().unwrap_or_else(fallback_rgb);
        out.push(DiffSpan {
            text: text.clone(),
            rgb,
            intra: if *changed { Intra::Change } else { Intra::None },
        });
        offset = offset.saturating_add(text.len());
    }
    if out.is_empty() { spans } else { out }
}

#[cfg(test)]
fn build(path: &Path, left: &str, right: &str, staged: bool) -> DiffDocument {
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
pub(crate) struct Originals<'a> {
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

pub struct DocumentAsyncInput<'a> {
    pub service: &'a crate::gui_services::Services,
    pub cwd: PathBuf,
    pub path: PathBuf,
    pub staged: bool,
    pub cancel: &'a terminator_core::async_service::CancellationToken,
}

pub async fn document_async(input: DocumentAsyncInput<'_>) -> Result<DiffDocument> {
    let DocumentAsyncInput {
        service,
        cwd,
        path,
        staged,
        cancel,
    } = input;
    document_with_context(DocumentWithContextInput {
        service,
        cwd,
        path,
        staged,
        context: CONTEXT,
        cancel,
    })
    .await
}

pub(crate) struct DocumentWithContextInput<'a> {
    pub(crate) service: &'a crate::gui_services::Services,
    pub(crate) cwd: PathBuf,
    pub(crate) path: PathBuf,
    pub(crate) staged: bool,
    pub(crate) context: usize,
    pub(crate) cancel: &'a terminator_core::async_service::CancellationToken,
}

pub(crate) async fn document_with_context(
    input: DocumentWithContextInput<'_>,
) -> Result<DiffDocument> {
    let DocumentWithContextInput {
        service,
        cwd,
        path,
        staged,
        context,
        cancel,
    } = input;
    let options = || CommandOptions {
        stdout_limit: 4 * 1024 * 1024,
        ..Default::default()
    };
    let root_bytes = terminator_git::run(
        service.processes(),
        service.fs(),
        terminator_git::RunInput {
            cwd: &cwd,
            args: vec!["rev-parse".into(), "--show-toplevel".into()],
            options: options(),
        },
    )
    .await;
    let root_bytes = match root_bytes {
        Ok(bytes) => bytes,
        Err(_) if !staged => {
            return crate::perforce::document(crate::perforce::DocumentInput {
                service,
                cwd,
                path,
                context,
                cancel,
            })
            .await;
        }
        Err(error) => return Err(error.into()),
    };
    let root = PathBuf::from(std::str::from_utf8(&root_bytes)?.trim());
    let (root, relative) = service
        .fs()
        .run(cancel, move || {
            let root = root.canonicalize().context("Resolve Git root")?;
            let relative = terminator_git::relative_path(&root, &path)?;
            anyhow::Ok((root, relative))
        })
        .await?;
    let mut args: Vec<std::ffi::OsString> = [
        "diff",
        "--raw",
        "-z",
        "--no-abbrev",
        "--no-ext-diff",
        "--no-textconv",
        "--find-renames",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    if staged {
        args.push("--cached".into());
    }
    let raw = terminator_git::run(
        service.processes(),
        service.fs(),
        terminator_git::RunInput {
            cwd: &root,
            args,
            options: options(),
        },
    )
    .await?;
    let oids = terminator_git::find_record(&raw, &relative)?;
    let (left, right) = if let Some((left, right)) = oids {
        let left = terminator_git::snapshot::blob_async(
            service.processes(),
            service.fs(),
            terminator_git::snapshot::BlobAsyncInput {
                root: &root,
                oid: &left,
                options: options(),
            },
        )
        .await?;
        let right = if staged {
            terminator_git::snapshot::blob_async(
                service.processes(),
                service.fs(),
                terminator_git::snapshot::BlobAsyncInput {
                    root: &root,
                    oid: &right,
                    options: options(),
                },
            )
            .await?
        } else {
            let path = root.join(&relative);
            service
                .fs()
                .run(cancel, move || {
                    terminator_git::worktree_file(&path).map_err(anyhow::Error::from)
                })
                .await?
        };
        (left, right)
    } else {
        let tracked = terminator_git::run(
            service.processes(),
            service.fs(),
            terminator_git::RunInput {
                cwd: &root,
                args: vec![
                    "ls-files".into(),
                    "--stage".into(),
                    "-z".into(),
                    "--".into(),
                    actions::literal(&relative),
                ],
                options: options(),
            },
        )
        .await?;
        let oid = tracked
            .split(|byte| *byte == b' ')
            .nth(1)
            .map(std::str::from_utf8)
            .transpose()?;
        let left = if let Some(oid) = oid {
            terminator_git::snapshot::blob_async(
                service.processes(),
                service.fs(),
                terminator_git::snapshot::BlobAsyncInput {
                    root: &root,
                    oid,
                    options: options(),
                },
            )
            .await?
        } else {
            Vec::new()
        };
        let right = if staged {
            left.clone()
        } else {
            let path = root.join(&relative);
            service
                .fs()
                .run(cancel, move || {
                    terminator_git::worktree_file(&path).map_err(anyhow::Error::from)
                })
                .await?
        };
        (left, right)
    };
    let source = actions::Source::capture(service, &root, &relative, cancel).await?;
    service
        .cpu()
        .run(cancel, move || {
            let (left, right) = decode_sides(&left, &right)?;
            let mut doc = build_with_context(BuildWithContextInput {
                path: &relative,
                left: &left,
                right: &right,
                staged,
                context,
            });
            doc.source = Some(source);
            Ok(doc)
        })
        .await
}
pub(crate) struct RemoteDocumentInput<'a> {
    pub(crate) service: &'a crate::gui_services::Services,
    pub(crate) client: &'a terminator_core::async_client::Client,
    pub(crate) tab: &'a crate::Tab,
    pub(crate) context: usize,
    pub(crate) cancel: &'a terminator_core::async_service::CancellationToken,
}

pub(crate) async fn remote_document(input: RemoteDocumentInput<'_>) -> Result<DiffDocument> {
    let RemoteDocumentInput {
        service,
        client,
        tab,
        context,
        cancel,
    } = input;
    use base64::Engine;
    let crate::Tab::Diff {
        remote: Some(project),
        cwd,
        path,
        staged,
    } = tab
    else {
        anyhow::bail!("Not a remote diff")
    };
    let relative = path
        .strip_prefix(cwd)?
        .to_str()
        .context("Remote diff path must be UTF-8")?
        .to_owned();
    terminator_core::remote::validate_relative(&relative)?;
    let response = client
        .rpc(terminator_core::Request::RemoteProject {
            project: project.clone(),
            operation: terminator_core::remote::Operation::GitDiff {
                path: relative.clone(),
                staged: *staged,
            },
        })
        .await?;
    let terminator_core::Response::RemoteProject(terminator_core::remote::Reply::Diff {
        path: actual,
        left,
        right,
        staged: actual_staged,
    }) = response
    else {
        anyhow::bail!("Unexpected remote diff response")
    };
    anyhow::ensure!(
        actual == relative && actual_staged == *staged,
        "Remote diff identity changed"
    );
    let path = path.clone();
    let staged = *staged;
    service
        .cpu()
        .run(cancel, move || {
            let left = base64::engine::general_purpose::STANDARD.decode(left)?;
            let right = base64::engine::general_purpose::STANDARD.decode(right)?;
            anyhow::ensure!(
                left.len() <= terminator_core::remote::MAX_FILE_BYTES
                    && right.len() <= terminator_core::remote::MAX_FILE_BYTES,
                "Remote diff exceeds its snapshot limit"
            );
            let left = String::from_utf8(left)
                .context("Remote binary files cannot be displayed as text diffs")?;
            let right = String::from_utf8(right)
                .context("Remote binary files cannot be displayed as text diffs")?;
            Ok(build_with_context(BuildWithContextInput {
                path: &path,
                left: &left,
                right: &right,
                staged,
                context,
            }))
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git_cmd(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git_cmd(dir.path(), &["init", "-q"]);
        git_cmd(dir.path(), &["config", "core.autocrlf", "false"]);
        git_cmd(
            dir.path(),
            &["config", "user.email", "fixture@example.invalid"],
        );
        git_cmd(dir.path(), &["config", "user.name", "Fixture"]);
        dir
    }

    /// 100 numbered lines with lines 10 and 80 changed.
    fn long_pair() -> (String, String) {
        let original: String = (0..100).fold(String::new(), |mut text, n| {
            use std::fmt::Write as _;
            let _ = writeln!(text, "{n}");
            text
        });
        let changed = original
            .replacen("10\n", "changed\n", 1)
            .replacen("80\n", "another\n", 1);
        (original, changed)
    }

    #[test]
    fn gap_steps_reveal_twenty_lines_from_either_edge() {
        let (original, changed) = long_pair();
        let doc = build_with_context(BuildWithContextInput {
            path: Path::new("f.txt"),
            left: &original,
            right: &changed,
            staged: false,
            context: 3,
        });
        // Leading, middle, trailing gaps.
        assert_eq!(doc.gaps.len(), 3);
        let middle = doc.gaps[1].old.clone();
        let top = doc
            .expand_gap(ExpandGapInput {
                path: Path::new("f.txt"),
                gap: 1,
                part: GapPart::Top,
                staged: false,
            })
            .unwrap();
        assert_eq!(top.gaps.len(), 3);
        assert_eq!(top.gaps[1].old, middle.start + GAP_STEP..middle.end);
        let both = top
            .expand_gap(ExpandGapInput {
                path: Path::new("f.txt"),
                gap: 1,
                part: GapPart::Bottom,
                staged: false,
            })
            .unwrap();
        assert_eq!(
            both.gaps[1].old,
            middle.start + GAP_STEP..middle.end - GAP_STEP
        );
        assert_eq!(both.revealed.len(), 2);
    }

    #[test]
    fn rebuild_keeps_revealed_ranges_and_whitespace_mode() {
        let (original, changed) = long_pair();
        let doc = build_with_context(BuildWithContextInput {
            path: Path::new("f.txt"),
            left: &original,
            right: &changed,
            staged: false,
            context: 3,
        });
        let expanded = doc
            .expand_gap(ExpandGapInput {
                path: Path::new("f.txt"),
                gap: 1,
                part: GapPart::All,
                staged: false,
            })
            .unwrap();
        let view = DiffView {
            context: 3,
            revealed: expanded.revealed.clone(),
            whitespace: false,
        };
        let refreshed = doc.rebuild(Path::new("f.txt"), false, &view);
        assert_eq!(refreshed.gaps.len(), 2);
        assert_eq!(refreshed.unified.len(), expanded.unified.len());
    }

    #[test]
    fn ignore_whitespace_hides_indent_only_changes_and_keeps_text() {
        let left = "fn a() {\nx();\n}\n";
        let right = "fn a() {\n    x();\n}\nnew();\n";
        let exact = build_with_context(BuildWithContextInput {
            path: Path::new("a.rs"),
            left,
            right,
            staged: false,
            context: 3,
        });
        assert_eq!(exact.hunks.len(), 2);
        let loose = exact.rebuild(
            Path::new("a.rs"),
            false,
            &DiffView {
                context: 3,
                revealed: Vec::new(),
                whitespace: true,
            },
        );
        // Only the added line is a change; the re-indent is not.
        assert_eq!(loose.hunks.len(), 1);
        assert_eq!(loose.hunks[0].new, 3..4);
        assert_eq!(loose.right_text, right);
        // Display keeps the real indentation.
        let shown: String = loose.unified[1]
            .spans
            .iter()
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(shown, "    x();");
    }

    #[test]
    fn aligned_left_has_one_row_per_buffer_line() {
        let left = "a\nb\nc\nd\n";
        let right = "a\nB\nnew\nd\n";
        let aligned = align_left(left, right, &[]);
        assert_eq!(aligned.doc.split.len(), 4);
        let left_no = |row: usize| aligned.doc.split[row].left.as_ref().and_then(|l| l.old_no);
        assert_eq!(left_no(0), Some(1));
        // `b`/`c` replaced by `B`/`new`: paired rows show the originals.
        assert_eq!(left_no(1), Some(2));
        assert_eq!(left_no(2), Some(3));
        assert_eq!(left_no(3), Some(4));
        assert!(aligned.deleted.is_empty());
        let removed = align_left("a\nb\nc\n", "a\nc\n", &[]);
        assert_eq!(removed.doc.split.len(), 2);
        assert_eq!(removed.deleted, [(1, vec!["b".to_owned()])]);
    }

    #[test]
    fn binary_sides_report_their_sizes() {
        let error = decode_sides(b"text", b"\0\x01\x02")
            .unwrap_err()
            .to_string();
        assert!(error.starts_with("Binary file · 4 B → 3 B"), "{error}");
        assert_eq!(human_size(2048), "2.0 KB");
    }

    #[test]
    fn staged_and_working_sides_and_word_highlights() {
        let dir = repo();
        let root = dir.path();
        let name = if cfg!(windows) {
            Path::new("space & ' 日本.rs")
        } else {
            Path::new("space | ' 日本.rs")
        };
        std::fs::write(root.join(name), "fn main() { base(); }\n").unwrap();
        git_cmd(root, &["add", "."]);
        git_cmd(root, &["commit", "-qm", "base"]);
        std::fs::write(root.join(name), "fn main() { staged(); }\n").unwrap();
        git_cmd(root, &["add", "."]);
        std::fs::write(root.join(name), "fn main() { working(); }\n").unwrap();
        let staged = document(DiffRequest {
            cwd: root,
            path: &root.join(name),
            staged: true,
        })
        .unwrap();
        assert_eq!(staged.left_label, "HEAD");
        assert_eq!(staged.right_label, "Index");
        assert_eq!(
            staged.left_text, "fn main() { base(); }\n",
            "left_text should contain the old blob content"
        );
        assert_eq!(
            staged.right_text, "fn main() { staged(); }\n",
            "right_text should contain the staged content"
        );
        let joined: String = staged
            .unified
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.text.as_str()))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("staged"));
        assert!(!joined.contains("working"));
        assert!(
            staged
                .unified
                .iter()
                .any(|line| line.kind == LineKind::Insert
                    && line.spans.iter().any(|s| s.intra == Intra::Change))
        );
        let working = document(DiffRequest {
            cwd: root,
            path: &root.join(name),
            staged: false,
        })
        .unwrap();
        assert_eq!(
            working.left_text, "fn main() { staged(); }\n",
            "working left_text should be the staged content"
        );
        assert_eq!(
            working.right_text, "fn main() { working(); }\n",
            "working right_text should be the working tree content"
        );
        let joined: String = working
            .unified
            .iter()
            .flat_map(|line| line.spans.iter().map(|s| s.text.as_str()))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("working"));
        assert!(!joined.contains("base"));
        assert!(!working.split.is_empty());
    }
}
