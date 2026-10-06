//! Read-only snapshots for diff review: blob pairs with symlink, submodule,
//! size, and text checks shared by the native viewer and `CodeDiff` reviews.
use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::OsStr,
    io::Read,
    path::{Component, Path, PathBuf},
};
use terminator_core::{CommandOptions, git};

/// Snapshot inputs are bounded to 1 MiB, like the callers this replaces.
pub const SNAPSHOT_LIMIT: usize = 1024 * 1024;

fn fork_options(options: &CommandOptions) -> CommandOptions {
    CommandOptions {
        timeout: options.timeout,
        input: options.input.clone(),
        stdout_limit: options.stdout_limit,
        stderr_limit: options.stderr_limit,
        accepted_exit_codes: options.accepted_exit_codes.clone(),
    }
}

/// Resolve `path` against `root` to the repository-relative Git entry.
/// Directory aliases resolve, but the entry itself must not be a symlink:
/// resolving the final component could silently review a link's target.
pub fn relative_path(root: &Path, path: &Path) -> Result<PathBuf> {
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

/// Read a worktree file for review. Missing files read as empty (deletions);
/// symlinks, submodules, and oversized files are refused.
pub fn worktree_file(path: &Path) -> Result<Vec<u8>> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        meta.is_file(),
        "Diff review supports regular files, not symlinks or submodules"
    );
    ensure!(
        meta.len() <= SNAPSHOT_LIMIT as u64,
        "Diff file exceeds 1 MiB"
    );
    let mut bytes = vec![];
    std::fs::File::open(path)?
        .take((SNAPSHOT_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= SNAPSHOT_LIMIT, "Diff file exceeds 1 MiB");
    Ok(bytes)
}

/// Snapshot bytes must be NUL-free UTF-8 before rendering. Callers report
/// their own viewer name on failure.
pub fn check_text(bytes: &[u8]) -> bool {
    !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok()
}

/// Find the before/after blob ids for `path` in `diff --raw -z` output.
/// Renames match on the target path; conflicts, symlinks, and submodules are
/// refused with the same messages the viewers already report.
pub fn find_record(raw: &[u8], path: &Path) -> Result<Option<(String, String)>> {
    let mut fields = raw.split(|b| *b == 0).filter(|f| !f.is_empty());
    while let Some(header) = fields.next() {
        let header = std::str::from_utf8(header)?;
        let parts: Vec<_> = header.split_whitespace().collect();
        let Some(&[old_mode, new_mode, before, after, status]) = parts.get(..5) else {
            bail!("Invalid Git diff record");
        };
        if parts.len() != 5 {
            bail!("Invalid Git diff record");
        }
        let first = fields.next().context("Missing Git path")?;
        let target = if status.starts_with(['R', 'C']) {
            fields.next().context("Missing rename target")?
        } else {
            first
        };
        if target != terminator_core::os_bytes(path.as_os_str()).as_ref() {
            continue;
        }
        ensure!(
            !status.starts_with('U'),
            "Resolve this file's merge conflict in your editor before opening a two-way diff"
        );
        ensure!(
            old_mode != ":160000"
                && new_mode != "160000"
                && old_mode != ":120000"
                && new_mode != "120000",
            "Diff review supports regular files, not symlinks or submodules"
        );
        return Ok(Some((before.to_owned(), after.to_owned())));
    }
    Ok(None)
}

fn raw_args(staged: bool) -> Vec<&'static OsStr> {
    let mut args: Vec<&'static OsStr> = vec![
        OsStr::new("diff"),
        OsStr::new("--raw"),
        OsStr::new("-z"),
        OsStr::new("--no-abbrev"),
        OsStr::new("--no-ext-diff"),
        OsStr::new("--no-textconv"),
        OsStr::new("--find-renames"),
    ];
    if staged {
        args.push(OsStr::new("--cached"));
    }
    args
}

/// Read one blob. All-zero ids (added/deleted sides) read as empty.
pub fn blob(root: &Path, oid: &str, options: &CommandOptions) -> Result<Vec<u8>> {
    if oid.bytes().all(|b| b == b'0') {
        return Ok(vec![]);
    }
    let bytes = git::run_blocking(root, &["cat-file", "blob", oid], fork_options(options))?;
    ensure!(bytes.len() <= SNAPSHOT_LIMIT, "Diff file exceeds 1 MiB");
    Ok(bytes)
}

