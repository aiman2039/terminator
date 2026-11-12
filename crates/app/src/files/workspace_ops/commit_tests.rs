use super::commit::{LONG_GIT_TIMEOUT, long_options, tail};
#[cfg(unix)]
use super::tests::git;
use super::tests::repo;
use super::*;
use std::time::Duration;

#[cfg(unix)]
fn request(message: &str) -> CommitRequest<'_> {
    CommitRequest {
        message,
        amend: false,
        signoff: false,
    }
}

fn subject(root: &Path) -> String {
    git_text(root, &["log", "-1", "--pretty=%s"]).unwrap()
}

#[test]
fn commit_and_push_get_the_long_cap_and_quick_calls_keep_20_seconds() {
    assert_eq!(git_options().timeout, Duration::from_secs(20));
    let long = long_options(None);
    assert_eq!(LONG_GIT_TIMEOUT, Duration::from_mins(30));
    assert_eq!(long.timeout, LONG_GIT_TIMEOUT);
    // A failing hook is read from its output, and verbose hooks fit.
    assert!(long.accepted_exit_codes.is_none());
    assert!(long.stderr_limit >= 8 * 1024 * 1024);
    // The quick path refuses commit, so commit never runs under 20 s.
    let dir = repo();
    let root = dir.path();
    fs::write(root.join("a.txt"), "next\n").unwrap();
    perform(root, Op::Stage(root.join("a.txt"))).unwrap();
    let op = Op::Commit {
        message: "m".into(),
        amend: false,
        signoff: false,
        push: false,
        cancel: CancellationToken::new(),
    };
    let error = perform(root, op).unwrap_err();
    assert!(error.to_string().contains("cancellable"), "{error}");
    assert_eq!(subject(root), "base");
}

#[test]
fn output_tail_is_plain_text_from_the_end() {
    assert_eq!(tail(""), "");
    assert_eq!(tail("\n \n\n"), "");
    // Colors and titles go; a redrawn progress line keeps its last state.
    let noisy = "\u{1b}[31merror\u{1b}[0m: bad\r\nWriting 10%\rWriting 100%\n\
                 \u{1b}]0;title\u{7}done\n\n";
    assert_eq!(tail(noisy), "error: bad\nWriting 100%\ndone");
    // Only the last lines stay; `…` marks the cut.
    let mut long = String::new();
    for n in 1..=50 {
        long.push_str(&format!("line {n}\n"));
    }
    let kept = tail(&long);
    assert!(kept.starts_with("…\nline 11\n"), "{kept}");
    assert!(kept.ends_with("line 50"));
    assert_eq!(kept.lines().count(), 41);
    // One huge line keeps its end.
    let huge = format!("{}end", "x".repeat(10_000));
    let kept = tail(&huge);
    assert!(kept.starts_with("…\n"));
    assert!(kept.ends_with("end"));
    assert_eq!(kept.chars().count(), 4002);
}

#[cfg(unix)]
mod hooks {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Instant;
    use terminator_core::async_process::Processes;

    /// A hook in its own folder. Local config wins over a global
    /// `core.hooksPath`.
    fn hook(root: &Path, name: &str, script: &str) {
        let hooks = root.join(".git/test-hooks");
        fs::create_dir_all(&hooks).unwrap();
        let path = hooks.join(name);
        fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        git(
            root,
            &["config", "core.hooksPath", &hooks.to_string_lossy()],
        );
    }

    /// A hook that records its PID, then waits far longer than the test.
    fn slow_hook(root: &Path, name: &str) -> PathBuf {
        let pid = root.join(".git/hook.pid");
        hook(
            root,
            name,
            &format!("echo $$ > '{}'\nexec sleep 30", pid.display()),
        );
        pid
    }

    fn change(root: &Path, text: &str) {
        fs::write(root.join("a.txt"), text).unwrap();
        perform(root, Op::Stage(root.join("a.txt"))).unwrap();
    }

    /// A bare `origin` with `main` pushed and tracked.
    fn remote(root: &Path) -> tempfile::TempDir {
        let remote = tempfile::tempdir().unwrap();
        git(remote.path(), &["init", "-q", "--bare", "-b", "main"]);
        git(
            root,
            &["remote", "add", "origin", &remote.path().to_string_lossy()],
        );
        git(root, &["push", "-q", "-u", "origin", "main"]);
        remote
    }

    struct Pool {
        processes: Processes,
        stop: CancellationToken,
        owner:
            tokio::task::JoinHandle<Result<(), terminator_core::async_process::AsyncProcessError>>,
    }

