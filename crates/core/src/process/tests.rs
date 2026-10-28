use super::*;
#[cfg(unix)]
fn shell(script: &str) -> Command {
    let mut c = Command::new("sh");
    c.args(["-c", script]);
    c
}
#[cfg(not(unix))]
fn shell(script: &str) -> Command {
    let mut c = Command::new("cmd");
    c.args(["/c", script]);
    c
}
#[cfg(unix)]
#[test]
fn timeout_includes_inherited_pipes() {
    let start = Instant::now();
    assert!(
        run_command(
            shell("sleep 10 & exit 0"),
            CommandOptions {
                timeout: Duration::from_millis(80),
                ..Default::default()
            }
        )
        .unwrap_err()
        .to_string()
        .contains("timed out")
    );
    assert!(start.elapsed() < Duration::from_secs(2));
}
/// A child that never reads stdin must not wedge the caller in a blocking
/// write: the timeout clock starts before stdin is touched.
#[cfg(windows)]
#[test]
fn blocked_stdin_still_times_out() {
    let start = Instant::now();
    let error = run_command(
        shell("ping -n 30 127.0.0.1 > nul"),
        CommandOptions {
            timeout: Duration::from_secs(3),
            input: Some(vec![b'x'; 4 * 1024 * 1024]),
            ..Default::default()
        },
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("timed out"), "{error}");
    assert!(start.elapsed() < Duration::from_secs(20));
}

#[cfg(unix)]
#[test]
fn overflow_is_explicit_for_both_streams() {
    for script in ["yes x", "yes x >&2"] {
        assert!(
            run_command(
                shell(script),
                CommandOptions {
                    stdout_limit: 100,
                    stderr_limit: 100,
                    ..Default::default()
                }
            )
            .unwrap_err()
            .to_string()
            .contains("exceeds")
        );
    }
}
#[cfg(unix)]
#[test]
fn session_leader_pid_equals_session_id() {
    let dir = tempfile::tempdir().unwrap();
    let ready = dir.path().join("ready");
    let staging = dir.path().join("ready.tmp");
    let mut command = Command::new("sh");
    command.arg("-c").arg(format!(
        "echo $$ > {staging} && mv {staging} {ready}; exec sleep 8",
        staging = staging.display(),
        ready = ready.display()
    ));
    let mut child = spawn_session_leader(command).unwrap();
    assert!(child.wait().unwrap().success());
    let start = Instant::now();
    let pid = loop {
        assert!(start.elapsed() < Duration::from_secs(3));
        if let Ok(contents) = std::fs::read_to_string(&ready)
            && let Ok(pid) = contents.trim().parse::<i32>()
        {
            break pid;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let tracked = rustix::process::Pid::from_raw(pid).unwrap();
    let sid = rustix::process::getsid(Some(tracked)).unwrap();
    assert_eq!(sid.as_raw_pid(), pid);
    crate::signals::signal_process(
        u32::try_from(pid).unwrap(),
        crate::signals::ProcSignal::Kill,
    )
    .unwrap();
}

#[cfg(unix)]
#[test]
fn stdin_and_accepted_exit_codes() {
    let o = run_command(
        shell("cat; exit 1"),
        CommandOptions {
            input: Some(b"hello".to_vec()),
            accepted_exit_codes: Some(vec![0, 1]),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(o.stdout, b"hello");
}

#[cfg(not(unix))]
#[test]
fn timeout_kills_a_sleeping_child() {
    let start = Instant::now();
    assert!(
        run_command(
            shell("ping -n 10 127.0.0.1 >nul"),
            CommandOptions {
                timeout: Duration::from_millis(300),
                ..Default::default()
            }
        )
        .unwrap_err()
        .to_string()
        .contains("timed out")
    );
    assert!(start.elapsed() < Duration::from_secs(10));
}

#[cfg(not(unix))]
#[test]
fn echo_returns_its_output() {
    let o = run_command(
        shell("echo hello"),
        CommandOptions {
            ..Default::default()
        },
    )
    .unwrap();
    assert!(String::from_utf8_lossy(&o.stdout).contains("hello"));
}