/// Async [`blob`] through the shared pools.
pub async fn blob_async(
    processes: &terminator_core::async_process::Processes,
    files: &terminator_core::async_service::NativePool,
    root: &Path,
    oid: &str,
    options: CommandOptions,
) -> Result<Vec<u8>> {
    if oid.bytes().all(|b| b == b'0') {
        return Ok(Vec::new());
    }
    let bytes = git::run(
        processes,
        files,
        root,
        vec!["cat-file".into(), "blob".into(), oid.into()],
        options,
    )
    .await?;
    ensure!(bytes.len() <= SNAPSHOT_LIMIT, "Diff file exceeds 1 MiB");
    Ok(bytes)
}

/// Capture the before/after bytes for `path` (repository-relative). Staged
/// snapshots compare index sides; working snapshots compare the index side
/// against the worktree file. Untracked files snapshot as empty vs content.
pub fn snapshots(
    root: &Path,
    path: &Path,
    staged: bool,
    options: &CommandOptions,
) -> Result<(Vec<u8>, Vec<u8>)> {
    ensure!(
        !path.is_absolute() && path.components().all(|c| matches!(c, Component::Normal(_))),
        "Review path must be inside the repository"
    );
    let raw = git::run_blocking(root, &raw_args(staged), fork_options(options))?;
    match find_record(&raw, path)? {
        Some((before, after)) => {
            let left = blob(root, &before, options)?;
            let right = if staged {
                blob(root, &after, options)?
            } else {
                worktree_file(&root.join(path))?
            };
            Ok((left, right))
        }
        None if staged => bail!("This file has no staged changes; refresh Git status"),
        None => {
            let tracked = git::run_blocking(
                root,
                &[
                    OsStr::new("ls-files"),
                    OsStr::new("-z"),
                    OsStr::new("--"),
                    path.as_os_str(),
                ],
                fork_options(options),
            )?;
            ensure!(
                tracked.is_empty(),
                "This file has no working-tree changes; refresh Git status"
            );
            ensure!(
                root.join(path).exists(),
                "File no longer exists; refresh Git status"
            );
            Ok((vec![], worktree_file(&root.join(path))?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn options() -> CommandOptions {
        CommandOptions {
            stdout_limit: 4 * 1024 * 1024,
            ..Default::default()
        }
    }

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
    fn partially_staged_snapshots_show_the_correct_side() {
        let dir = repo();
        let root = dir.path();
        let name = Path::new("space file.txt");
        std::fs::write(root.join(name), "base\n").unwrap();
        git_cmd(root, &["add", "."]);
        git_cmd(root, &["commit", "-qm", "fixture"]);
        std::fs::write(root.join(name), "staged\n").unwrap();
        git_cmd(root, &["add", "."]);
        std::fs::write(root.join(name), "working\n").unwrap();
        let options = options();
        let (left, right) = snapshots(root, name, true, &options).unwrap();
        assert_eq!(left, b"base\n");
        assert_eq!(right, b"staged\n");
        let (left, right) = snapshots(root, name, false, &options).unwrap();
        assert_eq!(left, b"staged\n");
        assert_eq!(right, b"working\n");
    }

    #[test]
    #[cfg(unix)]
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
        for name in ["inside.rs", "outside.rs", "dangling.rs"] {
            for path in [PathBuf::from(name), root.join(name)] {
                let error = relative_path(root, &path).unwrap_err();
                assert!(
                    error.to_string().contains("symlinks"),
                    "{path:?}: {error:#}"
                );
            }
        }
    }

    #[test]
    #[cfg(unix)]
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
        let relative = relative_path(&alias, &alias.join("file.rs")).unwrap();
        assert_eq!(relative, PathBuf::from("file.rs"));
        let options = options();
        let (left, right) = snapshots(root, &relative, false, &options).unwrap();
        assert_eq!(left, b"old\n");
        assert!(right.is_empty());
    }

    #[test]
    fn untracked_and_binary_and_outside_paths_are_explicit() {
        let dir = repo();
        let root = dir.path();
        let options = options();
        std::fs::write(root.join("new.rs"), "fn new() {}\n").unwrap();
        let (left, right) = snapshots(root, Path::new("new.rs"), false, &options).unwrap();
        assert!(left.is_empty());
        assert!(check_text(&left) && check_text(&right));
        std::fs::write(root.join("new.rs"), b"binary\0").unwrap();
        let (_, right) = snapshots(root, Path::new("new.rs"), false, &options).unwrap();
        assert!(!check_text(&right));
        assert!(snapshots(root, Path::new("../outside.rs"), false, &options).is_err());
    }
}
