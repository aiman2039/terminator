//! Fetch, Pull and Push from the Git panel. They wait on the network, so
//! each runs as its own cancellable GUI job (not the commit job): one at a
//! time per repository, a 5-minute cap, and Cancel sends Ctrl-C to the Git
//! process group, then kills it after 5 seconds. They stay `HEAD` writers
//! for `App::refuse_during_commit`.
use super::{BranchOp, Report};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};
use terminator_core::{
    CommandOptions,
    async_process::{AsyncProcessError, Processes},
    async_service::CancellationToken,
    git,
};

/// Cap of one Fetch, Pull or Push.
pub(crate) const SYNC_TIMEOUT: Duration = Duration::from_mins(5);
const SYNC_LIMIT: &str = "5 minutes";
/// Scheduling subsystem of the sync runs.
pub(crate) const SYNC_SUBSYSTEM: &str = "git-sync";
/// Status note when a second sync is asked for in the same repository.
pub(crate) const SYNC_BUSY: &str = "Wait for the running Fetch, Pull or Push to finish";
/// Output kept from one call.
const OUTPUT_LIMIT: usize = 1024 * 1024;

impl BranchOp {
    /// Fetch, Pull and Push run as a cancellable sync job.
    #[must_use]
    pub(crate) fn is_sync(&self) -> bool {
        matches!(self, Self::Fetch | Self::Pull | Self::Push)
    }
}

/// The reply of a sync that Cancel stopped.
pub(crate) fn sync_cancelled(op: &BranchOp) -> String {
    format!("{} cancelled", op.name())
}

/// The job payload of one sync.
#[derive(Clone, Debug)]
pub struct SyncRun {
    pub op: BranchOp,
    pub cancel: CancellationToken,
}

/// The sync in flight in one repository, as the Git panel shows it.
#[derive(Clone, Debug)]
pub struct SyncProgress {
    pub root: PathBuf,
    pub op: BranchOp,
    pub cancel: CancellationToken,
}

impl SyncProgress {
    /// The panel's progress line.
    #[must_use]
    pub fn line(&self) -> String {
        let verb = match (self.cancel.is_cancelled(), &self.op) {
            (true, _) => "Stopping",
            (false, BranchOp::Fetch) => "Fetching",
            (false, BranchOp::Pull) => "Pulling",
            (false, _) => "Pushing",
        };
        format!("{verb}…")
    }
}

/// The sync runs in flight, at most one per repository root.
#[derive(Debug, Default)]
pub struct SyncSlots(HashMap<PathBuf, SyncProgress>);

impl SyncSlots {
    /// Starts `op` in `root` unless a sync runs there: the double-start
    /// guard. Returns the job to queue.
    pub fn start(&mut self, root: &Path, op: BranchOp) -> Option<SyncRun> {
        if !op.is_sync() || self.0.contains_key(root) {
            return None;
        }
        let cancel = CancellationToken::new();
        let progress = SyncProgress {
            root: root.to_path_buf(),
            op: op.clone(),
            cancel: cancel.clone(),
        };
        self.0.insert(root.to_path_buf(), progress);
        Some(SyncRun { op, cancel })
    }

    #[must_use]
    pub fn progress(&self, root: &Path) -> Option<&SyncProgress> {
        self.0.get(root)
    }

    /// The job's reply ends the run in `root`.
    pub fn finish(&mut self, root: &Path) {
        self.0.remove(root);
    }
}

/// Runs one sync in `root`. Git never prompts.
pub(crate) async fn perform_sync(
    processes: &Processes,
    root: &Path,
    run: &SyncRun,
) -> Result<Report, String> {
    let git = SyncGit {
        processes,
        root,
        run,
    };
    let message = match run.op {
        BranchOp::Fetch => {
            git.long(&["fetch"]).await?;
            "Fetched".to_owned()
        }
        BranchOp::Pull => {
            git.long(&["pull", "--no-edit"]).await?;
            "Pulled".to_owned()
        }
        BranchOp::Push if git.has_upstream().await => {
            git.long(&["push"]).await?;
            "Pushed".to_owned()
        }
        BranchOp::Push => {
            if !git.has_origin().await {
                return Err(
                    "Push failed: Push needs an upstream branch or an origin remote".into(),
                );
            }
            git.long(&["push", "--set-upstream", "origin", "HEAD"])
                .await?;
            "Pushed and set the upstream on origin".to_owned()
        }
        _ => return Err("Only Fetch, Pull and Push run as a sync job".into()),
    };
    Ok(Report::Message(message))
}

struct SyncGit<'a> {
    processes: &'a Processes,
    root: &'a Path,
    run: &'a SyncRun,
}

impl SyncGit<'_> {
    /// A quick read; `true` when Git printed something.
    async fn read(&self, args: &[&str]) -> bool {
        let command = git::command(self.root, args);
        self.processes
            .run(command, CommandOptions::default(), None)
            .await
            .is_ok_and(|output| output.status.success() && !output.stdout.trim_ascii().is_empty())
    }

    async fn has_upstream(&self) -> bool {
        let args = [
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ];
        self.read(&args).await
    }

    async fn has_origin(&self) -> bool {
        self.read(&["remote", "get-url", "origin"]).await
    }

    /// The network call, with the sync cap and Cancel.
    async fn long(&self, args: &[&str]) -> Result<(), String> {
        let name = self.run.op.name();
        let mut command = git::command(self.root, args);
        command
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_EDITOR", "true")
            .env("GIT_MERGE_AUTOEDIT", "no");
        let options = CommandOptions {
            timeout: SYNC_TIMEOUT,
            input: None,
            stdout_limit: OUTPUT_LIMIT,
            stderr_limit: OUTPUT_LIMIT,
            accepted_exit_codes: None,
        };
        let result = self
            .processes
            .run_cancellable(command, options, &self.run.cancel)
            .await;
        match result {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => Err(format!("{name} failed{}", super::commit::failure(&output))),
            Err(_) if self.run.cancel.is_cancelled() => Err(sync_cancelled(&self.run.op)),
            Err(AsyncProcessError::TimedOut { .. }) => Err(format!(
                "{name} stopped after {SYNC_LIMIT}. Run git {} in a terminal for a slow remote",
                args.first().copied().unwrap_or_default()
            )),
            Err(error) => Err(format!("{name} failed: {error}")),
        }
    }
}

#[cfg(test)]
#[path = "sync_tests.rs"]
mod tests;
