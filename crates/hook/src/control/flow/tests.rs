use super::*;
use terminator_core::flow::{RunState, StepStatus};

const FLOW: &str = "version = 1\nname = \"n\"\n[[step]]\nid = \"build\"\ncommand = \"x\"\n[[step]]\nid = \"ok\"\ngate = \"Good?\"\nneeds = [\"build\"]\n";

/// A run folder whose runner (this test) holds the lock.
fn run_folder(gate: StepStatus) -> (tempfile::TempDir, control::RunnerLock) {
    let dir = tempfile::tempdir().unwrap();
    let mut state = RunState::new(&flow::parse(FLOW).unwrap(), "n", "p");
    state.set("ok", gate);
    std::fs::write(
        dir.path().join(STATE_FILE),
        serde_json::to_vec(&state).unwrap(),
    )
    .unwrap();
    let runner = control::hold_runner(dir.path()).unwrap();
    (dir, runner)
}

#[test]
fn approve_is_written_only_for_a_gate_that_waits() {
    let (dir, _runner) = run_folder(StepStatus::Running);
    control::answer_gate(dir.path(), "ok", true).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join(ANSWERS).join("ok")).unwrap(),
        APPROVE
    );
    control::answer_gate(dir.path(), "ok", false).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join(ANSWERS).join("ok")).unwrap(),
        REJECT
    );
    // A command step, an unknown step, and a gate that does not wait.
    assert!(control::answer_gate(dir.path(), "build", true).is_err());
    assert!(control::answer_gate(dir.path(), "../x", true).is_err());
    let (waiting, _waiting_runner) = run_folder(StepStatus::Waiting);
    assert!(control::answer_gate(waiting.path(), "ok", true).is_err());
    assert!(!waiting.path().join(ANSWERS).exists());
}

#[test]
fn run_names_cannot_be_paths_and_a_run_folder_is_made_once() {
    let data = tempfile::tempdir().unwrap();
    for bad in ["", "../x", "a/b", "a.toml", "-x y"] {
        assert!(new_run_dir(data.path(), bad).is_err(), "{bad}");
    }
    assert!(!data.path().join(RUN_DIR).join("a").exists());
    let dir = new_run_dir(data.path(), "plan-and_build1").unwrap();
    assert_eq!(dir, data.path().join(RUN_DIR).join("plan-and_build1"));
    assert!(dir.is_dir());
    // A second run with the same name is refused, so two runners never
    // share a folder.
    let error = new_run_dir(data.path(), "plan-and_build1").unwrap_err();
    assert!(error.to_string().contains("exists"), "{error:#}");
}

#[test]
fn flow_run_takes_an_optional_run_name() {
    let parse = |args: &[&str]| {
        Cli::try_parse_from(std::iter::once("flow").chain(args.iter().copied()))
            .map(|cli| cli.command)
    };
    assert!(matches!(
        parse(&["run", "p", "n"]),
        Ok(Command::Run { run_id: None, .. })
    ));
    let Ok(Command::Run { run_id, .. }) = parse(&["run", "p", "n", "--run", "n-7"]) else {
        panic!("--run is refused");
    };
    assert_eq!(run_id.as_deref(), Some("n-7"));
}

#[test]
fn status_shows_a_run_whose_runner_is_gone_as_stopped() {
    let (_data, dir, runner) = data_with_run(false);
    let value = status(&dir).unwrap();
    assert_eq!(value["status"], "running");
    assert_eq!(value["steps"]["build"]["status"], "running");
    drop(runner);
    // A process that another test forks holds an inherited lock until its
    // exec: wait a moment for it.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut value = status(&dir).unwrap();
    while value["status"] == "running" && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
        value = status(&dir).unwrap();
    }
    assert_eq!(value["status"], "stopped (runner gone)");
    assert_eq!(value["steps"]["build"]["status"], "cancelled");
}

#[test]
fn flow_has_no_command_that_starts_a_flow_without_run() {
    let parse = |args: &[&str]| {
        Cli::try_parse_from(std::iter::once("flow").chain(args.iter().copied()))
            .map(|cli| cli.command)
    };
    assert!(matches!(
        parse(&["check", "p", "n"]),
        Ok(Command::Check { .. })
    ));
    assert!(matches!(parse(&["run", "p", "n"]), Ok(Command::Run { .. })));
    assert!(parse(&["run", "p", "n", "--trust"]).is_err());
    assert!(parse(&["start", "p", "n"]).is_err());
}

/// A data folder with one run in which step `build` runs in session `s1`,
/// and the lock of its runner (this test).
fn data_with_run(ended: bool) -> (tempfile::TempDir, PathBuf, control::RunnerLock) {
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join(RUN_DIR).join("n-1");
    std::fs::create_dir_all(&dir).unwrap();
    let mut state = RunState::new(&flow::parse(FLOW).unwrap(), "n", "p");
    state.set("build", StepStatus::Running);
    state.steps.get_mut("build").unwrap().session = Some("s1".into());
    state.ended = ended;
    std::fs::write(dir.join(STATE_FILE), serde_json::to_vec(&state).unwrap()).unwrap();
    let runner = control::hold_runner(&dir).unwrap();
    (data, dir, runner)
}