    fn pool() -> Pool {
        let stop = CancellationToken::new();
        let (processes, actor) = Processes::new(stop.clone());
        Pool {
            processes,
            stop,
            owner: tokio::spawn(actor),
        }
    }

    impl Pool {
        async fn close(self) {
            assert_eq!(self.processes.children(), 0, "every child was reaped");
            self.stop.cancel();
            self.owner.await.unwrap().unwrap();
        }
    }

    /// Waits for the hook to start, then presses Cancel. Returns its PID.
    async fn cancel_when_started(pid_file: &Path, cancel: &CancellationToken) -> i32 {
        let deadline = Instant::now().checked_add(Duration::from_secs(20)).unwrap();
        loop {
            if let Some(pid) = fs::read_to_string(pid_file)
                .ok()
                .and_then(|text| text.trim().parse().ok())
            {
                cancel.cancel();
                return pid;
            }
            assert!(Instant::now() < deadline, "hook never started");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Exited: gone, or a zombie that a container's init has not reaped
    /// (Linux shows state `Z`; macOS launchd reaps at once).
    fn exited(raw: i32) -> bool {
        let pid = rustix::process::Pid::from_raw(raw).unwrap();
        rustix::process::test_kill_process(pid).is_err()
            || fs::read_to_string(format!("/proc/{raw}/stat"))
                .ok()
                .and_then(|stat| {
                    let (_, state) = stat.rsplit_once(')')?;
                    Some(state.trim_start().starts_with('Z'))
                })
                .unwrap_or(false)
    }

    /// The hook died with Git's process group.
    async fn assert_gone(pid: i32) {
        let deadline = Instant::now().checked_add(Duration::from_secs(5)).unwrap();
        while !exited(pid) {
            assert!(Instant::now() < deadline, "hook still runs after Cancel");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn amend_signoff_and_push_run_as_long_jobs() {
        let dir = repo();
        let root = dir.path();
        let pool = pool();
        let cancel = CancellationToken::new();
        let run = CommitRun {
            processes: &pool.processes,
            root,
            cancel: &cancel,
        };
        let body = || git_text(root, &["log", "-1", "--pretty=%b"]).unwrap();
        // Amend with an empty message keeps the previous subject.
        change(root, "amended\n");
        let amend = CommitRequest {
            message: "",
            amend: true,
            signoff: true,
        };
        run.commit(amend).await.unwrap();
        assert_eq!(subject(root), "base");
        assert!(body().contains("Signed-off-by: Ops <ops@example.com>"));
        // Amend with a message replaces the subject.
        change(root, "again\n");
        let rename = CommitRequest {
            message: "renamed",
            amend: true,
            signoff: false,
        };
        run.commit(rename).await.unwrap();
        assert_eq!(subject(root), "renamed");
        // An empty message without amend never reaches Git.
        assert_eq!(
            run.commit(request(" ")).await,
            Err("Commit message is empty".into())
        );
        // Push goes to the tracked upstream.
        let remote = remote(root);
        change(root, "pushed\n");
        run.commit(request("pushed change")).await.unwrap();
        let report = run.push().await.unwrap();
        assert!(matches!(report, Report::Message(ref m) if m == "Committed and pushed"));
        assert_eq!(
            git_text(remote.path(), &["rev-parse", "main"]).unwrap(),
            git_text(root, &["rev-parse", "HEAD"]).unwrap()
        );
        pool.close().await;
    }

    #[tokio::test]
    async fn a_failing_hook_reports_the_end_of_its_output() {
        let dir = repo();
        let root = dir.path();
        hook(
            root,
            "pre-commit",
            "i=1; while [ $i -le 100 ]; do echo \"line $i\"; i=$((i+1)); done\n\
             echo 'rejected by hook' >&2\nexit 1",
        );
        change(root, "blocked\n");
        let pool = pool();
        let cancel = CancellationToken::new();
        let run = CommitRun {
            processes: &pool.processes,
            root,
            cancel: &cancel,
        };
        let error = run.commit(request("blocked")).await.unwrap_err();
        let mut lines = error.lines();
        assert_eq!(lines.next(), Some("Commit failed (exit status: 1)"));
        assert_eq!(lines.next(), Some("…"), "{error}");
        assert_eq!(lines.next(), Some("line 62"));
        assert!(error.ends_with("line 100\nrejected by hook"), "{error}");
        assert!(!commit_landed(&error) && !commit_cancelled(&error));
        assert_eq!(subject(root), "base");
        pool.close().await;
    }

    #[tokio::test]
    async fn cancel_kills_the_hook_and_commits_nothing() {
        let dir = repo();
        let root = dir.path();
        let pid = slow_hook(root, "pre-commit");
        change(root, "slow\n");
        let pool = pool();
        let cancel = CancellationToken::new();
        let run = CommitRun {
            processes: &pool.processes,
            root,
            cancel: &cancel,
        };
        let started = Instant::now();
        let (result, hook_pid) = tokio::join!(
            run.commit(request("slow")),
            cancel_when_started(&pid, &cancel)
        );
        assert_eq!(result, Err(COMMIT_CANCELLED.into()));
        assert!(started.elapsed() < Duration::from_secs(25));
        assert_gone(hook_pid).await;
        assert_eq!(subject(root), "base");
        pool.close().await;
    }

    #[tokio::test]
    async fn push_cancel_and_failure_say_the_commit_landed() {
        let dir = repo();
        let root = dir.path();
        let remote = remote(root);
        let pid = slow_hook(root, "pre-push");
        change(root, "landed\n");
        let pool = pool();
        let cancel = CancellationToken::new();
        let run = CommitRun {
            processes: &pool.processes,
            root,
            cancel: &cancel,
        };
        run.commit(request("landed")).await.unwrap();
        let (result, hook_pid) = tokio::join!(run.push(), cancel_when_started(&pid, &cancel));
        let error = result.unwrap_err();
        assert_eq!(error, PUSH_CANCELLED);
        assert!(commit_landed(&error) && commit_cancelled(&error));
        assert_gone(hook_pid).await;
        assert_eq!(subject(root), "landed");
        assert_ne!(
            git_text(remote.path(), &["rev-parse", "main"]).unwrap(),
            git_text(root, &["rev-parse", "HEAD"]).unwrap()
        );
        // A rejecting pre-push hook fails the push, not the commit.
        hook(root, "pre-push", "echo 'remote says no' >&2\nexit 1");
        let fresh = CancellationToken::new();
        let run = CommitRun {
            processes: &pool.processes,
            root,
            cancel: &fresh,
        };
        let error = run.push().await.unwrap_err();
        assert!(error.starts_with(PUSH_FAILED), "{error}");
        assert!(error.contains("\nremote says no"), "{error}");
        assert!(commit_landed(&error) && !commit_cancelled(&error));
        pool.close().await;
    }
}

#[test]
fn actions_are_classed_by_what_they_can_change_in_a_running_commit() {
    let path = || PathBuf::from("/repo/a.txt");
    let index = [
        Op::Stage(path()),
        Op::StageAll(vec![path()]),
        Op::Unstage(path()),
        Op::Discard {
            path: path(),
            untracked: true,
        },
    ];
    for op in index {
        assert_eq!(op.commit_risk(), CommitRisk::Index, "{op:?}");
    }
    // The push after the commit reads HEAD too.
    assert_eq!(Op::Switch("main".into()).commit_risk(), CommitRisk::Head);
    let branch = [
        BranchOp::Fetch,
        BranchOp::Pull,
        BranchOp::Push,
        BranchOp::New {
            name: "n".into(),
            start: None,
        },
        BranchOp::Rename {
            from: "a".into(),
            to: "b".into(),
        },
        BranchOp::Delete {
            name: "a".into(),
            force: false,
        },
        BranchOp::Merge("a".into()),
        BranchOp::Rebase("a".into()),
        BranchOp::Checkout("abc1234".into()),
        BranchOp::CherryPick("abc1234".into()),
        BranchOp::Revert("abc1234".into()),
    ];
    for op in branch {
        assert_eq!(
            Op::Branch(op.clone()).commit_risk(),
            CommitRisk::Head,
            "{op:?}"
        );
    }
    let stash = [
        Op::Shelve(vec![path()]),
        Op::Unshelve("stash@{0}".into()),
        Op::DropShelf("stash@{0}".into()),
    ];
    for op in stash {
        assert_eq!(op.commit_risk(), CommitRisk::Stash, "{op:?}");
    }
    let free = [
        Op::Branches,
        Op::Log,
        Op::Shelves,
        Op::HeadMessage,
        Op::Compare { base: None },
        Op::CompareTargets,
        Op::FileHistory(path()),
        Op::Delete(path()),
        Op::CreateFile(path()),
        Op::Rename {
            from: path(),
            to: "/repo/b.txt".into(),
        },
        Op::Reveal(path()),
    ];
    for op in free {
        assert_eq!(op.commit_risk(), CommitRisk::None, "{op:?}");
    }
}
