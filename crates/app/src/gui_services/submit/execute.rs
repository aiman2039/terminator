//! Job execution: per-editor serialization, then topic dispatch.
use super::Services;
use crate::gui_services::jobs::JobRouting;
use crate::{Job, Update};
use anyhow::Result;
use std::sync::Arc;
use terminator_core::async_service::CancellationToken;

pub(super) const MISROUTED: &str = "Job was routed to the wrong service topic";

impl Services {
    pub(super) async fn execute(&self, job: Job, cancel: CancellationToken) -> Result<Vec<Update>> {
        let _guards = self.lock_editors(&job).await;
        if let Job::Control(request, _) = &job
            && let Some(updates) = self.reuse_review(request, &cancel).await?
        {
            return Ok(updates);
        }
        let mut updates = Vec::new();
        self.dispatch(job, cancel, &mut updates).await?;
        Ok(updates)
    }

    /// Serializes jobs that touch the same editor sessions.
    async fn lock_editors(&self, job: &Job) -> Vec<tokio::sync::OwnedMutexGuard<()>> {
        let ids = JobRouting::editor_ids(job);
        let locks: Vec<_> = {
            let mut editors = self
                .0
                .editors
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            editors.retain(|_, lock| lock.strong_count() != 0);
            ids.iter()
                .map(|id| {
                    let lock = editors
                        .get(id)
                        .and_then(std::sync::Weak::upgrade)
                        .unwrap_or_default();
                    editors.insert(id.clone(), Arc::downgrade(&lock));
                    lock
                })
                .collect()
        };
        let mut guards = Vec::new();
        for lock in locks {
            guards.push(lock.lock_owned().await);
        }
        guards
    }

    async fn dispatch(
        &self,
        job: Job,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        match job {
            job @ (Job::PrepareLayouts(..)
            | Job::SaveLayout(..)
            | Job::Control(..)
            | Job::CloseIdle(..)
            | Job::CloseEditors(..)
            | Job::ExitSave(..)
            | Job::ExitDrain(..)
            | Job::RepairInstallation(..)
            | Job::RestartSessionService(..)
            | Job::StartSessionService) => self.session_job(job, cancel, updates).await,
            job @ (Job::Preferences(..)
            | Job::SaveAppearance(..)
            | Job::MigrateAttention
            | Job::MigrateTypography
            | Job::HookStatus
            | Job::Install(..)
            | Job::TestNtfy { .. }) => self.settings_job(job, cancel, updates).await,
            job @ (Job::ResolveTarget(..)
            | Job::OpenProject(..)
            | Job::Workspace(..)
            | Job::Search { .. }
            | Job::LinePreviews(..)
            | Job::PasteClipboard(..)
            | Job::Browser(..)
            | Job::External(..)
            | Job::TestExternal(..)) => self.files_job(job, cancel, updates).await,
            job @ (Job::DiffAction(..)
            | Job::DiffContext(..)
            | Job::DiffExpand(..)
            | Job::Diff(..)
            | Job::CreateWorktree(..)
            | Job::CommitFiles(..)
            | Job::EditorBlame(..)
            | Job::GitBase(..)
            | Job::GitView(..)) => self.git_job(job, cancel, updates).await,
            job @ (Job::RemoteAdd { .. }
            | Job::RemoteRead { .. }
            | Job::HostKey { .. }
            | Job::SshConfigHosts(..)
            | Job::Forwards(..)
            | Job::SshControlStatus(..)
            | Job::RemoteAsset { .. }
            | Job::RemoteTransfer { .. }
            | Job::RemoteTransferPoll { .. }) => self.remote_job(job, cancel, updates).await,
            job @ (Job::VncPickTransfer { .. }
            | Job::VncTransferPoll { .. }
            | Job::VncConnectTab { .. }
            | Job::VncFrame { .. }) => self.vnc_job(job, cancel, updates).await,
        }
    }
}
