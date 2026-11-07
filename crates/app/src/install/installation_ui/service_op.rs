//! One session-service operation at a time. Restart, repair and start
//! share the service; a second one is refused while the first runs.
use crate::{Job, Update};
use terminator_core::async_service::{Failure, OperationContext};

/// The session-service operation in progress.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ServiceOp {
    #[default]
    Idle,
    /// The "Restart session service?" confirmation is open.
    ConfirmingRestart,
    /// The restart helper runs. Success closes this GUI.
    Restarting,
    Repairing,
    Starting,
}

impl ServiceOp {
    /// A worker job runs: no other service operation may begin.
    pub(crate) fn running(self) -> bool {
        matches!(self, Self::Restarting | Self::Repairing | Self::Starting)
    }

    /// Job tag that lets a failed or timed-out job still answer its
    /// operation. These jobs share the workspace queue, so the scheduling
    /// key cannot carry it.
    pub(crate) fn job_tag(self) -> Option<&'static str> {
        match self {
            Self::Restarting => Some("service-op:restart"),
            Self::Repairing => Some("service-op:repair"),
            Self::Starting => Some("service-op:start"),
            Self::Idle | Self::ConfirmingRestart => None,
        }
    }
}

/// Which service operation a recorded failure came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ServiceTask {
    /// The launch-time service upgrade (kept in `service-upgrade-error.txt`).
    Upgrade,
    Repair,
    Restart,
    Start,
}

/// The last failed service operation. It is kept apart from the status
/// line, so a later unrelated error does not hide its retry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ServiceFailure {
    pub(crate) task: ServiceTask,
    pub(crate) message: String,
}

impl ServiceFailure {
    /// "Retry service update" applies to a failed upgrade or repair.
    pub(crate) fn retries_update(&self) -> bool {
        matches!(self.task, ServiceTask::Upgrade | ServiceTask::Repair)
    }
}

/// Context tag for a service job, so a failed job still answers its
/// operation.
pub(crate) fn job_tag(job: &Job) -> Option<String> {
    match job {
        Job::RepairInstallation(..) => ServiceOp::Repairing.job_tag(),
        Job::RestartSessionService(_) => ServiceOp::Restarting.job_tag(),
        Job::StartSessionService => ServiceOp::Starting.job_tag(),
        _ => None,
    }
    .map(Into::into)
}

/// The completion a failed service job owes its operation, so the GUI
/// does not wait for a reply that never comes.
pub(crate) fn failure_update(context: &OperationContext, error: &Failure) -> Option<Update> {
    let error = error.to_string();
    match context.tab.as_deref()? {
        tag if Some(tag) == ServiceOp::Restarting.job_tag() => Some(Update::RestartFinished(error)),
        tag if Some(tag) == ServiceOp::Repairing.job_tag() => {
            Some(Update::InstallationRepaired(Err(error)))
        }
        tag if Some(tag) == ServiceOp::Starting.job_tag() => {
            Some(Update::ServiceStarted(Err(error)))
        }
        _ => None,
    }
}
