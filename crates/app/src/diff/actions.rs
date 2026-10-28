//! Explicit native-review mutations, executed through the shared background pools.
use super::{DiffDocument, review::Hunk};
use crate::gui_services::Services;
use anyhow::{Context, Result, ensure};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};
use terminator_core::{CommandOptions, async_service::CancellationToken};

#[cfg(test)]
#[path = "actions_tests.rs"]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Source {
    pub root: PathBuf,
    pub path: PathBuf,
    index: Vec<u8>,
    head: Vec<u8>,
    exists: bool,
    mode: u32,
}

pub(super) fn literal(path: &Path) -> OsString {
    let mut value = OsString::from(":(literal)");
    value.push(path);
    value
}

async fn git(
    service: &Services,
    root: &Path,
    args: Vec<OsString>,
    input: Option<Vec<u8>>,
) -> Result<Vec<u8>> {
    terminator_git::run(
        service.processes(),
        service.fs(),
        terminator_git::RunInput {
            cwd: root,
            args,
            options: CommandOptions {
                input,
                stdout_limit: 4 * 1024 * 1024,
                ..Default::default()
            },
        },
    )
    .await
}

impl Source {
    pub async fn capture(
        service: &Services,
        root: &Path,
        path: &Path,
        cancel: &CancellationToken,
    ) -> Result<Self> {
        let index = git(
            service,
            root,
            vec![
                "ls-files".into(),
                "--stage".into(),
                "-z".into(),
                "--".into(),
                literal(path),
            ],
            None,
        )
        .await?;
        // An unborn branch has no HEAD tree.
        let head = terminator_git::run(
            service.processes(),
            service.fs(),
            terminator_git::RunInput {
                cwd: root,
                args: vec![
                    "ls-tree".into(),
                    "-z".into(),
                    "HEAD".into(),
                    "--".into(),
                    literal(path),
                ],
                options: CommandOptions {
                    accepted_exit_codes: Some(vec![0, 128]),
                    ..Default::default()
                },
            },
        )
        .await?;
        let absolute = root.join(path);
        let (exists, mode) = service
            .fs()
            .run(cancel, move || file_identity(&absolute))
            .await?;
        Ok(Self {
            root: root.into(),
            path: path.into(),
            index,
            head,
            exists,
            mode,
        })
    }
}

fn file_identity(path: &Path) -> Result<(bool, u32)> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            ensure!(
                meta.is_file() && !meta.file_type().is_symlink(),
                "Hunk actions require a regular file"
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                Ok((
                    true,
                    if meta.permissions().mode() & 0o111 == 0 {
                        0o100644
                    } else {
                        0o100755
                    },
                ))
            }
            #[cfg(not(unix))]
            {
                Ok((true, 0o100644))
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok((false, 0o100644)),
        Err(error) => Err(error.into()),
    }
}

fn entry_mode(entry: &[u8]) -> u32 {
    entry
        .split(|byte| *byte == b' ')
        .next()
        .and_then(|mode| std::str::from_utf8(mode).ok())
        .and_then(|mode| u32::from_str_radix(mode, 8).ok())
        .unwrap_or(0o100644)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Stage,
    Unstage,
    Discard,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Stage => "Stage",
            Self::Unstage => "Unstage",
            Self::Discard => "Discard",
        }
    }
}

pub(crate) enum Request {
    Save {
        document: Arc<DiffDocument>,
        text: String,
    },
    Hunk {
        document: Arc<DiffDocument>,
        hunk: usize,
        kind: Kind,
        /// Only these lines of the hunk (line-level staging); `None` is
        /// the whole hunk.
        lines: Option<LineSelection>,
    },
    Undo(Undo),
}

/// Lines picked inside one hunk: left-side (removed) and right-side
/// (added) line indices.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LineSelection {
    pub old: std::collections::BTreeSet<usize>,
    pub new: std::collections::BTreeSet<usize>,
}

