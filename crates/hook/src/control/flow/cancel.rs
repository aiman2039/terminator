//! Cancel during launches: a launch can take seconds (a worktree, a new
//! terminal), so the runner checks Cancel and the signals before each step
//! it starts, and again after a step's worktree setup. A launch that Cancel
//! stopped starts no terminal; its step stays waiting until the run's
//! cancel marks it cancelled.
use super::runner::Runner;

/// A launch that Cancel or a signal stopped before its terminal existed.
#[derive(Debug)]
pub(super) struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("The run was cancelled before this step started")
    }
}

impl std::error::Error for Cancelled {}

impl Runner {
    /// Starts each step that can start, and stops at the first Cancel.
    pub(super) fn start_ready(&mut self) {
        for step in self.ready() {
            if self.cancelled() {
                return;
            }
            self.start(&step);
        }
    }
}
