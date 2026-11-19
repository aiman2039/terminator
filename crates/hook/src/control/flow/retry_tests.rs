use super::tests::runner_without_daemon;
use super::*;
use std::time::{Duration, Instant};
use terminator_core::flow::{StepStatus, control::RETRIES};

/// Polls the run's state file until `check` passes.
fn wait_for(dir: &Path, what: &str, check: impl Fn(&flow::RunState) -> bool) {
    let deadline = Instant::now().checked_add(Duration::from_secs(20)).unwrap();
    loop {
        if control::read_state(dir).is_ok_and(|state| check(&state)) {
            return;
        }
        assert!(Instant::now() < deadline, "no {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn retry_request_resets_the_failed_step_once_and_drops_its_old_report() {
    let (_data, dir, mut runner, _interrupted) = runner_without_daemon();
    runner.running_command("build", "s1");
    let own = agent::Own {
        dir: dir.clone(),
        step: "build".into(),
        state: runner.state().clone(),
    };
    agent::report(&own, false, Some("broke")).unwrap();
    runner.poll_running();
    assert_eq!(runner.state().status("build"), StepStatus::Failed);
    // A request for another attempt, or for a step that did not fail, is
    // dropped.
    control::write(&dir, RETRIES, "build", b"5").unwrap();
    control::write(&dir, RETRIES, "ok", b"2").unwrap();
    runner.retries();
    assert_eq!(runner.state().status("build"), StepStatus::Failed);
    assert!(
        std::fs::read_dir(dir.join(RETRIES))
            .unwrap()
            .next()
            .is_none()
    );
    control::write(&dir, RETRIES, "build", b"2").unwrap();
    runner.retries();
    let build = &runner.state().steps["build"];
    assert_eq!((build.status, build.attempt()), (StepStatus::Waiting, 2));
    assert_eq!(build.session, None, "the old terminal is not used again");
    assert!(agent::read_report(&dir, "build").is_none());
    // The same request again does nothing.
    control::write(&dir, RETRIES, "build", b"2").unwrap();
    runner.retries();
    assert_eq!(runner.state().steps["build"].attempt(), 2);
}

#[test]
fn held_runner_waits_for_retry_and_starts_the_next_attempt() {
    let (_data, dir, runner, _interrupted) = runner_without_daemon();
    // Without a daemon the step cannot start, so each attempt fails.
    let running = std::thread::spawn(move || runner.run());
    wait_for(&dir, "a failed first attempt", |state| {
        state.status("build") == StepStatus::Failed
    });
    assert!(control::alive(&dir), "the runner waits for Retry or Cancel");
    assert!(control::read_run(&dir).unwrap().state.held());
    control::retry(&dir, "build").unwrap();
    wait_for(&dir, "a failed second attempt", |state| {
        state.steps["build"].attempt() == 2 && state.status("build") == StepStatus::Failed
    });
    assert_eq!(
        control::read_state(&dir).unwrap().status("ok"),
        StepStatus::Waiting
    );
    control::cancel(&dir).unwrap();
    assert!(matches!(running.join().unwrap().unwrap(), Outcome::Stopped));
    let state = control::read_state(&dir).unwrap();
    assert!(state.ended);
    assert_eq!(state.status("ok"), StepStatus::Cancelled);
    assert!(
        control::retry(&dir, "build").is_err(),
        "an ended run takes no retry"
    );
}

#[test]
fn end_on_failure_ends_the_run_at_the_first_stop() {
    let (_data, dir, runner, _interrupted) = runner_without_daemon();
    let runner = runner.with_inputs(std::collections::BTreeMap::new(), false);
    assert!(matches!(runner.run().unwrap(), Outcome::Stopped));
    let state = control::read_state(&dir).unwrap();
    assert!(state.ended);
    assert_eq!(
        (state.status("build"), state.status("ok")),
        (StepStatus::Failed, StepStatus::Waiting)
    );
}

#[test]
fn flow_run_takes_repeated_set_and_end_on_failure() {
    let parse = |args: &[&str]| {
        Cli::try_parse_from(std::iter::once("flow").chain(args.iter().copied()))
            .map(|cli| cli.command)
    };
    let Ok(Command::Run {
        set,
        end_on_failure,
        ..
    }) = parse(&["run", "p", "n", "--set", "a=1", "--set", "b=x y"])
    else {
        panic!("--set is refused");
    };
    assert_eq!(set, ["a=1", "b=x y"]);
    assert!(!end_on_failure);
    assert!(matches!(
        parse(&["retry", "n-1", "build"]),
        Ok(Command::Retry { .. })
    ));
}

#[test]
fn run_values_are_checked_before_anything_starts() {
    let project = tempfile::tempdir().unwrap();
    let flows = project.path().join(flow::FLOW_DIR);
    std::fs::create_dir_all(&flows).unwrap();
    std::fs::write(
        flows.join("fix.toml"),
        "version = 1\nname = \"Fix\"\n[inputs.issue]\n[[step]]\nid = \"s\"\ncommand = \"echo {{inputs.issue}}\"\n",
    )
    .unwrap();
    let project = Project {
        id: "p".into(),
        name: "p".into(),
        path: project.path().into(),
        layout: Value::Null,
    };
    let refused = |set: &[&str]| {
        let set: Vec<String> = set.iter().map(|text| (*text).to_owned()).collect();
        flow_with_inputs(&project, "fix", &set)
            .unwrap_err()
            .to_string()
    };
    assert!(refused(&[]).contains("required"));
    assert!(refused(&["nope=1"]).contains("no input"));
    assert!(refused(&["issue"]).contains("NAME=VALUE"));
    let set = vec!["issue=4; rm -rf x".to_owned()];
    let (flow, values) = flow_with_inputs(&project, "fix", &set).unwrap();
    assert_eq!(values["issue"], "4; rm -rf x");
    let flow::StepKind::Command { command, .. } = &flow.steps[0].kind else {
        panic!("not a command");
    };
    assert_eq!(command, "echo '4; rm -rf x'");
}
