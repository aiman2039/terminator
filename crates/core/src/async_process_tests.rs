//! The process pool: caps, cancellation, reaping, and the long user
//! commands' Ctrl-C and output tail.
use super::*;
use std::time::{Duration, Instant};
#[cfg(unix)]
fn shell(script: &str) -> Command {
    let mut command = Command::new("sh");
    command.args(["-c", script]);
    command
}
#[cfg(not(unix))]
fn shell(script: &str) -> Command {
    let mut command = Command::new("cmd");
    command.args(["/c", script]);
    command
}
#[cfg(unix)]
#[tokio::test]
async fn stalled_children_inherited_pipes_and_overflow_are_reaped() {
    let stop = CancellationToken::new();
    let (processes, actor) = Processes::new(stop.clone());
    let owner = tokio::spawn(actor);
    for script in ["sleep 10", "sleep 10 & exit 0", "yes x", "yes x >&2"] {
        let started = Instant::now();
        assert!(
            processes
                .run(
                    shell(script),
                    CommandOptions {
                        timeout: Duration::from_millis(80),
                        stdout_limit: 128,
                        stderr_limit: 128,
                        ..Default::default()
                    },
                    None
                )
                .await
                .is_err()
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(processes.children(), 0);
    }
    stop.cancel();
    owner.await.unwrap().unwrap();
}
#[cfg(unix)]
#[tokio::test]
async fn cancellation_reaps_child_and_other_repository_progresses() {
    let stop = CancellationToken::new();
    let (processes, actor) = Processes::new(stop.clone());
    let owner = tokio::spawn(actor);
    let other = processes.clone();
    let blocked = tokio::spawn(async move {
        other
            .run(
                shell("sleep 10"),
                CommandOptions::default(),
                Some("one".into()),
            )
            .await
    });
    let output = processes
        .run(
            shell("printf ready"),
            CommandOptions::default(),
            Some("two".into()),
        )
        .await
        .unwrap();
    assert_eq!(output.stdout, b"ready");
    blocked.abort();
    let _ = blocked.await;
    let deadline = Instant::now() + Duration::from_secs(2);
    while processes.children() != 0 {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    stop.cancel();
    owner.await.unwrap().unwrap();
}
#[cfg(unix)]
#[tokio::test]
async fn caller_cancel_returns_only_after_the_child_is_reaped() {
    let stop = CancellationToken::new();
    let (processes, actor) = Processes::new(stop.clone());
    let owner = tokio::spawn(actor);
    let cancel = CancellationToken::new();
    let run = {
        let processes = processes.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            let options = CommandOptions {
                timeout: Duration::from_mins(1),
                ..Default::default()
            };
            processes
                .run_cancellable(shell("sleep 30"), options, &cancel)
                .await
        })
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while processes.children() == 0 {
        assert!(Instant::now() < deadline, "child never started");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let started = Instant::now();
    cancel.cancel();
    let result = run.await.unwrap();
    assert!(matches!(
        result,
        Err(AsyncProcessError::Stopped(Failure::Cancelled))
    ));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(processes.children(), 0, "reaped before returning");
    // A request cancelled before it starts reports the cancel too.
    let early = processes
        .run_cancellable(shell("true"), CommandOptions::default(), &cancel)
        .await;
    assert!(matches!(
        early,
        Err(AsyncProcessError::Stopped(Failure::Cancelled))
    ));
    stop.cancel();
    owner.await.unwrap().unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn caller_cancel_interrupts_first_so_the_command_can_clean_up() {
    let stop = CancellationToken::new();
    let (processes, actor) = Processes::new(stop.clone());
    let owner = tokio::spawn(actor);
    let dir = tempfile::tempdir().unwrap();
    let (ready, restored) = (dir.path().join("ready"), dir.path().join("restored"));
    // As `pre-commit` puts stashed edits back on Ctrl-C. The loop forks
    // nothing, so the interrupt cannot land between a fork and an exec.
    let script = format!(
        "trap 'echo > \"{}\"; exit 130' INT; echo > \"{}\"; while :; do :; done",
        restored.display(),
        ready.display()
    );
    let cancel = CancellationToken::new();
    let run = {
        let processes = processes.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move {
            let options = CommandOptions {
                timeout: Duration::from_mins(1),
                ..Default::default()
            };
            processes
                .run_cancellable(shell(&script), options, &cancel)
                .await
        })
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready.exists() {
        assert!(Instant::now() < deadline, "script never started");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let started = Instant::now();
    cancel.cancel();
    let result = run.await.unwrap();
    assert!(matches!(
        result,
        Err(AsyncProcessError::Stopped(Failure::Cancelled))
    ));
    assert!(restored.exists(), "the interrupt handler ran");
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(processes.children(), 0);
    stop.cancel();
    owner.await.unwrap().unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn long_user_commands_keep_the_end_of_a_large_output() {
    let stop = CancellationToken::new();
    let (processes, actor) = Processes::new(stop.clone());
    let owner = tokio::spawn(actor);
    let options = CommandOptions {
        timeout: Duration::from_mins(1),
        stdout_limit: 1024,
        stderr_limit: 1024,
        ..Default::default()
    };
    let script = "i=0; while [ $i -lt 400 ]; do echo line$i; i=$((i+1)); done";
    let output = processes
        .run_cancellable(shell(script), options, &CancellationToken::new())
        .await
        .expect("a verbose command still succeeds");
    assert!(output.status.success());
    assert!(output.stdout.len() <= 1024);
    assert!(String::from_utf8_lossy(&output.stdout).ends_with("line399\n"));
    stop.cancel();
    owner.await.unwrap().unwrap();
}

#[cfg(not(unix))]
#[tokio::test]
async fn timeout_reaps_a_sleeping_child() {
    let stop = CancellationToken::new();
    let (processes, actor) = Processes::new(stop.clone());
    let owner = tokio::spawn(actor);
    let started = Instant::now();
    assert!(
        processes
            .run(
                shell("ping -n 10 127.0.0.1 >nul"),
                CommandOptions {
                    timeout: Duration::from_millis(200),
                    ..Default::default()
                },
                None
            )
            .await
            .is_err()
    );
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(processes.children(), 0);
    stop.cancel();
    owner.await.unwrap().unwrap();
}
