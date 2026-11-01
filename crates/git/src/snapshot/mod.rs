//! Read-only snapshots for diff review: blob pairs with symlink, submodule,
//! size, and text checks shared by the native viewer and `CodeDiff` reviews.
use crate::{Error, Result};
use std::{
    ffi::{OsStr, OsString},
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
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(Error::NotRegularFile { path: absolute });
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let absolute = absolute
        .parent()
        .and_then(|parent| parent.canonicalize().ok())
        .and_then(|parent| absolute.file_name().map(|name| parent.join(name)))
        .unwrap_or(absolute);
    // Windows canonicalization adds a verbatim prefix. Resolve the root too
    // so Git's ordinary drive paths compare with canonical parent paths.
    let canonical_root = root.canonicalize()?;
    let relative = absolute
        .strip_prefix(&canonical_root)
        .or_else(|_| absolute.strip_prefix(root))
        .or_else(|_| path.strip_prefix(root))
        .map_err(|source| Error::OutsideRepository {
            context: "Diff file is outside the repository",
            source,
        })?;
    if !(relative
        .components()
        .all(|c| matches!(c, Component::Normal(_))))
    {
        return Err(Error::PathEscapes {
            path: relative.to_path_buf(),
        });
    }
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
    if !meta.is_file() {
        return Err(Error::NotRegularFile { path: path.into() });
    }
    if meta.len() > SNAPSHOT_LIMIT as u64 {
        return Err(Error::TooLarge { bytes: meta.len() });
    }
    let mut bytes = vec![];
    std::fs::File::open(path)?
        .take((SNAPSHOT_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > SNAPSHOT_LIMIT {
        return Err(too_large(&bytes));
    }
    Ok(bytes)
}

fn too_large(bytes: &[u8]) -> Error {
    Error::TooLarge {
        bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
    }
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
        let invalid = || Error::InvalidDiffRecord {
            header: header.to_owned(),
        };
        let Some(&[old_mode, new_mode, before, after, status]) = parts.get(..5) else {
            return Err(invalid());
        };
        if parts.len() != 5 {
            return Err(invalid());
        }
        let first = fields.next().ok_or_else(|| Error::MissingPath {
            header: header.to_owned(),
        })?;
        let target = if status.starts_with(['R', 'C']) {
            fields.next().ok_or_else(|| Error::MissingRenameTarget {
                header: header.to_owned(),
            })?
        } else {
            first
        };
        if target != terminator_core::os_bytes(path.as_os_str()).as_ref() {
            continue;
        }
        if status.starts_with('U') {
            return Err(Error::Conflicted { path: path.into() });
        }
        if !(old_mode != ":160000"
            && new_mode != "160000"
            && old_mode != ":120000"
            && new_mode != "120000")
        {
            return Err(Error::NotRegularFile { path: path.into() });
        }
        return Ok(Some((before.to_owned(), after.to_owned())));
    }
    Ok(None)
}

/// A pathspec that matches only `path`, never as a glob.
fn literal(path: &Path) -> OsString {
    let mut spec = OsString::from(":(literal)");
    spec.push(path);
    spec
}

/// The diff record for `path`. The diff is first limited to `path`, so one
/// file does not need a full-repository scan. Only an addition can be a
/// rename target; then the full rename scan runs. When that scan fails
/// (output too large, timeout), the addition record is kept.
fn path_record(
    root: &Path,
    path: &Path,
    staged: bool,
    options: &CommandOptions,
) -> Result<Option<(String, String)>> {
    let spec = literal(path);
    let mut args: Vec<&OsStr> = raw_args(staged);
    args.extend([OsStr::new("--"), spec.as_os_str()]);
    let scoped = find_record(
        &git::run_blocking(root, &args, fork_options(options))?,
        path,
    )?;
    let Some((before, after)) = scoped else {
        return Ok(None);
    };
    if !before.bytes().all(|b| b == b'0') {
        return Ok(Some((before, after)));
    }
    match git::run_blocking(root, &raw_args(staged), fork_options(options)) {
        Ok(raw) => Ok(find_record(&raw, path)?.or(Some((before, after)))),
        Err(_) => Ok(Some((before, after))),
    }
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
    if bytes.len() > SNAPSHOT_LIMIT {
        return Err(too_large(&bytes));
    }
    Ok(bytes)
}

/// Async [`blob`] through the shared pools.
pub struct BlobAsyncInput<'a> {
    pub root: &'a Path,
    pub oid: &'a str,
    pub options: CommandOptions,
}

pub async fn blob_async(
    processes: &terminator_core::async_process::Processes,
    files: &terminator_core::async_service::NativePool,
    input: BlobAsyncInput<'_>,
) -> Result<Vec<u8>> {
    let BlobAsyncInput { root, oid, options } = input;
    if oid.bytes().all(|b| b == b'0') {
        return Ok(Vec::new());
    }
    let bytes = git::run(
        processes,
        files,
        git::RunInput {
            cwd: root,
            args: vec!["cat-file".into(), "blob".into(), oid.into()],
            options,
        },
    )
    .await?;
    if bytes.len() > SNAPSHOT_LIMIT {
        return Err(too_large(&bytes));
    }
    Ok(bytes)
}

/// Capture the before/after bytes for `path` (repository-relative). Staged
/// snapshots compare index sides; working snapshots compare the index side
/// against the worktree file. Untracked files snapshot as empty vs content.
pub struct SnapshotsInput<'a> {
    pub root: &'a Path,
    pub path: &'a Path,
    pub staged: bool,
    pub options: &'a CommandOptions,
}

pub fn snapshots(input: SnapshotsInput<'_>) -> Result<(Vec<u8>, Vec<u8>)> {
    let SnapshotsInput {
        root,
        path,
        staged,
        options,
    } = input;
    if !(!path.is_absolute() && path.components().all(|c| matches!(c, Component::Normal(_)))) {
        return Err(Error::PathEscapes { path: path.into() });
    }
    match path_record(root, path, staged, options)? {
        Some((before, after)) => {
            let left = blob(root, &before, options)?;
            let right = if staged {
                blob(root, &after, options)?
            } else {
                worktree_file(&root.join(path))?
            };
            Ok((left, right))
        }
        None if staged => Err(Error::NoStagedChanges { path: path.into() }),
        None => {
            let tracked = git::run_blocking(
                root,
                &[
                    OsStr::new("ls-files"),
                    OsStr::new("-z"),
                    OsStr::new("--"),
                    literal(path).as_os_str(),
                ],
                fork_options(options),
            )?;
            if !tracked.is_empty() {
                return Err(Error::NoWorkingChanges { path: path.into() });
            }
            if !root.join(path).exists() {
                return Err(Error::Deleted { path: path.into() });
            }
            Ok((vec![], worktree_file(&root.join(path))?))
        }
    }
}

#[cfg(test)]
mod tests;
