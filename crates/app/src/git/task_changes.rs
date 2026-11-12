//! Changes since task start: the files a task worktree changed from its
//! task base (the commit New task started from; else the merge base with
//! the main checkout's branch) to the working tree, committed work
//! included. Read-only Git on a worker: never a daemon request, never the
//! GUI thread, never a Git mutation. A row opens the native revision diff
//! of that file (base against the working file).
use crate::*;
use std::collections::HashMap;
use terminator_core::CommandOptions;

/// One changed file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TaskFile {
    /// Git status letter; `?` for an untracked file.
    pub status: char,
    pub path: PathBuf,
    /// Lines added and deleted; `None` for an untracked or binary file.
    pub lines: Option<(u64, u64)>,
}

/// Data of the Changes since task start page.
pub(crate) struct TaskChanges {
    pub worktree: PathBuf,
    /// The verified base commit.
    pub base: String,
    /// What the base is called: the Start-from branch, or a short hash.
    pub label: String,
    pub files: Option<Result<Vec<TaskFile>, String>>,
}

/// Quick read-only Git calls: a hung call fails after 20 seconds.
fn options() -> CommandOptions {
    CommandOptions {
        stdout_limit: 4 * 1024 * 1024,
        timeout: std::time::Duration::from_secs(20),
        ..Default::default()
    }
}

/// The task base of a new checkout: the commit it starts at, named by
/// the Start-from revision. `HEAD` names the source checkout's branch then,
/// since "HEAD" says nothing later.
pub(crate) fn task_base(
    source: &Path,
    checkout: &Path,
    start: &str,
) -> Option<crate::worktree_stats::TaskBase> {
    let git = |cwd: &Path, args: &[&str]| {
        terminator_core::git::run_blocking(cwd, args, options())
            .ok()
            .map(|bytes| String::from_utf8_lossy(&bytes).trim().to_owned())
            .filter(|text| !text.is_empty())
    };
    let commit = git(checkout, &["rev-parse", "--verify", "HEAD^{commit}"])?;
    let start = start.trim();
    let name = if start.is_empty() || start == "HEAD" {
        git(source, &["symbolic-ref", "--short", "-q", "HEAD"])
            .unwrap_or_else(|| commit.get(..8).unwrap_or(&commit).to_owned())
    } else {
        start.to_owned()
    };
    Some(crate::worktree_stats::TaskBase { name, commit })
}

/// Files changed from `base` to the working tree of `worktree`, then the
/// untracked files. The base must pass the Compare with… check.
pub(crate) fn list_changes(worktree: &Path, base: &str) -> Result<Vec<TaskFile>, String> {
    let base = crate::revision_compare::verify_revision(worktree, base)?;
    let git = |args: &[&str]| {
        terminator_core::git::run_blocking(worktree, args, options())
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .map_err(|error| format!("git {}: {error}", args.first().unwrap_or(&"")))
    };
    let status = git(&["diff", "--no-renames", "--name-status", "-z", &base, "--"])?;
    let numstat = git(&["diff", "--no-renames", "--numstat", "-z", &base, "--"])?;
    let untracked = git(&["ls-files", "--others", "--exclude-standard", "-z"])?;
    Ok(parse_changes(worktree, (&status, &numstat, &untracked)))
}

/// `--name-status -z`, `--numstat -z` and `ls-files -z` output, joined.
pub(crate) fn parse_changes(
    root: &Path,
    (status, numstat, untracked): (&str, &str, &str),
) -> Vec<TaskFile> {
    let mut counts = HashMap::new();
    for record in numstat.split('\0').filter(|record| !record.is_empty()) {
        let mut parts = record.trim_start_matches('\n').splitn(3, '\t');
        if let (Some(added), Some(deleted), Some(path)) = (parts.next(), parts.next(), parts.next())
        {
            counts.insert(path, added.parse().ok().zip(deleted.parse().ok()));
        }
    }
    let mut fields = status
        .split('\0')
        .map(|field| field.trim_start_matches('\n'))
        .filter(|field| !field.is_empty());
    let mut files = Vec::new();
    while let (Some(letter), Some(path)) = (fields.next(), fields.next()) {
        files.push(TaskFile {
            status: letter.chars().next().unwrap_or('M'),
            path: root.join(path),
            lines: counts.get(path).copied().flatten(),
        });
    }
    for path in untracked.split('\0').filter(|path| !path.is_empty()) {
        files.push(TaskFile {
            status: '?',
            path: root.join(path),
            lines: None,
        });
    }
    files
}

/// Lines added and deleted over all files.
pub(crate) fn totals(files: &[TaskFile]) -> (u64, u64) {
    files
        .iter()
        .filter_map(|file| file.lines)
        .fold((0, 0), |(a, d), (added, deleted)| {
            (a.saturating_add(added), d.saturating_add(deleted))
        })
}
