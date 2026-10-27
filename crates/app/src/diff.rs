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
    pub(crate) source: Option<actions::Source>,
    pub(crate) generation: DiffGeneration,
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

    pub(crate) fn expand_gap(&self, path: &Path, gap: usize, staged: bool) -> Result<Self> {
        let gap = self
            .gaps
            .get(gap)
            .context("The hidden lines changed; refresh the diff")?;
        let mut revealed = self.revealed.clone();
        revealed.push(gap.old.clone());
        let mut doc = build_with_revealed(
            path,
            &self.left_text,
            &self.right_text,
            staged,
            self.context,
            revealed,
        );
        doc.read_only = self.read_only;
        doc.source.clone_from(&self.source);
        doc.left_label.clone_from(&self.left_label);
        doc.right_label.clone_from(&self.right_label);
        Ok(doc)
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
    Ok(build(
        &relative,
        &decode_side(&left)?,
        &decode_side(&right)?,
        staged,
    ))
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

fn decode_side(bytes: &[u8]) -> Result<String> {
    ensure!(
        terminator_git::check_text(bytes),
        "Binary or non-UTF-8 files cannot be reviewed natively"
    );
    Ok(String::from_utf8_lossy(bytes).into_owned())
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
    build_with_context(path, left, right, staged, CONTEXT)
}

pub(crate) fn build_with_context(
    path: &Path,
    left: &str,
    right: &str,
    staged: bool,
    context: usize,
) -> DiffDocument {
    build_with_revealed(path, left, right, staged, context, Vec::new())
}

fn build_with_revealed(
    path: &Path,
    left: &str,
    right: &str,
    staged: bool,
    context: usize,
    revealed: Vec<std::ops::Range<usize>>,
) -> DiffDocument {
    let old_hl = highlight_file(path, left);
    let new_hl = highlight_file(path, right);
    let diff = TextDiff::configure()
        .timeout(DIFF_TIMEOUT)
        .diff_lines(left, right);
    let mut doc = DiffDocument {
        left_label: if staged { "HEAD" } else { "Index" }.into(),
        right_label: if staged { "Index" } else { "Working tree" }.into(),
        left_text: left.into(),
        right_text: right.into(),
        context,
        revealed,
        ..Default::default()
    };
    let mut old_end = 0;
    let mut new_end = 0;
    for group in diff.grouped_ops(context) {
        if let Some(first) = group.first() {
            emit_gap(
                &mut doc,
                &diff,
                old_end..first.old_range().start,
                new_end..first.new_range().start,
                &old_hl,
                &new_hl,
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
            emit_op(
                &diff,
                op,
                &old_hl,
                &new_hl,
                &mut doc.unified,
                &mut doc.split,
            );
            old_end = op.old_range().end;
            new_end = op.new_range().end;
        }
    }
    if !doc.hunks.is_empty() {
        emit_gap(
            &mut doc,
            &diff,
            old_end..left.split_inclusive('\n').count(),
            new_end..right.split_inclusive('\n').count(),
            &old_hl,
            &new_hl,
        );
    }
    doc
}

fn emit_gap(
    doc: &mut DiffDocument,
    diff: &TextDiff<'_, '_, str>,
    old: std::ops::Range<usize>,
    new: std::ops::Range<usize>,
    old_hl: &[Vec<DiffSpan>],
    new_hl: &[Vec<DiffSpan>],
) {
    if old.is_empty() {
        return;
    }
    if doc
        .revealed
        .iter()
        .any(|range| range.start <= old.start && range.end >= old.end)
    {
        let op = similar::DiffOp::Equal {
            old_index: old.start,
            new_index: new.start,
            len: old.len(),
        };
        emit_op(diff, &op, old_hl, new_hl, &mut doc.unified, &mut doc.split);
        return;
    }
    let line = DiffLine {
        kind: LineKind::Hunk,
        old_no: None,
        new_no: None,
        spans: vec![DiffSpan {
            text: format!("⋯ {} unchanged lines ⋯", old.len()),
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

fn emit_op(
    diff: &TextDiff<'_, '_, str>,
    op: &similar::DiffOp,
    old_hl: &[Vec<DiffSpan>],
    new_hl: &[Vec<DiffSpan>],
    unified: &mut Vec<DiffLine>,
    split: &mut Vec<SplitRow>,
) {
    match op.tag() {
        DiffTag::Equal => {
            for change in diff.iter_changes(op) {
                let line = equal_line(change, old_hl);
                unified.push(line.clone());
                split.push(SplitRow {
                    left: Some(line.clone()),
                    right: Some(line),
                });
            }
        }
        DiffTag::Delete => {
            for change in diff.iter_inline_changes(op) {
                let line = change_line(change, LineKind::Delete, old_hl, true);
                unified.push(line.clone());
                split.push(SplitRow {
                    left: Some(line),
                    right: None,
                });
            }
        }
        DiffTag::Insert => {
            for change in diff.iter_inline_changes(op) {
                let line = change_line(change, LineKind::Insert, new_hl, false);
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
                        deletes.push(change_line(change, LineKind::Delete, old_hl, true));
                    }
                    ChangeTag::Insert => {
                        inserts.push(change_line(change, LineKind::Insert, new_hl, false));
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

fn equal_line(change: similar::Change<&str>, old_hl: &[Vec<DiffSpan>]) -> DiffLine {
    let text = change.value();
    DiffLine {
        kind: LineKind::Equal,
        old_no: change
            .old_index()
            .map(|n| u32::try_from(n).unwrap_or(u32::MAX).saturating_add(1)),
        new_no: change
            .new_index()
            .map(|n| u32::try_from(n).unwrap_or(u32::MAX).saturating_add(1)),
        spans: line_spans(old_hl, change.old_index(), text),
    }
}

fn change_line(
    change: similar::InlineChange<'_, str>,
    kind: LineKind,
    highlighted: &[Vec<DiffSpan>],
    old_side: bool,
) -> DiffLine {
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

pub async fn document_async(
    service: &crate::gui_services::Services,
    cwd: PathBuf,
    path: PathBuf,
    staged: bool,
    cancel: &terminator_core::async_service::CancellationToken,
) -> Result<DiffDocument> {
    document_with_context(service, cwd, path, staged, CONTEXT, cancel).await
}

pub(crate) async fn document_with_context(
    service: &crate::gui_services::Services,
    cwd: PathBuf,
    path: PathBuf,
    staged: bool,
    context: usize,
    cancel: &terminator_core::async_service::CancellationToken,
) -> Result<DiffDocument> {
    let options = || CommandOptions {
        stdout_limit: 4 * 1024 * 1024,
        ..Default::default()
    };
    let root_bytes = terminator_git::run(
        service.processes(),
        service.fs(),
        &cwd,
        vec!["rev-parse".into(), "--show-toplevel".into()],
        options(),
    )
    .await;
    let root_bytes = match root_bytes {
        Ok(bytes) => bytes,
        Err(_) if !staged => {
            return crate::perforce::document(service, cwd, path, context, cancel).await;
        }
        Err(error) => return Err(error),
    };
    let root = PathBuf::from(std::str::from_utf8(&root_bytes)?.trim());
    let (root, relative) = service
        .fs()
        .run(cancel, move || {
            let root = root.canonicalize().context("Resolve Git root")?;
            let relative = terminator_git::relative_path(&root, &path)?;
            Ok((root, relative))
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
    let raw =
        terminator_git::run(service.processes(), service.fs(), &root, args, options()).await?;
    let oids = terminator_git::find_record(&raw, &relative)?;
    let (left, right) = if let Some((left, right)) = oids {
        let left = terminator_git::snapshot::blob_async(
            service.processes(),
            service.fs(),
            &root,
            &left,
            options(),
        )
        .await?;
        let right = if staged {
            terminator_git::snapshot::blob_async(
                service.processes(),
                service.fs(),
                &root,
                &right,
                options(),
            )
            .await?
        } else {
            let path = root.join(&relative);
            service
                .fs()
                .run(cancel, move || terminator_git::worktree_file(&path))
                .await?
        };
        (left, right)
    } else {
        let tracked = terminator_git::run(
            service.processes(),
            service.fs(),
            &root,
            vec![
                "ls-files".into(),
                "--stage".into(),
                "-z".into(),
                "--".into(),
                actions::literal(&relative),
            ],
            options(),
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
                &root,
                oid,
                options(),
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
                .run(cancel, move || terminator_git::worktree_file(&path))
                .await?
        };
        (left, right)
    };
    let source = actions::Source::capture(service, &root, &relative, cancel).await?;
    service
        .cpu()
        .run(cancel, move || {
            let mut doc = build_with_context(
                &relative,
                &decode_side(&left)?,
                &decode_side(&right)?,
                staged,
                context,
            );
            doc.source = Some(source);
            Ok(doc)
        })
        .await
}
pub(crate) async fn remote_document(
    service: &crate::gui_services::Services,
    client: &terminator_core::async_client::Client,
    tab: &crate::Tab,
    context: usize,
    cancel: &terminator_core::async_service::CancellationToken,
) -> Result<DiffDocument> {
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
            Ok(build_with_context(&path, &left, &right, staged, context))
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
