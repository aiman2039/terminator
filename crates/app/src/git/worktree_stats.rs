//! Branch and `+added −deleted` for worktree rows, from a read-only Git
//! job in the GUI (never a daemon request, never on the GUI thread). A
//! painted row asks again at most once per [`REFRESH`].
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The shortest time between two jobs for one worktree.
pub(crate) const REFRESH: Duration = Duration::from_secs(30);

/// What one job read from a worktree.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct WorktreeGit {
    /// The checked-out branch, `None` when HEAD is detached.
    pub branch: Option<String>,
    /// Lines added and deleted by uncommitted changes against HEAD.
    pub added: u64,
    pub deleted: u64,
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
        };
        stats.apply(path.clone(), Some(git.clone()));
        stats.apply(path.clone(), None);
        assert_eq!(stats.get(&path), Some(&git));
        assert_eq!(git.branch_leaf(), Some("fix-login"));
    }
}
