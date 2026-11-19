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
        state: runner.state().clone(),
    };
    agent::report(&own, true, Some(note)).unwrap();
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
