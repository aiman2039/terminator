//! Read-only Git snapshots rendered with similar + syntect. Never runs on the GUI thread.
use anyhow::{Context, Result, ensure};
use similar::{ChangeTag, DiffTag, TextDiff};
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};
use syntect::{
    easy::HighlightLines,
    highlighting::{Theme, ThemeSet},
    parsing::{SyntaxReference, SyntaxSet},
    util::LinesWithEndings,
};
use terminator_core::CommandOptions;

const CONTEXT: usize = 3;

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

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DiffDocument {
    pub left_label: String,
    pub right_label: String,
    pub left_text: String,
    pub right_text: String,
    pub unified: Vec<DiffLine>,
    pub split: Vec<SplitRow>,
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

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme() -> &'static Theme {
    static SET: OnceLock<ThemeSet> = OnceLock::new();
    static FALLBACK: OnceLock<Theme> = OnceLock::new();
    let themes = SET.get_or_init(ThemeSet::load_defaults);
    themes
        .themes
        .get("base16-ocean.dark")
        .unwrap_or_else(|| FALLBACK.get_or_init(Theme::default))
}

fn highlight_file(path: &Path, text: &str) -> Vec<Vec<DiffSpan>> {
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

fn syntax_for<'a>(path: &Path, set: &'a SyntaxSet) -> &'a SyntaxReference {
    path.extension()
        .and_then(|ext| ext.to_str())
        .and_then(|ext| set.find_syntax_by_extension(ext))
        .or_else(|| {
            path.file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| set.find_syntax_by_extension(name))
        })
        .unwrap_or_else(|| set.find_syntax_plain_text())
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

fn hunk_line(old_start: usize, old_len: usize, new_start: usize, new_len: usize) -> DiffLine {
    DiffLine {
        kind: LineKind::Hunk,
        old_no: None,
        new_no: None,
        spans: vec![DiffSpan {
            text: format!(
                "@@ -{},{} +{},{} @@",
                old_start.saturating_add(1),
                old_len,
                new_start.saturating_add(1),
                new_len
            ),
            rgb: fallback_rgb(),
            intra: Intra::None,
        }],
    }
}

fn build(path: &Path, left: &str, right: &str, staged: bool) -> DiffDocument {
    let old_hl = highlight_file(path, left);
    let new_hl = highlight_file(path, right);
    let diff = TextDiff::from_lines(left, right);
    let mut unified = Vec::new();
    let mut split = Vec::new();
    for group in diff.grouped_ops(CONTEXT) {
        if let (Some(first), Some(last)) = (group.first(), group.last()) {
            unified.push(hunk_line(
                first.old_range().start,
                last.old_range().end.saturating_sub(first.old_range().start),
                first.new_range().start,
                last.new_range().end.saturating_sub(first.new_range().start),
            ));
            split.push(SplitRow {
                left: unified.last().cloned(),
                right: None,
            });
        }
        for op in &group {
            emit_op(&diff, op, &old_hl, &new_hl, &mut unified, &mut split);
        }
    }
    DiffDocument {
        left_label: if staged { "HEAD" } else { "Index" }.into(),
        right_label: if staged { "Index" } else { "Working tree" }.into(),
        left_text: left.into(),
        right_text: right.into(),
        unified,
        split,
    }
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
    .await?;
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
        ensure!(
            !staged,
            "This file has no staged changes; refresh Git status"
        );
        let tracked = terminator_git::run(
            service.processes(),
            service.fs(),
            &root,
            vec![
                "ls-files".into(),
                "-z".into(),
                "--".into(),
                relative.as_os_str().to_owned(),
            ],
            options(),
        )
        .await?;
        ensure!(
            tracked.is_empty(),
            "This file has no working-tree changes; refresh Git status"
        );
        let path = root.join(&relative);
        let right = service
            .fs()
            .run(cancel, move || {
                ensure!(path.exists(), "File no longer exists; refresh Git status");
                terminator_git::worktree_file(&path)
            })
            .await?;
        (Vec::new(), right)
    };
    service
        .cpu()
        .run(cancel, move || {
            Ok(build(
                &relative,
                &decode_side(&left)?,
                &decode_side(&right)?,
                staged,
            ))
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
        let name = Path::new("space | ' 日本.rs");
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
