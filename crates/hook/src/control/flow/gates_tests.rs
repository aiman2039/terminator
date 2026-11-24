use super::tests::{run_folder_of, runner_of};
use super::*;
use terminator_core::flow::StepStatus;

/// build, a review gate with ship and rework, a release step for ship, a
/// fix step for rework that loops back to build (2 rounds), and a report
/// step that needs only the release step.
const REVIEW: &str = r#"
version = 1
name = "Review"

[[step]]
id = "build"
command = "x"

[[step]]
id = "review"
gate = "Ship it?"
needs = ["build"]
options = ["ship", "rework"]

[[step]]
id = "release"
command = "x"
needs = ["review"]
when = "review.ship"

[[step]]
id = "fix"
command = "x"
needs = ["review"]
when = "review.rework"
loop_to = "build"
max_rounds = 2

[[step]]
id = "report"
command = "x"
needs = ["release"]
"#;

/// Ends command step `id` as done through its own report.
fn report_done(runner: &mut runner::Runner, dir: &Path, id: &str, note: &str) {
    runner.running_command(id, &format!("s-{id}"));
    let own = agent::Own {
        dir: dir.to_path_buf(),
        step: id.into(),
        racer: None,
        state: runner.state().clone(),
    };
    agent::report(&own, true, Some(note), &[]).unwrap();
    runner.poll_running();
}

/// The gate answer file for `answer`, as `flow approve` writes it.
fn answer(runner: &mut runner::Runner, dir: &Path, answer: GateAnswer<'_>) {
    runner.running_gate("review");
    control::write(dir, ANSWERS, "review", answer.text().as_bytes()).unwrap();
    runner.poll_running();
}

#[test]
fn gate_with_options_takes_only_one_of_its_options() {
    let (dir, _lock) = run_folder_of(REVIEW, "review", StepStatus::Running);
    let error = control::answer_gate(dir.path(), "review", GateAnswer::Approve).unwrap_err();
    assert!(error.to_string().contains("ship, rework"), "{error}");
    let error =
        control::answer_gate(dir.path(), "review", GateAnswer::Choose("maybe")).unwrap_err();
    assert!(error.to_string().contains("maybe"), "{error}");
    assert!(!dir.path().join(ANSWERS).join("review").exists());
    control::answer_gate(dir.path(), "review", GateAnswer::Choose("ship")).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join(ANSWERS).join("review")).unwrap(),
        "option:ship"
    );
}

#[test]
fn approve_takes_an_option_on_the_command_line() {
    let parse = |args: &[&str]| {
        Cli::try_parse_from(std::iter::once("flow").chain(args.iter().copied()))
            .map(|cli| cli.command)
    };
    let Ok(Command::Approve { option, .. }) =
        parse(&["approve", "r", "review", "--option", "ship"])
    else {
        panic!("approve with --option");
    };
    assert_eq!(option.as_deref(), Some("ship"));
}

#[test]
fn chosen_option_runs_its_branch_and_skips_the_other() {
    let (_data, dir, mut runner, _interrupted) = runner_of(REVIEW);
    report_done(&mut runner, &dir, "build", "built");
    answer(&mut runner, &dir, GateAnswer::Choose("ship"));
    let review = &runner.state().steps["review"];
    assert_eq!(
        (review.status, review.choice.as_deref()),
        (StepStatus::Done, Some("ship"))
    );
    runner.skips();
    assert_eq!(runner.state().status("fix"), StepStatus::Skipped);
    assert_eq!(runner.state().status("release"), StepStatus::Waiting);
}

#[test]
fn rework_skips_the_release_and_the_steps_that_need_only_it() {
    let (_data, dir, mut runner, _interrupted) = runner_of(REVIEW);
    report_done(&mut runner, &dir, "build", "built");
    answer(&mut runner, &dir, GateAnswer::Choose("rework"));
    runner.skips();
    assert_eq!(runner.state().status("release"), StepStatus::Skipped);
    assert_eq!(runner.state().status("report"), StepStatus::Skipped);
    assert_eq!(runner.state().status("fix"), StepStatus::Waiting);
}

#[test]
fn approve_without_an_option_does_not_end_a_gate_with_options() {
    let (_data, dir, mut runner, _interrupted) = runner_of(REVIEW);
    report_done(&mut runner, &dir, "build", "built");
    answer(&mut runner, &dir, GateAnswer::Approve);
    assert_eq!(runner.state().status("review"), StepStatus::Running);
    answer(&mut runner, &dir, GateAnswer::Reject);
    assert_eq!(runner.state().status("review"), StepStatus::Failed);
}

#[test]
fn loop_runs_a_second_round_and_then_goes_on() {
    let (_data, dir, mut runner, _interrupted) = runner_of(REVIEW);
    report_done(&mut runner, &dir, "build", "built");
    answer(&mut runner, &dir, GateAnswer::Choose("rework"));
    runner.skips();
    report_done(&mut runner, &dir, "fix", "the tests fail");
    // Round 2: the loop's steps wait again, with no old answer or report.
    for id in ["build", "review", "fix"] {
        let run = &runner.state().steps[id];
        assert_eq!((run.status, run.round()), (StepStatus::Waiting, 2), "{id}");
        assert_eq!(run.session, None, "{id}: a new terminal");
    }
    assert_eq!(runner.state().status("release"), StepStatus::Waiting);
    assert!(!dir.join(ANSWERS).join("review").exists());
    assert!(agent::read_report(&dir, "fix").is_none());
    assert_eq!(runner.loop_note("build"), Some(("fix", "the tests fail")));
    // The last round: fix stays done and the run goes on.
    report_done(&mut runner, &dir, "build", "built again");
    answer(&mut runner, &dir, GateAnswer::Choose("rework"));
    runner.skips();
    report_done(&mut runner, &dir, "fix", "still failing");
    let fix = &runner.state().steps["fix"];
    assert_eq!((fix.status, fix.round()), (StepStatus::Done, 2));
    assert_eq!(fix.round_label().as_deref(), Some("round 2 of 2"));
    let flow = flow::parse(REVIEW).unwrap();
    assert_eq!(flow.outcome(runner.state()), Outcome::Done);
}

