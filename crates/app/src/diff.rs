//! Read-only Git snapshots rendered with similar + syntect. Never runs on the GUI thread.
use anyhow::{Context, Result, bail, ensure};
use similar::{ChangeTag, DiffTag, TextDiff};
use std::{
    io::Read,
    os::unix::ffi::OsStrExt,
    path::{Component, Path, PathBuf},
    process::Command,
    sync::OnceLock,
};
use syntect::{
    easy::HighlightLines,
    highlighting::{Theme, ThemeSet},
    parsing::{SyntaxReference, SyntaxSet},
    util::LinesWithEndings,
};

const LIMIT: usize = 1024 * 1024;
const CONTEXT: usize = 3;

pub struct DiffRequest<'a> {
    pub cwd: &'a Path,
    pub path: &'a Path,
    pub staged: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intra {
    None,
    Change,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Hunk,
    Equal,
    Delete,
    Insert,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffSpan {
    pub text: String,
    pub rgb: [u8; 3],
    pub intra: Intra,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub spans: Vec<DiffSpan>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitRow {
    pub left: Option<DiffLine>,
    pub right: Option<DiffLine>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffDocument {
    pub left_label: String,
    pub right_label: String,
    pub unified: Vec<DiffLine>,
    pub split: Vec<SplitRow>,
}

pub fn document(DiffRequest { cwd, path, staged }: DiffRequest<'_>) -> Result<DiffDocument> {
    let root = git_root(cwd)?;
    let relative = relative_path(&root, path)?;
    let (left, right) = snapshots(&root, &relative, staged)?;
    Ok(build(&relative, &left, &right, staged))
}

fn git_root(cwd: &Path) -> Result<PathBuf> {
    let bytes = git(cwd, &["rev-parse", "--show-toplevel"])?;
    let text = std::str::from_utf8(&bytes)?.trim();
    PathBuf::from(text)
        .canonicalize()
        .context("Resolve Git root")
}

fn relative_path(root: &Path, path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    // Resolve directory aliases, but retain the Git entry's own identity.
    // Resolving the final component could silently review a symlink's target.
    match std::fs::symlink_metadata(&absolute) {
        Ok(meta) => ensure!(
            !meta.file_type().is_symlink(),
            "Diff review supports regular files, not symlinks or submodules"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let absolute = absolute
        .parent()
        .and_then(|parent| parent.canonicalize().ok())
        .and_then(|parent| absolute.file_name().map(|name| parent.join(name)))
        .unwrap_or(absolute);
    let relative = absolute
        .strip_prefix(root)
        .or_else(|_| path.strip_prefix(root))
        .context("Diff file is outside the repository")?;
    ensure!(
        relative
            .components()
            .all(|c| matches!(c, Component::Normal(_))),
        "Review path must be inside the repository"
    );
    Ok(relative.to_path_buf())
}

fn git(cwd: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let mut cmd = Command::new("git");
    cmd.env("GIT_OPTIONAL_LOCKS", "0")
        .arg("-C")
        .arg(cwd)
        .args(args);
    crate::services::run(cmd)
}

fn git_os(cwd: &Path, args: &[&std::ffi::OsStr]) -> Result<Vec<u8>> {
    let mut cmd = Command::new("git");
    cmd.env("GIT_OPTIONAL_LOCKS", "0")
        .arg("-C")
        .arg(cwd)
        .args(args);
    crate::services::run(cmd)
}

fn blob(root: &Path, oid: &str) -> Result<Vec<u8>> {
    if oid.bytes().all(|b| b == b'0') {
        return Ok(vec![]);
    }
    let bytes = git_os(root, &["cat-file".as_ref(), "blob".as_ref(), oid.as_ref()])?;
    ensure!(bytes.len() <= LIMIT, "Diff file exceeds 1 MiB");
    Ok(bytes)
}

fn worktree_file(path: &Path) -> Result<Vec<u8>> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        meta.is_file(),
        "Diff review supports regular files, not symlinks or submodules"
    );
    ensure!(meta.len() <= LIMIT as u64, "Diff file exceeds 1 MiB");
    let mut bytes = vec![];
    std::fs::File::open(path)?
        .take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= LIMIT, "Diff file exceeds 1 MiB");
    Ok(bytes)
}

fn changed_blob(root: &Path, path: &Path, staged: bool) -> Result<Option<(Vec<u8>, Vec<u8>)>> {
    let mut args: Vec<&std::ffi::OsStr> = vec![
        "diff".as_ref(),
        "--raw".as_ref(),
        "-z".as_ref(),
        "--no-abbrev".as_ref(),
        "--no-ext-diff".as_ref(),
        "--no-textconv".as_ref(),
        "--find-renames".as_ref(),
    ];
    if staged {
        args.push("--cached".as_ref());
    }
    let raw = git_os(root, &args)?;
    let mut fields = raw.split(|b| *b == 0).filter(|f| !f.is_empty());
    while let Some(header) = fields.next() {
        let header = std::str::from_utf8(header)?;
        let parts: Vec<_> = header.split_whitespace().collect();
        ensure!(parts.len() == 5, "Invalid Git diff record");
        let first = fields.next().context("Missing Git path")?;
        let target = if parts[4].starts_with(['R', 'C']) {
            fields.next().context("Missing rename target")?
        } else {
            first
        };
        if target != path.as_os_str().as_bytes() {
            continue;
        }
        ensure!(
            !parts[4].starts_with('U'),
            "Resolve this file's merge conflict in your editor before opening a two-way diff"
        );
        ensure!(
            parts[0] != ":160000"
                && parts[1] != "160000"
                && parts[0] != ":120000"
                && parts[1] != "120000",
            "Diff review supports regular files, not symlinks or submodules"
        );
        let left = blob(root, parts[2])?;
        let right = if staged {
            blob(root, parts[3])?
        } else {
            worktree_file(&root.join(path))?
        };
        return Ok(Some((left, right)));
    }
    Ok(None)
}

fn snapshots(root: &Path, path: &Path, staged: bool) -> Result<(String, String)> {
    let pair = match changed_blob(root, path, staged)? {
        Some(pair) => pair,
        None if staged => bail!("This file has no staged changes; refresh Git status"),
        None => {
            let tracked = git_os(
                root,
                &[
                    "ls-files".as_ref(),
                    "-z".as_ref(),
                    "--".as_ref(),
                    path.as_os_str(),
                ],
            )?;
            ensure!(
                tracked.is_empty(),
                "This file has no working-tree changes; refresh Git status"
            );
            ensure!(
                root.join(path).exists(),
                "File no longer exists; refresh Git status"
            );
            (vec![], worktree_file(&root.join(path))?)
        }
    };
    Ok((decode_side(&pair.0)?, decode_side(&pair.1)?))
}

fn decode_side(bytes: &[u8]) -> Result<String> {
    ensure!(
        !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok(),
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
    let themes = SET.get_or_init(ThemeSet::load_defaults);
    &themes.themes["base16-ocean.dark"]
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
        offset += text.len();
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
                old_start + 1,
                old_len,
                new_start + 1,
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
        old_no: change.old_index().map(|n| n as u32 + 1),
        new_no: change.new_index().map(|n| n as u32 + 1),
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
        old_no: change.old_index().map(|n| n as u32 + 1),
        new_no: change.new_index().map(|n| n as u32 + 1),
        spans,
    }
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

    #[test]
    fn symlink_reviews_reject_the_link_instead_of_reviewing_its_target() {
        let dir = repo();
        let root = dir.path();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(root.join("target.rs"), "old\n").unwrap();
        std::fs::write(outside.path().join("target.rs"), "outside\n").unwrap();
        std::os::unix::fs::symlink("target.rs", root.join("inside.rs")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("target.rs"), root.join("outside.rs"))
            .unwrap();
        std::os::unix::fs::symlink("missing.rs", root.join("dangling.rs")).unwrap();
        git_cmd(root, &["add", "."]);
        git_cmd(root, &["commit", "-qm", "base"]);
        std::fs::write(root.join("target.rs"), "staged\n").unwrap();
        git_cmd(root, &["add", "target.rs"]);
        std::fs::write(root.join("target.rs"), "working\n").unwrap();
        for name in ["inside.rs", "outside.rs", "dangling.rs"] {
            for path in [PathBuf::from(name), root.join(name)] {
                for staged in [false, true] {
                    let error = document(DiffRequest {
                        cwd: root,
                        path: &path,
                        staged,
                    })
                    .unwrap_err();
                    assert!(
                        error.to_string().contains("symlinks"),
                        "{path:?}: {error:#}"
                    );
                }
            }
        }
    }

    #[test]
    fn directory_aliases_and_deleted_files_keep_their_git_identity() {
        let dir = repo();
        let root = dir.path();
        std::fs::write(root.join("file.rs"), "old\n").unwrap();
        git_cmd(root, &["add", "."]);
        git_cmd(root, &["commit", "-qm", "base"]);
        let alias_dir = tempfile::tempdir().unwrap();
        let alias = alias_dir.path().join("repo");
        std::os::unix::fs::symlink(root, &alias).unwrap();
        std::fs::remove_file(root.join("file.rs")).unwrap();
        let doc = document(DiffRequest {
            cwd: &alias,
            path: &alias.join("file.rs"),
            staged: false,
        })
        .unwrap();
        assert!(doc.unified.iter().any(|line| line.kind == LineKind::Delete));
    }

    #[test]
    fn untracked_and_binary_and_outside_paths_are_explicit() {
        let dir = repo();
        let root = dir.path();
        std::fs::write(root.join("new.rs"), "fn new() {}\n").unwrap();
        let doc = document(DiffRequest {
            cwd: root,
            path: Path::new("new.rs"),
            staged: false,
        })
        .unwrap();
        assert!(doc.unified.iter().any(|line| line.kind == LineKind::Insert
            && line.spans.iter().any(|s| s.text.contains("new"))));
        std::fs::write(root.join("new.rs"), b"binary\0").unwrap();
        assert!(
            document(DiffRequest {
                cwd: root,
                path: Path::new("new.rs"),
                staged: false,
            })
            .unwrap_err()
            .to_string()
            .contains("Binary")
        );
        assert!(
            document(DiffRequest {
                cwd: root,
                path: Path::new("../outside.rs"),
                staged: false,
            })
            .is_err()
        );
    }
}
