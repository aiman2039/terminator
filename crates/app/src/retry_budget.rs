//! Bound retries for missing directories and pace terminal reattachment.
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(crate) const MISSING_PATH_RETRY_LIMIT: u8 = 3;
const ATTACH_FAILURE_WINDOW: Duration = Duration::from_secs(1);
pub(crate) const ATTACH_STABLE_WINDOW: Duration = Duration::from_secs(10);

/// Attachment failures never permanently disable a terminal. Each visible
/// terminal retries independently, without restarting its daemon-owned shell.
#[derive(Clone, Debug, Default)]
pub(crate) struct AttachmentRetry {
    failures: u8,
    retry_at: Option<Instant>,
}

impl AttachmentRetry {
    pub(crate) fn record_failure(&mut self, now: Instant) {
        let seconds = (1_u64 << self.failures.min(5)).min(30);
        self.failures = self.failures.saturating_add(1);
        self.retry_at = now.checked_add(Duration::from_secs(seconds));
    }

    pub(crate) fn remaining(&self, now: Instant) -> Option<Duration> {
        self.retry_at
            .and_then(|at| at.checked_duration_since(now))
            .filter(|remaining| !remaining.is_zero())
    }
}

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

/// Failed bridge exits always consume the budget, even after a slow connection.
pub(crate) fn attach_exit_is_failure(lifetime: Option<Duration>, failed: bool) -> bool {
    failed || lifetime.is_none_or(|lifetime| lifetime < ATTACH_FAILURE_WINDOW)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_retries_continue_after_three_failures_with_a_capped_delay() {
        let mut retry = AttachmentRetry::default();
        let mut now = Instant::now();
        assert_eq!(retry.remaining(now), None);
        for seconds in [1, 2, 4, 8, 16, 30, 30, 30] {
            retry.record_failure(now);
            let delay = Duration::from_secs(seconds);
            assert_eq!(retry.remaining(now), Some(delay));
            assert!(
                retry
                    .remaining((now + delay).checked_sub(Duration::from_nanos(1)).unwrap())
                    .is_some()
            );
            now += delay;
            assert_eq!(retry.remaining(now), None, "Retry needs no user action");
        }
        // Persistent failure cannot overflow the counter or increase the cap.
        for _ in 0..300 {
            retry.record_failure(now);
        }
        assert_eq!(retry.remaining(now), Some(Duration::from_secs(30)));
    }

    #[test]
    fn attachment_failures_do_not_delay_other_terminals() {
        let now = Instant::now();
        let mut failed = AttachmentRetry::default();
        failed.record_failure(now);
        let healthy = AttachmentRetry::default();
        assert!(failed.remaining(now).is_some());
        assert!(healthy.remaining(now).is_none());
    }

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
        assert!(attach_exit_is_failure(None, false));
        assert!(attach_exit_is_failure(
            Some(Duration::from_millis(200)),
            false
        ));
        assert!(!attach_exit_is_failure(Some(Duration::from_secs(1)), false));
    }

    #[test]
    fn slow_failed_attachments_still_count_as_failures() {
        assert!(attach_exit_is_failure(Some(Duration::from_secs(5)), true));
    }

    #[test]
    fn path_missing_is_not_found_only() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!path_missing(dir.path()));
        assert!(path_missing(&dir.path().join("deleted-worktree")));
    }
}