impl LineSelection {
    pub fn is_empty(&self) -> bool {
        self.old.is_empty() && self.new.is_empty()
    }

    pub fn len(&self) -> usize {
        self.old.len().saturating_add(self.new.len())
    }
}

/// The hunk region rewritten with only the picked lines applied. Staging
/// drops picked removals and adds picked additions to the left text;
/// unstaging or discarding restores picked removals and drops picked
/// additions from the right text. Unpicked lines stay as they were.
fn partial_lines(doc: &DiffDocument, hunk: &Hunk, kind: Kind, picked: &LineSelection) -> String {
    let left: Vec<&str> = doc.left_text.split_inclusive('\n').collect();
    let right: Vec<&str> = doc.right_text.split_inclusive('\n').collect();
    let old = hunk
        .old
        .clone()
        .filter_map(|index| Some((index, *left.get(index)?)));
    let new = hunk
        .new
        .clone()
        .filter_map(|index| Some((index, *right.get(index)?)));
    let mut region = String::new();
    if kind == Kind::Stage {
        region.extend(
            old.filter(|(index, _)| !picked.old.contains(index))
                .map(|(_, line)| line),
        );
        region.extend(
            new.filter(|(index, _)| picked.new.contains(index))
                .map(|(_, line)| line),
        );
    } else {
        region.extend(
            old.filter(|(index, _)| picked.old.contains(index))
                .map(|(_, line)| line),
        );
        region.extend(
            new.filter(|(index, _)| !picked.new.contains(index))
                .map(|(_, line)| line),
        );
    }
    region
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Version {
    text: String,
    exists: bool,
    mode: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct Undo {
    root: PathBuf,
    path: PathBuf,
    index: bool,
    before: Version,
    after: Version,
}

#[derive(Debug)]
pub(crate) struct Outcome {
    pub undo: Option<Undo>,
    pub message: String,
}

fn replace_lines(
    text: &str,
    range: &std::ops::Range<usize>,
    replacement: &str,
    selected: &std::ops::Range<usize>,
) -> String {
    text.split_inclusive('\n')
        .take(range.start)
        .chain(
            replacement
                .split_inclusive('\n')
                .skip(selected.start)
                .take(selected.len()),
        )
        .chain(text.split_inclusive('\n').skip(range.end))
        .collect()
}

struct EditInput<'a> {
    source: &'a Source,
    doc: &'a DiffDocument,
    hunk: &'a Hunk,
    kind: Kind,
    lines: Option<&'a LineSelection>,
}

fn edit(input: EditInput<'_>) -> Undo {
    let EditInput {
        source,
        doc,
        hunk,
        kind,
        lines,
    } = input;
    let index = kind != Kind::Discard;
    let before = match kind {
        Kind::Stage => Version {
            text: doc.left_text.clone(),
            exists: !source.index.is_empty(),
            mode: entry_mode(&source.index),
        },
        Kind::Unstage => Version {
            text: doc.right_text.clone(),
            exists: !source.index.is_empty(),
            mode: entry_mode(&source.index),
        },
        Kind::Discard => Version {
            text: doc.right_text.clone(),
            exists: source.exists,
            mode: source.mode,
        },
    };
    let text = match lines.filter(|picked| !picked.is_empty()) {
        Some(picked) => {
            let region = partial_lines(doc, hunk, kind, picked);
            let (base, range) = if kind == Kind::Stage {
                (&doc.left_text, &hunk.old)
            } else {
                (&doc.right_text, &hunk.new)
            };
            let count = region.split_inclusive('\n').count();
            replace_lines(base, range, &region, &(0..count))
        }
        None if kind == Kind::Stage => {
            replace_lines(&doc.left_text, &hunk.old, &doc.right_text, &hunk.new)
        }
        None => replace_lines(&doc.right_text, &hunk.new, &doc.left_text, &hunk.old),
    };
    let (base_exists, base_mode) = match kind {
        Kind::Stage => (source.exists, source.mode),
        Kind::Unstage => (!source.head.is_empty(), entry_mode(&source.head)),
        Kind::Discard => (!source.index.is_empty(), entry_mode(&source.index)),
    };
    let after = Version {
        exists: !text.is_empty() || base_exists,
        text,
        mode: if before.exists {
            before.mode
        } else {
            base_mode
        },
    };
    Undo {
        root: source.root.clone(),
        path: source.path.clone(),
        index,
        before,
        after,
    }
}

// Git's quoted patch paths use byte-wise octal escapes, including non-UTF-8 names.
fn quoted(prefix: &str, path: &Path) -> String {
    use std::fmt::Write;
    let mut out = format!("\"{prefix}/");
    for byte in terminator_core::os_bytes(path.as_os_str()).iter().copied() {
        match byte {
            b'"' | b'\\' => {
                out.push('\\');
                out.push(char::from(byte));
            }
            32..=126 => out.push(char::from(byte)),
            _ => {
                let _ = write!(out, "\\{byte:03o}");
            }
        }
    }
    out.push('"');
    out
}

fn patch(path: &Path, before: &Version, after: &Version) -> Vec<u8> {
    let left = quoted("a", path);
    let right = quoted("b", path);
    let mut patch = format!("diff --git {left} {right}\n");
    if !before.exists {
        patch.push_str(&format!("new file mode {:06o}\n", after.mode));
    }
    if !after.exists {
        patch.push_str(&format!("deleted file mode {:06o}\n", before.mode));
    }
    if before.exists && after.exists && before.mode != after.mode {
        patch.push_str(&format!(
            "old mode {:06o}\nnew mode {:06o}\n",
            before.mode, after.mode
        ));
    }
    patch.push_str(
        &similar::TextDiff::from_lines(&before.text, &after.text)
            .unified_diff()
            .context_radius(3)
            .header(
                if before.exists { &left } else { "/dev/null" },
                if after.exists { &right } else { "/dev/null" },
            )
            .to_string(),
    );
    patch.into_bytes()
}

async fn current(service: &Services, change: &Undo, cancel: &CancellationToken) -> Result<Version> {
    if change.index {
        let entry = git(
            service,
            &change.root,
            vec![
                "ls-files".into(),
                "--stage".into(),
                "-z".into(),
                "--".into(),
                literal(&change.path),
            ],
            None,
        )
        .await?;
        ensure!(
            entry.split(|b| *b == 0).filter(|s| !s.is_empty()).count() <= 1,
            "Resolve the file's merge conflict first"
        );
        let oid = entry
            .split(|b| *b == b' ')
            .nth(1)
            .map(std::str::from_utf8)
            .transpose()?;
        let text = if let Some(oid) = oid {
            git(
                service,
                &change.root,
                vec!["cat-file".into(), "blob".into(), oid.into()],
                None,
            )
            .await?
        } else {
            Vec::new()
        };
        Ok(Version {
            text: String::from_utf8(text)?,
            exists: !entry.is_empty(),
            mode: entry_mode(&entry),
        })
    } else {
        let path = change.root.join(&change.path);
        service
            .fs()
            .run(cancel, move || {
                let (exists, mode) = file_identity(&path)?;
                Ok(Version {
                    text: String::from_utf8(terminator_git::worktree_file(&path)?)?,
                    exists,
                    mode,
                })
            })
            .await
    }
}

async fn apply(service: &Services, change: &Undo, cancel: &CancellationToken) -> Result<()> {
    let root = change.root.clone();
    let path = change.path.clone();
    service
        .fs()
        .run(cancel, move || {
            ensure!(
                terminator_git::relative_path(&root, &root.join(&path))? == path,
                "The file path changed; refresh the diff"
            );
            Ok(())
        })
        .await?;
    ensure!(
        current(service, change, cancel).await? == change.before,
        "The file or index changed; refresh the diff before retrying"
    );
    let path = change.path.clone();
    let before = change.before.clone();
    let after = change.after.clone();
    let bytes = service
        .cpu()
        .run(cancel, move || Ok(patch(&path, &before, &after)))
        .await?;
    let mut args: Vec<OsString> = vec!["apply".into(), "--whitespace=nowarn".into()];
    if change.index {
        args.push("--cached".into());
    }
    let mut check = args.clone();
    check.push("--check".into());
    git(service, &change.root, check, Some(bytes.clone())).await?;
    // Recheck the complete target, including changes outside the selected hunk.
    ensure!(
        current(service, change, cancel).await? == change.before,
        "The file or index changed; refresh the diff before retrying"
    );
    git(service, &change.root, args, Some(bytes)).await?;
    Ok(())
}

pub(crate) async fn execute(
    service: &Services,
    request: Request,
    staged: bool,
    cancel: &CancellationToken,
) -> Result<Outcome> {
    // Serialize the full read/check/apply sequence across native diff tabs.
    static MUTATIONS: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    let _guard = MUTATIONS
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await;
    match request {
        Request::Save { document, text } => {
            ensure!(!document.read_only, "This diff is read-only");
            ensure!(!staged, "Only the working tree can be edited");
            let source = document
                .source
                .as_ref()
                .context("Refresh the diff before editing")?;
            let fresh = super::document_async(super::DocumentAsyncInput {
                service,
                cwd: source.root.clone(),
                path: source.root.join(&source.path),
                staged: false,
                cancel,
            })
            .await?;
            ensure!(
                fresh.source == document.source
                    && fresh.left_text == document.left_text
                    && fresh.right_text == document.right_text,
                "The file, HEAD, or index changed; your edits were kept. Reopen the file to resolve the changes"
            );
            let before = Version {
                text: document.right_text.clone(),
                exists: source.exists,
                mode: source.mode,
            };
            let change = Undo {
                root: source.root.clone(),
                path: source.path.clone(),
                index: false,
                after: Version {
                    text,
                    exists: true,
                    mode: before.mode,
                },
                before,
            };
            apply(service, &change, cancel).await?;
            Ok(Outcome {
                undo: Some(change),
                message: "File saved".into(),
            })
        }
        Request::Hunk {
            document,
            hunk,
            kind,
            lines,
        } => {
            ensure!(!document.read_only, "This diff is read-only");
            ensure!(
                (kind == Kind::Unstage) == staged,
                "This action does not match the diff side"
            );
            let source = document
                .source
                .as_ref()
                .context("Refresh the diff before changing a hunk")?;
            let fresh = super::document_async(super::DocumentAsyncInput {
                service,
                cwd: source.root.clone(),
                path: source.root.join(&source.path),
                staged,
                cancel,
            })
            .await?;
            ensure!(
                fresh.source == document.source
                    && fresh.left_text == document.left_text
                    && fresh.right_text == document.right_text,
                "The file, HEAD, or index changed; refresh the diff before retrying"
            );
            let hunk = document
                .hunks
                .get(hunk)
                .context("This hunk is no longer available; refresh the diff")?;
            let change = edit(EditInput {
                source,
                doc: &document,
                hunk,
                kind,
                lines: lines.as_ref(),
            });
            apply(service, &change, cancel).await?;
            Ok(Outcome {
                undo: Some(change),
                message: match &lines {
                    Some(picked) if !picked.is_empty() => format!(
                        "{} complete for {} line{}",
                        kind.label(),
                        picked.len(),
                        if picked.len() == 1 { "" } else { "s" }
                    ),
                    _ => format!("{} complete", kind.label()),
                },
            })
        }
        Request::Undo(change) => {
            let reverse = Undo {
                before: change.after.clone(),
                after: change.before.clone(),
                ..change
            };
            apply(service, &reverse, cancel).await?;
            Ok(Outcome {
                undo: None,
                message: "Undo complete".into(),
            })
        }
    }
}