#[test]
fn agent_commands_find_only_the_running_step_of_their_terminal() {
    let (data, dir, runner) = data_with_run(false);
    let own = agent::own_step(data.path(), Some("s1".into())).unwrap();
    assert_eq!(
        (own.step.as_str(), own.dir.as_path()),
        ("build", dir.as_path())
    );
    assert!(agent::own_step(data.path(), Some("other".into())).is_err());
    let (ended, _, _ended_runner) = data_with_run(true);
    assert!(agent::own_step(ended.path(), Some("s1".into())).is_err());
    // A run whose runner is gone has no running step for its agent.
    drop(runner);
    assert!(agent::own_step(data.path(), Some("s1".into())).is_err());
}

#[test]
fn ask_ends_when_the_runner_is_gone() {
    let (data, dir, runner) = data_with_run(false);
    let own = agent::own_step(data.path(), Some("s1".into())).unwrap();
    drop(runner);
    let error = agent::ask(&own, "Anyone?", std::time::Duration::from_secs(20)).unwrap_err();
    assert!(error.to_string().contains("gone"), "{error:#}");
    assert!(control::question(&dir, "build").is_none());
}

#[test]
fn context_names_the_run_the_step_and_the_needed_steps() {
    let (data, dir, _runner) = data_with_run(false);
    let mut state = control::read_state(&dir).unwrap();
    state.set("build", StepStatus::Done);
    state.set("ok", StepStatus::Running);
    state.steps.get_mut("ok").unwrap().session = Some("s2".into());
    std::fs::write(dir.join(STATE_FILE), serde_json::to_vec(&state).unwrap()).unwrap();
    let own = agent::own_step(data.path(), Some("s2".into())).unwrap();
    let context = agent::context(&own);
    assert_eq!(context["run"], "n-1");
    assert_eq!(context["step"], "ok");
    assert_eq!(context["needs"][0]["step"], "build");
    assert_eq!(context["needs"][0]["status"], "done");
    assert_eq!(context["needs"][0]["session"], "s1");
}

#[test]
fn report_is_recorded_for_the_runner_and_starts_nothing() {
    let (data, dir, _runner) = data_with_run(false);
    let own = agent::own_step(data.path(), Some("s1".into())).unwrap();
    assert!(agent::read_report(&dir, "build").is_none());
    agent::report(&own, false, Some(" tests fail ")).unwrap();
    let report = agent::read_report(&dir, "build").unwrap();
    assert!(!report.done);
    assert_eq!(report.note.as_deref(), Some("tests fail"));
    agent::report(&own, true, None).unwrap();
    assert!(agent::read_report(&dir, "build").unwrap().done);
    // The state file is the runner's: a report does not change it.
    assert_eq!(
        control::read_state(&dir).unwrap().status("build"),
        StepStatus::Running
    );
}

#[test]
fn ask_waits_for_the_users_answer_and_returns_it() {
    let (data, dir, _runner) = data_with_run(false);
    // No open question: an answer is refused.
    assert!(control::answer_question(&dir, "build", "yes").is_err());
    let own = agent::own_step(data.path(), Some("s1".into())).unwrap();
    let asking = std::thread::spawn(move || {
        agent::ask(&own, "Use plan A?", std::time::Duration::from_secs(20))
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while control::question(&dir, "build").is_none() {
        assert!(std::time::Instant::now() < deadline, "no question");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        control::question(&dir, "build").as_deref(),
        Some("Use plan A?")
    );
    assert!(control::answer_question(&dir, "../x", "yes").is_err());
    control::answer_question(&dir, "build", "Yes, plan A").unwrap();
    let reply = asking.join().unwrap().unwrap();
    assert_eq!(reply["answer"], "Yes, plan A");
    assert!(control::question(&dir, "build").is_none());
}

#[test]
fn ask_without_an_answer_ends_with_an_error_and_removes_the_question() {
    let (data, dir, _runner) = data_with_run(false);
    let own = agent::own_step(data.path(), Some("s1".into())).unwrap();
    assert!(agent::ask(&own, "Anyone?", std::time::Duration::ZERO).is_err());
    assert!(control::question(&dir, "build").is_none());
    assert!(agent::ask(&own, "  ", std::time::Duration::ZERO).is_err());
}

#[test]
fn mcp_has_no_flow_tool_for_the_users_side() {
    let args = |words: &[&str]| words.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
    for words in [
        &["run", "p", "n"][..],
        &["approve", "n-1", "ok"],
        &["reject", "n-1", "ok"],
        &["cancel", "n-1"],
        &["answer", "n-1", "build", "yes"],
        &["skill"],
    ] {
        let error = execute_args(&args(words)).err().unwrap();
        assert!(error.to_string().contains("not available"), "{error:#}");
    }
    for tool in ["flow_run", "flow_approve", "flow_answer", "flow_cancel"] {
        assert!(crate::mcp::tool_argv(tool, &serde_json::Map::new()).is_none());
    }
    let argv = crate::mcp::tool_argv(
        "flow_report",
        serde_json::json!({"result": "done", "note": "--session=x"})
            .as_object()
            .unwrap(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(argv, ["flow", "report", "--note=--session=x", "--", "done"]);
}
