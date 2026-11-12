//! Branch and `+added −deleted` for worktree rows, from a read-only Git
//! job in the GUI (never a daemon request, never on the GUI thread). A
//! painted row asks again at most once per [`REFRESH`].
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The shortest time between two jobs for one worktree.
pub(crate) const REFRESH: Duration = Duration::from_secs(30);

/// The commit a task worktree started from, saved when New task made it
/// (GUI preferences, by worktree path).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct TaskBase {
    /// The Start-from branch or revision, for the label (`main`).
    pub name: String,
    /// Full hash of that commit.
    pub commit: String,
}

/// What one job read from a worktree.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct WorktreeGit {
    /// The checked-out branch, `None` when HEAD is detached.
    pub branch: Option<String>,
    /// Lines added and deleted against the task base (committed work
    /// included), else uncommitted changes against HEAD.
    pub added: u64,
    pub deleted: u64,
    /// The verified task base commit the numbers count from.
    pub base: Option<String>,
    /// What that base is called when it came from the main checkout's
    /// branch (no saved task base).
    pub base_name: Option<String>,
}

impl WorktreeGit {
    /// The last part of the branch: `fix-login` for `user/fix-login`.
    pub(crate) fn branch_leaf(&self) -> Option<&str> {
        self.branch
            .as_deref()
            .map(|branch| branch.rsplit('/').next().unwrap_or(branch))
            .filter(|leaf| !leaf.is_empty())
    }
}

#[derive(Default)]
pub(crate) struct WorktreeStats {
    entries: HashMap<PathBuf, Entry>,
}

#[derive(Default)]
struct Entry {
    git: Option<WorktreeGit>,
    asked: Option<Instant>,
}

impl WorktreeStats {
    /// True when a job for `path` may start now; it then counts as asked.
    pub(crate) fn take_due(&mut self, path: &Path, now: Instant) -> bool {
        let entry = self.entries.entry(path.to_owned()).or_default();
        let due = entry
            .asked
            .is_none_or(|asked| now.saturating_duration_since(asked) >= REFRESH);
        if due {
            entry.asked = Some(now);
        }
        due
    }

    /// A job's result; `None` (not a Git checkout, or Git failed) keeps
    /// the last good value.
    pub(crate) fn apply(&mut self, path: PathBuf, git: Option<WorktreeGit>) {
        if let Some(git) = git {
            self.entries.entry(path).or_default().git = Some(git);
        }
    }

    pub(crate) fn get(&self, path: &Path) -> Option<&WorktreeGit> {
        self.entries.get(path).and_then(|entry| entry.git.as_ref())
    }
}

/// The branch of the main checkout: the first record of `git worktree
/// list --porcelain -z`.
pub(crate) fn main_branch(porcelain: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(porcelain);
    text.split('\0')
        .take_while(|field| !field.is_empty())
        .find_map(|field| field.strip_prefix("branch "))
        .map(|branch| {
            branch
                .strip_prefix("refs/heads/")
                .unwrap_or(branch)
                .to_owned()
        })
}

/// Lines added and deleted in `git diff --shortstat` output, such as
/// ` 3 files changed, 128 insertions(+), 12 deletions(-)`.
pub(crate) fn parse_shortstat(text: &str) -> (u64, u64) {
    let mut added = 0;
    let mut deleted = 0;
    for part in text.split(',') {
        let mut words = part.split_whitespace();
        let (Some(count), Some(kind)) = (words.next(), words.next()) else {
            continue;
        };
        let Ok(count) = count.parse::<u64>() else {
            continue;
        };
        if kind.starts_with("insertion") {
            added = count;
        } else if kind.starts_with("deletion") {
            deleted = count;
        }
    }
    (added, deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortstat_counts_insertions_and_deletions() {
        assert_eq!(
            parse_shortstat(" 3 files changed, 128 insertions(+), 12 deletions(-)\n"),
            (128, 12)
        );
        assert_eq!(parse_shortstat(" 1 file changed, 1 insertion(+)"), (1, 0));
        assert_eq!(parse_shortstat(" 2 files changed, 4 deletions(-)"), (0, 4));
        assert_eq!(parse_shortstat(""), (0, 0));
    }

    #[test]
    fn the_main_branch_is_the_first_worktree_record() {
        let porcelain = b"worktree /r\0HEAD abc\0branch refs/heads/main\0\0\
            worktree /w\0HEAD def\0branch refs/heads/terminator/fix\0\0";
        assert_eq!(main_branch(porcelain).as_deref(), Some("main"));
        let detached = b"worktree /r\0HEAD abc\0detached\0\0worktree /w\0branch refs/heads/x\0\0";
        assert_eq!(main_branch(detached), None);
    }

    #[test]
    fn a_worktree_job_starts_at_most_once_per_refresh() {
        let mut stats = WorktreeStats::default();
        let path = Path::new("/w/fix");
        let start = Instant::now();
        assert!(stats.take_due(path, start));
        assert!(!stats.take_due(path, start + Duration::from_secs(5)));
        assert!(stats.take_due(path, start + REFRESH));
    }

    #[test]
    fn a_failed_job_keeps_the_last_numbers_and_the_branch_leaf_drops_the_prefix() {
        let mut stats = WorktreeStats::default();
        let path = PathBuf::from("/w/fix");
        let git = WorktreeGit {
            branch: Some("user/fix-login".into()),
            added: 3,
            deleted: 1,
            ..WorktreeGit::default()
        };
        stats.apply(path.clone(), Some(git.clone()));
        stats.apply(path.clone(), None);
        assert_eq!(stats.get(&path), Some(&git));
        assert_eq!(git.branch_leaf(), Some("fix-login"));
    }
}
