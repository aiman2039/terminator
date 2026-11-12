//! Fetch, Pull and Push. Like the commit and Perforce runs, the sync is
//! admitted on the root's workspace-op queue, then hands the network call to
//! its own queue, so later reads in the root do not wait for it. Its reply
//! is an `Update::Workspace` for the same op.
use super::super::submit::Services;
use crate::{Update, workspace_ops};
use std::path::{Path, PathBuf};
use terminator_core::async_service::{CancellationToken, OperationContext, Policy};

/// The call takes as long as it takes, up to
/// [`workspace_ops::SYNC_TIMEOUT`]; Cancel stops it, and the run must still
/// reply.
fn run_context(root: &Path) -> OperationContext {
    let mut context = OperationContext::new(
        workspace_ops::SYNC_SUBSYSTEM,
        root.display().to_string(),
        Policy::OrderedMutation,
    );
    context.deadline = None;
    context
}

impl Services {
    pub(in crate::gui_services) fn sync_run(
        &self,
        root: PathBuf,
        op: workspace_ops::Op,
        updates: &mut Vec<Update>,
    ) {
        let service = self.clone();
        let (run_root, run_op) = (root.clone(), op.clone());
        let submitted =
            self.handle()
                .submit(run_context(&root), CancellationToken::new(), async move {
                    let result = match &run_op {
                        workspace_ops::Op::SyncRun(run) => {
                            workspace_ops::perform_sync(service.processes(), &run_root, run).await
                        }
                        _ => Err("Only Fetch, Pull and Push run here".into()),
                    };
                    Ok(vec![Update::Workspace(run_root, run_op, result)])
                });
        if let Err(error) = submitted {
            updates.push(Update::Workspace(root, op, Err(error.to_string())));
        }
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use crate::Job;
    use eframe::egui;
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };
    use terminator_core::{CommandOptions, Paths, git, run_command};
    use workspace_ops::{BranchOp, Op, Report, SyncSlots};

    fn git(cwd: &Path, args: &[&str]) {
        let options = CommandOptions {
            timeout: Duration::from_secs(20),
            ..Default::default()
        };
        run_command(git::command(cwd, args), options).unwrap();
    }

    /// A repository with one commit and a tracked local bare `origin`.
    fn repository(dir: &Path) -> PathBuf {
        let root = dir.join("repo");
        std::fs::create_dir_all(&root).unwrap();
        git(dir, &["init", "-q", "--bare", "-b", "main", "origin.git"]);
        git(&root, &["init", "-q", "-b", "main"]);
        git(&root, &["config", "user.email", "ops@example.com"]);
        git(&root, &["config", "user.name", "Ops"]);
        git(&root, &["config", "core.hooksPath", "/dev/null"]);
        std::fs::write(root.join("a.txt"), "base\n").unwrap();
        git(&root, &["add", "a.txt"]);
        git(&root, &["commit", "-qm", "base"]);
        let origin = dir.join("origin.git");
        git(
            &root,
            &["remote", "add", "origin", &origin.to_string_lossy()],
        );
        git(&root, &["push", "-q", "-u", "origin", "main"]);
        root
    }

    fn services(dir: &Path) -> (Services, crate::gui_services::Owner) {
        let (updates, _) = mpsc::channel();
        Services::new(
            Paths::at(dir.join("data")),
            egui::Context::default(),
            updates,
        )
        .unwrap()
    }

    fn reply(owner: &mut crate::gui_services::Owner) -> Result<Report, String> {
        let deadline = Instant::now().checked_add(Duration::from_secs(30)).unwrap();
        loop {
            if let Some(mut completion) = owner.supervisor.try_recv()
                && let Some(Ok(updates)) = completion.result.take()
            {
                for update in updates {
                    if let Update::Workspace(_, Op::SyncRun(_), result) = update {
                        return result;
                    }
                }
            }
            assert!(Instant::now() < deadline, "no reply");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn head(repository: &Path, name: &str) -> String {
        let output = run_command(
            git::command(repository, &["rev-parse", name]),
            CommandOptions::default(),
        )
        .unwrap();
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    #[test]
    fn push_runs_as_a_sync_job_and_moves_the_remote_branch() {
        let dir = tempfile::Builder::new()
            .prefix("sync-job-")
            .tempdir_in("/tmp")
            .unwrap();
        let root = repository(dir.path());
        std::fs::write(root.join("a.txt"), "next\n").unwrap();
        git(&root, &["commit", "-qam", "next"]);
        let (service, mut owner) = services(dir.path());
        let mut slots = SyncSlots::default();
        let run = slots.start(&root, BranchOp::Push).unwrap();
        service
            .send(Job::Workspace(root.clone(), Op::SyncRun(run)))
            .unwrap();
        let end = reply(&mut owner);
        assert!(
            matches!(&end, Ok(Report::Message(text)) if text == "Pushed"),
            "{end:?}"
        );
        assert_eq!(
            head(&dir.path().join("origin.git"), "main"),
            head(&root, "HEAD")
        );
    }

    #[test]
    fn cancel_stops_a_hanging_fetch_and_reports_it() {
        let dir = tempfile::Builder::new()
            .prefix("sync-job-")
            .tempdir_in("/tmp")
            .unwrap();
        let root = repository(dir.path());
        // The remote side never answers: `git fetch` waits on it.
        let pid = dir.path().join("pack.pid");
        let script = dir.path().join("upload-pack.sh");
        let body = format!("#!/bin/sh\necho $$ > '{}'\nexec sleep 30\n", pid.display());
        std::fs::write(&script, body).unwrap();
        let executable = std::os::unix::fs::PermissionsExt::from_mode(0o755);
        std::fs::set_permissions(&script, executable).unwrap();
        git(
            &root,
            &[
                "config",
                "remote.origin.uploadpack",
                &script.to_string_lossy(),
            ],
        );
        let (service, mut owner) = services(dir.path());
        let mut slots = SyncSlots::default();
        let run = slots.start(&root, BranchOp::Fetch).unwrap();
        assert!(slots.start(&root, BranchOp::Pull).is_none(), "one per root");
        service
            .send(Job::Workspace(root.clone(), Op::SyncRun(run)))
            .unwrap();
        let deadline = Instant::now().checked_add(Duration::from_secs(20)).unwrap();
        while !pid.exists() {
            assert!(Instant::now() < deadline, "fetch never reached the remote");
            std::thread::sleep(Duration::from_millis(5));
        }
        let started = Instant::now();
        slots.progress(&root).unwrap().cancel.cancel();
        assert_eq!(slots.progress(&root).unwrap().line(), "Stopping…");
        let end = reply(&mut owner);
        assert_eq!(end.unwrap_err(), "Fetch cancelled");
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "SIGINT, then SIGKILL"
        );
        slots.finish(&root);
        assert!(slots.start(&root, BranchOp::Pull).is_some());
    }
}
