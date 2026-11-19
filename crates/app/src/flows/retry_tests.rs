use super::tests::{app_with_project, painted, project_with_flows};
use super::*;
use terminator_core::flow::{self, StepRun, StepStatus};

const FLOW: &str = "version = 1\nname = \"Build\"\n[[step]]\nid = \"one\"\ncommand = \"claude\"\n[[step]]\nid = \"two\"\ncommand = \"claude\"\nneeds = [\"one\"]\n";

/// A run of [`FLOW`] whose step `one` failed in session `s1`.
fn failed_run(dir: PathBuf, gone: bool) -> Run {
    let mut state = RunState::new(&flow::parse(FLOW).expect("flow"), "build", "p");
    state.fail("one", "broke");
    state.steps.get_mut("one").expect("step").session = Some("s1".into());
    Run { dir, state, gone }
}

fn session(id: &str, lifecycle: Lifecycle) -> Session {
    Session {
        remote: None,
        review: false,
        id: id.into(),
        project_id: "p".into(),
        label: "agent".into(),
        cwd: "/p".into(),
        kind: SessionKind::Shell,
        file: None,
        lifecycle,
        created: 0,
        exit_code: None,
        rows: 24,
        cols: 80,
        generation: "g".into(),
        pid: None,
        truncated: false,
        cwd_confirmed: true,
    }
}

fn agent(session: &str, updated: u64) -> Agent {
    Agent {
        invocation_id: "i".into(),
        session_id: session.into(),
        kind: "claude".into(),
        provider_session_id: None,
        state: AgentState::Running,
        sequence: None,
        updated,
        resume: None,
        process: None,
    }
}

#[test]
fn retry_shows_only_for_a_failed_step_of_a_run_whose_runner_lives() {
    let live = failed_run("/d/build-1".into(), false);
    let one = &live.state.steps["one"];
    assert!(status::can_retry(&live, one));
    assert!(!status::can_retry(&live, &live.state.steps["two"]));
    assert_eq!(status::run_chip(&live), ("failed", status::Tone::Failed));
    let gone = failed_run("/d/build-2".into(), true);
    assert!(!status::can_retry(&gone, &gone.state.steps["one"]));
    let mut retried = one.clone();
    retried.attempt = 2;
    assert_eq!(
        status::attempt_label(&retried).as_deref(),
        Some("attempt 2")
    );
    assert_eq!(status::attempt_label(one), None);
}

#[test]
fn stall_badge_reads_only_session_and_hook_state() {
    let mut step = StepRun {
        status: StepStatus::Running,
        session: Some("s1".into()),
        started: Some(1_000),
        ..StepRun::default()
    };
    let mut snapshot = State::default();
    snapshot.sessions.push(session("s1", Lifecycle::Running));
    let now = 1_000 + 11 * 60;
    assert_eq!(
        status::stall(&step, &snapshot, now, 10).as_deref(),
        Some("no activity 11 min")
    );
    assert_eq!(status::stall(&step, &snapshot, now, 0), None, "0 is off");
    // A newer hook report is activity.
    snapshot.agents.push(agent("s1", now - 60));
    assert_eq!(status::stall(&step, &snapshot, now, 10), None);
    // A gate or a question waits for the user, not for the agent.
    snapshot.agents.clear();
    step.question = Some("A?".into());
    assert_eq!(status::stall(&step, &snapshot, now, 10), None);
    step.question = None;
    // An ended terminal with no report.
    snapshot.sessions[0].lifecycle = Lifecycle::Ended;
    assert_eq!(
        status::stall(&step, &snapshot, now, 10).as_deref(),
        Some("exited (no report)")
    );
    let failed = StepRun {
        status: StepStatus::Failed,
        session: Some("gone".into()),
        exited: true,
        ..StepRun::default()
    };
    assert_eq!(
        status::stall(&failed, &State::default(), now, 0).as_deref(),
        Some("exited (no report)")
    );
}

#[test]
fn tool_window_shows_retry_attempt_and_stall_and_retry_writes_one_request() {
    let project = project_with_flows();
    let (mut app, received, data) = app_with_project(project.path());
    let dir = data.path().join(flow::RUN_DIR).join("build-1");
    std::fs::create_dir_all(&dir).expect("dir");
    let mut run = failed_run(dir.clone(), false);
    run.state.steps.get_mut("one").expect("step").attempt = 2;
    run.state.steps.get_mut("one").expect("step").exited = true;
    std::fs::write(
        dir.join(flow::STATE_FILE),
        serde_json::to_vec(&run.state).expect("json"),
    )
    .expect("state");
    app.flows.runs = vec![run];
    app.flows.read_at = Some(Instant::now());
    let ctx = egui::Context::default();
    let text = painted(&ctx, |ui| app.flows_panel(ui));
    for label in ["failed · attempt 2", "exited (no report)", "failed"] {
        assert!(text.iter().any(|line| line == label), "{label}: {text:?}");
    }
    // Retry is an explicit click: nothing starts by painting.
    assert!(received.try_recv().is_err());
    assert!(!dir.join(flow::control::RETRIES).exists());
    let _runner = flow::control::hold_runner(&dir).expect("lock");
    app.apply_flow_act(panel::Act::Retry(dir.clone(), "one".into()));
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(5))
        .expect("deadline");
    let request = dir.join(flow::control::RETRIES).join("one");
    while !request.exists() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert_eq!(std::fs::read_to_string(request).expect("request"), "3");
    assert!(received.try_recv().is_err(), "the GUI opens no terminal");
}
