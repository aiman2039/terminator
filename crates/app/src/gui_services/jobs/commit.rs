//! The commit job. It is admitted on the root's workspace-op queue, so every
//! earlier action there (Stage, Stage All, diff hunk and gutter writes) has
//! finished. It then hands the long run to its own queue: later reads and
//! Explorer actions in the root do not wait for the minutes a hook or a push
//! may take. While Git commits, the GUI refuses the actions that could
//! change the commit, and the branch switch until the push ends too
//! (`App::refuse_during_commit`). The push runs as a second job on that
//! queue, so its progress report arrives before its end.
use super::super::submit::Services;
use crate::{Update, workspace_ops};
use std::path::{Path, PathBuf};
use terminator_core::async_service::{CancellationToken, Failure, OperationContext, Policy};

/// Scheduling subsystem of the commit and push runs.
pub(crate) const COMMIT_SUBSYSTEM: &str = "git-commit";

/// Hooks and pushes take as long as they take; Cancel stops them, and the
/// queued run must still reply.
fn run_context(root: &Path) -> OperationContext {
    let mut context = OperationContext::new(
        COMMIT_SUBSYSTEM,
        root.display().to_string(),
        Policy::OrderedMutation,
    );
    context.deadline = None;
    context
}

impl Services {
    /// Queues the commit run. Its reply is an `Update::Workspace` for `op`.
    pub(in crate::gui_services) fn commit(
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
                    let result = service.commit_step(&run_root, &run_op).await;
                    Ok(vec![Update::Workspace(run_root, run_op, result)])
                });
        if let Err(error) = submitted {
            updates.push(Update::Workspace(root, op, Err(error.to_string())));
        }
    }

    /// Commits. With a push to follow, queues it and reports `Pushing`.
    async fn commit_step(
        &self,
        root: &Path,
        op: &workspace_ops::Op,
    ) -> Result<workspace_ops::Report, String> {
        let workspace_ops::Op::Commit {
            message,
            amend,
            signoff,
            push,
            cancel,
        } = op
        else {
            return Err("Only a commit runs as a commit job".into());
        };
        let git = workspace_ops::CommitRun {
            processes: self.processes(),
            root,
            cancel,
        };
        git.commit(workspace_ops::CommitRequest {
            message,
            amend: *amend,
            signoff: *signoff,
        })
        .await?;
        if !*push {
            return Ok(workspace_ops::Report::Message("Committed".into()));
        }
        match self.queue_push(root, op) {
            Ok(()) => Ok(workspace_ops::Report::Pushing),
            Err(error) => Err(workspace_ops::push_failed(&error)),
        }
    }

    fn queue_push(&self, root: &Path, op: &workspace_ops::Op) -> Result<(), Failure> {
        let service = self.clone();
        let (root, op) = (root.to_path_buf(), op.clone());
        self.handle()
            .submit(run_context(&root), CancellationToken::new(), async move {
                let result = match &op {
                    workspace_ops::Op::Commit { cancel, .. } => {
                        let git = workspace_ops::CommitRun {
                            processes: service.processes(),
                            root: &root,
                            cancel,
                        };
                        git.push().await
                    }
                    _ => Err("Only a commit pushes".into()),
                };
                Ok(vec![Update::Workspace(root, op, result)])
            })
            .map(|_| ())
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
    use workspace_ops::{Op, Report};

    fn git(cwd: &Path, args: &[&str]) {
        let options = CommandOptions {
            timeout: Duration::from_secs(20),
            ..Default::default()
        };
        run_command(git::command(cwd, args), options).unwrap();
    }

    /// A repository with a staged change, a tracked bare `origin`, and an
    /// empty hooks folder (local config wins over a global one).
    fn repository(dir: &Path) -> (PathBuf, PathBuf) {
        let (root, origin, hooks) = (dir.join("repo"), dir.join("origin.git"), dir.join("hooks"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&hooks).unwrap();
        git(dir, &["init", "-q", "--bare", "-b", "main", "origin.git"]);
        git(&root, &["init", "-q", "-b", "main"]);
        git(&root, &["config", "user.email", "ops@example.com"]);
        git(&root, &["config", "user.name", "Ops"]);
        git(
            &root,
            &["config", "core.hooksPath", &hooks.to_string_lossy()],
        );
        std::fs::write(root.join("a.txt"), "base\n").unwrap();
        git(&root, &["add", "a.txt"]);
        git(&root, &["commit", "-qm", "base"]);
        git(
            &root,
            &["remote", "add", "origin", &origin.to_string_lossy()],
        );
        git(&root, &["push", "-q", "-u", "origin", "main"]);
        std::fs::write(root.join("a.txt"), "next\n").unwrap();
        git(&root, &["add", "a.txt"]);
        (root, origin)
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

    fn commit(push: bool, cancel: &CancellationToken) -> Op {
        Op::Commit {
            message: "next".into(),
            amend: false,
            signoff: false,
            push,
            cancel: cancel.clone(),
        }
    }

    /// The next workspace reply whose op `wanted` accepts.
    fn reply(
        owner: &mut crate::gui_services::Owner,
        wanted: fn(&Op) -> bool,
    ) -> Result<Report, String> {
        let deadline = Instant::now().checked_add(Duration::from_secs(20)).unwrap();
        loop {
            if let Some(mut completion) = owner.supervisor.try_recv()
                && let Some(Ok(updates)) = completion.result.take()
            {
                for update in updates {
                    if let Update::Workspace(_, op, result) = update
                        && wanted(&op)
                    {
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
    fn push_reports_progress_before_its_end() {
        let dir = tempfile::Builder::new()
            .prefix("commit-job-")
            .tempdir_in("/tmp")
            .unwrap();
        let (root, origin) = repository(dir.path());
        let (service, mut owner) = services(dir.path());
        let job = Job::Workspace(root.clone(), commit(true, &CancellationToken::new()));
        service.send(job).unwrap();
        let is_commit = |op: &Op| matches!(op, Op::Commit { .. });
        assert!(matches!(reply(&mut owner, is_commit), Ok(Report::Pushing)));
        let end = reply(&mut owner, is_commit);
        assert!(
            matches!(end, Ok(Report::Message(ref text)) if text == "Committed and pushed"),
            "{end:?}"
        );
        assert_eq!(head(&origin, "main"), head(&root, "HEAD"));
    }

    #[test]
    fn a_running_commit_does_not_hold_later_actions_in_its_root() {
        let dir = tempfile::Builder::new()
            .prefix("commit-job-")
            .tempdir_in("/tmp")
            .unwrap();
        let (root, _origin) = repository(dir.path());
        let pid = dir.path().join("hook.pid");
        let hook = dir.path().join("hooks/pre-commit");
        let script = format!("#!/bin/sh\necho $$ > '{}'\nexec sleep 30\n", pid.display());
        std::fs::write(&hook, script).unwrap();
        let executable = std::os::unix::fs::PermissionsExt::from_mode(0o755);
        std::fs::set_permissions(&hook, executable).unwrap();
        let (service, mut owner) = services(dir.path());
        let cancel = CancellationToken::new();
        service
            .send(Job::Workspace(root.clone(), commit(false, &cancel)))
            .unwrap();
        let deadline = Instant::now().checked_add(Duration::from_secs(20)).unwrap();
        while !pid.exists() {
            assert!(Instant::now() < deadline, "hook never started");
            std::thread::sleep(Duration::from_millis(5));
        }
        // The hook still runs; a later Explorer action and a read in the
        // same root still finish. (The GUI refuses index changes meanwhile.)
        let create = Job::Workspace(root.clone(), Op::CreateFile(root.join("b.txt")));
        service.send(create).unwrap();
        let created = reply(&mut owner, |op| matches!(op, Op::CreateFile(_)));
        assert!(created.is_ok(), "{created:?}");
        assert!(root.join("b.txt").is_file());
        service
            .send(Job::Workspace(root.clone(), Op::Branches))
            .unwrap();
        let listed = reply(&mut owner, |op| matches!(op, Op::Branches));
        assert!(
            matches!(&listed, Ok(Report::Branches(names)) if names == &["main"]),
            "{listed:?}"
        );
        cancel.cancel();
        let end = reply(&mut owner, |op| matches!(op, Op::Commit { .. }));
        assert_eq!(end.unwrap_err(), workspace_ops::COMMIT_CANCELLED);
        assert_eq!(head(&root, "HEAD"), head(&root, "origin/main"));
    }

    #[test]
    fn cancel_after_the_commit_landed_reports_the_commit_and_skips_the_push() {
        let dir = tempfile::Builder::new()
            .prefix("commit-job-")
            .tempdir_in("/tmp")
            .unwrap();
        let (root, _origin) = repository(dir.path());
        let pid = dir.path().join("hook.pid");
        let hook = dir.path().join("hooks/post-commit");
        let script = format!("#!/bin/sh\necho $$ > '{}'\nexec sleep 30\n", pid.display());
        std::fs::write(&hook, script).unwrap();
        let executable = std::os::unix::fs::PermissionsExt::from_mode(0o755);
        std::fs::set_permissions(&hook, executable).unwrap();
        let before = head(&root, "HEAD");
        let (service, mut owner) = services(dir.path());
        let cancel = CancellationToken::new();
        service
            .send(Job::Workspace(root.clone(), commit(true, &cancel)))
            .unwrap();
        let deadline = Instant::now().checked_add(Duration::from_secs(20)).unwrap();
        while !pid.exists() {
            assert!(Instant::now() < deadline, "hook never started");
            std::thread::sleep(Duration::from_millis(5));
        }
        // HEAD moved before `post-commit` ran; Cancel stops only the hook.
        cancel.cancel();
        let end = reply(&mut owner, |op| matches!(op, Op::Commit { .. }));
        let notice = end.unwrap_err();
        assert!(workspace_ops::commit_landed(&notice), "{notice}");
        assert!(workspace_ops::commit_cancelled(&notice), "{notice}");
        assert_ne!(head(&root, "HEAD"), before, "the commit is there");
        assert_eq!(head(&root, "origin/main"), before, "no push after Cancel");
    }
}
