//! Read-only facts about one task worktree for the Worktrees page: its
//! source checkout (the main worktree), whether its branch is merged
//! there, its last commit time, and its size on disk. Blocking: it runs on
//! the file pool, never on the GUI thread.
use std::path::{Path, PathBuf};
use std::time::Duration;
use terminator_core::{CommandOptions, git, run_command};

/// Cap of one read-only Git call.
const READ_CAP: Duration = Duration::from_secs(20);
/// The size walk stops after this many entries and says so.
const SIZE_ENTRY_LIMIT: usize = 500_000;

/// What one probe read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Probe {
    /// The main checkout of the repository: where Apply to source writes.
    pub source: Option<PathBuf>,
    /// The branch checked out there, `None` when detached.
    pub source_branch: Option<String>,
    /// The worktree's `HEAD` commit, full hash.
    pub head: Option<String>,
    /// `HEAD` is an ancestor of the source checkout's `HEAD`.
    pub merged: Option<bool>,
    /// Unix time of the worktree's `HEAD` commit.
    pub last_commit: Option<u64>,
    /// Bytes on disk (symbolic links not followed), and whether the walk
    /// stopped early.
    pub size: Option<(u64, bool)>,
}

/// One quick read-only Git call; its trimmed stdout when it exits 0.
pub(super) fn git_out(cwd: &Path, args: &[&str]) -> Option<String> {
    let mut command = git::command(cwd, args);
    command.env("GIT_OPTIONAL_LOCKS", "0");
    let options = CommandOptions {
        timeout: READ_CAP,
        accepted_exit_codes: None,
        ..Default::default()
    };
    let output = run_command(command, options).ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// The first record of `git worktree list --porcelain -z`: the main
/// checkout's path and branch.
pub(crate) fn main_checkout(porcelain: &[u8]) -> Option<(PathBuf, Option<String>)> {
    let text = String::from_utf8_lossy(porcelain);
    let record: Vec<&str> = text.split('\0').take_while(|f| !f.is_empty()).collect();
    let path = record
        .iter()
        .find_map(|field| field.strip_prefix("worktree "))?;
    let branch = record
        .iter()
        .find_map(|field| field.strip_prefix("branch "))
        .map(|branch| {
            branch
                .strip_prefix("refs/heads/")
                .unwrap_or(branch)
                .to_owned()
        });
    Some((PathBuf::from(path), branch))
}

/// Reads every fact of the worktree at `path`.
pub(crate) fn probe(path: &Path) -> Probe {
    let mut probe = Probe::default();
    let list = git_out(path, &["worktree", "list", "--porcelain", "-z"]);
    if let Some((source, branch)) = list.as_deref().map(str::as_bytes).and_then(main_checkout) {
        probe.source = Some(source.canonicalize().unwrap_or(source));
        probe.source_branch = branch;
    }
    probe.head = git_out(path, &["rev-parse", "--verify", "--quiet", "HEAD"]);
    probe.last_commit =
        git_out(path, &["log", "-1", "--format=%ct", "HEAD"]).and_then(|time| time.parse().ok());
    if let (Some(head), Some(source)) = (&probe.head, &probe.source)
        && let Some(target) = git_out(source, &["rev-parse", "--verify", "--quiet", "HEAD"])
    {
        probe.merged = Some(is_ancestor(path, head, &target));
    }
    probe.size = Some(disk_size(path));
    probe
}

/// `ancestor` is reachable from `of` (`git merge-base --is-ancestor`).
fn is_ancestor(cwd: &Path, ancestor: &str, of: &str) -> bool {
    git_out(cwd, &["merge-base", "--is-ancestor", ancestor, of]).is_some()
}

/// Bytes of the files under `root`, without following links.
pub(crate) fn disk_size(root: &Path) -> (u64, bool) {
    let mut total = 0u64;
    let mut seen = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            seen = seen.saturating_add(1);
            if seen > SIZE_ENTRY_LIMIT {
                return (total, true);
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                stack.push(entry.path());
            } else if kind.is_file()
                && let Ok(metadata) = entry.metadata()
            {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    (total, false)
}
