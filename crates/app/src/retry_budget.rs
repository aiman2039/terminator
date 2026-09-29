//! Stop hammering a working directory that is gone.
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub(crate) const MISSING_PATH_RETRY_LIMIT: u8 = 3;
const ATTACH_FAILURE_WINDOW: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RetryBudget {
    cwd: PathBuf,
    generation: u64,
    attempts: u8,
    started: bool,
}

impl Default for RetryBudget {
    fn default() -> Self {
        Self {
            cwd: PathBuf::new(),
            generation: 0,
            attempts: 0,
            started: false,
        }
    }
}

impl RetryBudget {
    pub(crate) fn exhausted(&self, cwd: &Path, generation: u64) -> bool {
        self.started
            && self.cwd == cwd
            && self.generation == generation
            && self.attempts >= MISSING_PATH_RETRY_LIMIT
    }

    pub(crate) fn record(&mut self, cwd: &Path, generation: u64, missing: bool) {
        if !self.started || self.cwd != cwd || self.generation != generation {
            self.started = true;
            self.cwd = cwd.to_path_buf();
            self.generation = generation;
            self.attempts = 0;
        }
        if missing {
            self.attempts = self.attempts.saturating_add(1);
        } else {
            self.attempts = 0;
        }
    }
}

pub(crate) fn path_missing(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(_) => false,
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    }
}

/// An attach that dies before the shell has been up counts as a failed retry.
pub(crate) fn attach_exit_is_failure(lifetime: Option<Duration>) -> bool {
    !lifetime.is_some_and(|lifetime| lifetime >= ATTACH_FAILURE_WINDOW)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_path_stops_after_three_attempts_until_the_request_changes() {
        let gone = Path::new("/gone/worktree");
        let mut budget = RetryBudget::default();
        for _ in 0..MISSING_PATH_RETRY_LIMIT {
            assert!(!budget.exhausted(gone, 1));
            budget.record(gone, 1, true);
        }
        assert!(budget.exhausted(gone, 1));
        budget.record(gone, 2, true);
        assert!(!budget.exhausted(gone, 2));
        budget.record(gone, 2, false);
        assert!(!budget.exhausted(gone, 2));
        budget.record(Path::new("/other"), 2, true);
        assert!(!budget.exhausted(Path::new("/other"), 2));
    }

    #[test]
    fn attach_exit_within_a_second_is_a_failed_retry() {
        assert!(attach_exit_is_failure(None));
        assert!(attach_exit_is_failure(Some(Duration::from_millis(200))));
        assert!(!attach_exit_is_failure(Some(Duration::from_secs(1))));
    }

    #[test]
    fn path_missing_is_not_found_only() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!path_missing(dir.path()));
        assert!(path_missing(&dir.path().join("deleted-worktree")));
    }
}