#[test]
fn gate_option_with_no_downstream_step_ends_run_as_done() {
    let text = r#"
version = 1
name = "Gate only"
[[step]]
id = "review"
gate = "Ship or rework?"
options = ["ship", "rework"]
[[step]]
id = "fix"
command = "echo fix"
needs = ["review"]
when = "review.rework"
loop_to = "review"
max_rounds = 2
"#;
    let (_data, dir, runner, _interrupted) = runner_of(text);
    let answer_dir = dir.clone();
    let responder = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            if let Ok(state) = control::read_state(&answer_dir)
                && state.status("review") == StepStatus::Running
            {
                control::answer_gate(&answer_dir, "review", GateAnswer::Choose("ship")).unwrap();
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("gate never reached Running");
    });
    let outcome = runner.run().unwrap();
    responder.join().unwrap();
    assert_eq!(outcome, Outcome::Done);
    let state = control::read_state(&dir).unwrap();
    assert_eq!(state.status("review"), StepStatus::Done);
    assert_eq!(state.status("fix"), StepStatus::Skipped);
}

#[test]
fn context_includes_rules_round_attempt_loop_note_and_gate_choice() {
    let text = format!("rules = \"Keep edits minimal.\"\n{REVIEW}");
    let (data, dir, mut runner, _interrupted) = runner_of(&text);
    let _lock = control::hold_runner(&dir).unwrap();
    report_done(&mut runner, &dir, "build", "built");
    answer(&mut runner, &dir, GateAnswer::Choose("rework"));
    runner.skips();
    runner.running_command("fix", "s-fix");
    let mut state = runner.state().clone();
    state.steps.get_mut("fix").unwrap().round = 2;
    state.steps.get_mut("fix").unwrap().attempt = 2;
    state.steps.get_mut("fix").unwrap().loop_note = Some("previous note".into());
    std::fs::write(dir.join(STATE_FILE), serde_json::to_vec(&state).unwrap()).unwrap();
    let own = agent::own_step(data.path(), Some("s-fix".into())).unwrap();
    let ctx = agent::context(&own);
    assert_eq!(ctx["rules"], "Keep edits minimal.");
    assert_eq!(ctx["round"], 2);
    assert_eq!(ctx["max_rounds"], 2);
    assert_eq!(ctx["attempt"], 2);
    assert_eq!(ctx["loop_note"], "previous note");
    assert_eq!(ctx["needs"][0]["choice"], "rework");
}

#[test]
fn auto_run_dir_adds_suffix_on_same_second_collision() {
    let data = tempfile::tempdir().unwrap();
    let (first, d1) = new_auto_run_dir(data.path(), "build", 1_700_000_000).unwrap();
    let (second, d2) = new_auto_run_dir(data.path(), "build", 1_700_000_000).unwrap();
    assert_eq!(first, "build-1700000000");
    assert_eq!(second, "build-1700000000-1");
    assert!(d1.is_dir() && d2.is_dir() && d1 != d2);
}

#[test]
fn pick_gate_validates_bounds_and_propagates_picked_worktree() {
    let text = r#"
version = 1
name = "Race"
[[step]]
id = "solve"
command = "claude"
count = 2
worktree = "try"
[[step]]
id = "choose"
gate = "Pick winner"
needs = ["solve"]
pick = "solve"
[[step]]
id = "verify"
command = "echo ok"
needs = ["choose"]
"#;
    let (_data, dir, mut runner, _interrupted) = runner_of(text);
    let _lock = control::hold_runner(&dir).unwrap();
    let wt1 = dir.join("wt-1");
    let wt2 = dir.join("wt-2");
    runner.state.steps.get_mut("solve").unwrap().racers = vec![
        flow::RacerRun {
            index: 1,
            worktree: "try-1".into(),
            worktree_path: Some(wt1),
            status: StepStatus::Done,
            ..Default::default()
        },
        flow::RacerRun {
            index: 2,
            worktree: "try-2".into(),
            worktree_path: Some(wt2.clone()),
            status: StepStatus::Done,
            ..Default::default()
        },
    ];
    runner.state.set("solve", StepStatus::Done);
    runner.state.set("choose", StepStatus::Running);
    runner.save().unwrap();
    // Plain Approve or out-of-range --pick is refused.
    assert!(control::answer_gate(&dir, "choose", GateAnswer::Approve).is_err());
    assert!(control::answer_gate(&dir, "choose", GateAnswer::Pick(3)).is_err());
    control::answer_gate(&dir, "choose", GateAnswer::Pick(2)).unwrap();
    let end = runner.gate_answer("choose");
    assert!(matches!(end, Some(Ok(()))));
    assert_eq!(runner.state.steps["choose"].picked, Some(2));
    let verify = runner.flow.step("verify").unwrap();
    let picked = runner.picked_worktree_for(verify).unwrap();
    assert_eq!(picked.path, wt2);
}
