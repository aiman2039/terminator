//! Long Perforce actions (Sync, Submit, Shelve, Unshelve) are network
//! transfers. They run as cancellable GUI jobs, like the Git panel commit:
//! one at a time, a 30-minute cap, and Cancel sends Ctrl-C to the `p4`
//! process group, then kills it after 5 seconds. Read queries keep their
//! short caps.
use super::Op;
use std::time::Duration;
use terminator_core::async_service::CancellationToken;

/// Cap of one long Perforce action.
pub const RUN_TIMEOUT: Duration = Duration::from_mins(30);
/// Scheduling subsystem of the long Perforce runs.
pub const RUN_SUBSYSTEM: &str = "perforce-run";
/// The reply of a run that Cancel stopped.
pub const RUN_CANCELLED: &str = "Perforce action cancelled; check the workspace before retrying";
/// Status note when a second long action is asked for.
pub const RUN_BUSY: &str = "Wait for the running Perforce action to finish";

impl Op {
    /// The `p4` command name of this action.
    #[must_use]
    pub fn verb(&self) -> &'static str {
        match self {
            Self::Edit(_) => "edit",
            Self::Reconcile(_) => "reconcile",
            Self::Sync => "sync",
            Self::Submit(_) => "submit",
            Self::Shelve(_) => "shelve",
            Self::Unshelve(_) => "unshelve",
        }
    }

    /// Network transfers that run as a cancellable job.
    #[must_use]
    pub fn is_long(&self) -> bool {
        matches!(
            self,
            Self::Sync | Self::Submit(_) | Self::Shelve(_) | Self::Unshelve(_)
        )
    }
}

/// The job payload of a long action.
#[derive(Clone, Debug)]
pub struct Run {
    pub op: Op,
    pub cancel: CancellationToken,
}

/// The long action in flight, as the Perforce panel shows it.
#[derive(Clone, Debug)]
pub struct Progress {
    pub verb: &'static str,
    pub cancel: CancellationToken,
}

impl Progress {
    /// The panel's progress line.
    #[must_use]
    pub fn line(&self) -> String {
        if self.cancel.is_cancelled() {
            format!("Stopping p4 {}…", self.verb)
        } else {
            format!("Running p4 {}…", self.verb)
        }
    }
}

/// The one long Perforce action that may run.
#[derive(Debug, Default)]
pub struct RunSlot(Option<Progress>);

impl RunSlot {
    /// Starts `op` unless a run is in flight: the double-start guard.
    /// Returns the job to queue.
    pub fn start(&mut self, op: Op) -> Option<Run> {
        if self.0.is_some() || !op.is_long() {
            return None;
        }
        let cancel = CancellationToken::new();
        self.0 = Some(Progress {
            verb: op.verb(),
            cancel: cancel.clone(),
        });
        Some(Run { op, cancel })
    }

    #[must_use]
    pub fn progress(&self) -> Option<&Progress> {
        self.0.as_ref()
    }

    /// The job's reply ends the run.
    pub fn finish(&mut self) {
        self.0 = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_network_transfers_run_as_long_jobs() {
        assert!(Op::Sync.is_long());
        for op in [Op::Submit("1".into()), Op::Shelve("1".into())] {
            assert!(op.is_long());
        }
        assert!(Op::Unshelve("1".into()).is_long());
        assert!(!Op::Edit("/a".into()).is_long());
        assert!(!Op::Reconcile("/a".into()).is_long());
    }

    #[test]
    fn a_second_long_perforce_action_is_refused_while_one_runs() {
        let mut slot = RunSlot::default();
        assert!(slot.start(Op::Edit("/ws/a".into())).is_none());
        let run = slot.start(Op::Sync).unwrap();
        assert!(slot.start(Op::Submit("2".into())).is_none());
        let progress = slot.progress().unwrap();
        assert_eq!(progress.line(), "Running p4 sync…");
        // Cancel from the panel stops the job's process group.
        progress.cancel.cancel();
        assert!(run.cancel.is_cancelled());
        assert_eq!(slot.progress().unwrap().line(), "Stopping p4 sync…");
        // The run stays until the job replies.
        slot.finish();
        assert!(slot.progress().is_none());
        assert!(slot.start(Op::Shelve("2".into())).is_some());
    }
}
