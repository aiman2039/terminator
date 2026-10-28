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
mod build;
mod highlight;
mod load;
pub(crate) mod review;
#[cfg(test)]
mod tests;

#[cfg(test)]
use build::build;
pub(crate) use build::{BuildWithContextInput, BuildWithOptionsInput, build_with_context};
use build::build_with_options;
pub(crate) use highlight::highlight_spans;
use highlight::{fallback_rgb, highlight_file, line_spans, overlay};
pub use load::{DocumentAsyncInput, document_async};
pub(crate) use load::{
    DocumentWithContextInput, RemoteDocumentInput, document_with_context, remote_document,
};

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
