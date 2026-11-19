use super::gates::{GateChoice, branch_labels};
use super::tests::{app_with_project, painted, project_with_flows, written};
use super::*;
use terminator_core::flow::{self, StepStatus};

/// build, a review gate with ship and rework, a release step for ship,
/// and a fix step for rework that loops back to build (3 rounds).
pub(super) const REVIEW: &str = r#"
version = 1
name = "Review"

[[step]]
id = "build"
command = "claude"

[[step]]
id = "review"
gate = "Ship it?"
needs = ["build"]
options = ["ship", "rework"]

[[step]]
id = "release"
command = "echo release"
needs = ["review"]
when = "review.ship"

[[step]]
id = "fix"
command = "claude"
needs = ["review"]
when = "review.rework"
loop_to = "build"
max_rounds = 3
"#;

fn review_run(dir: PathBuf) -> Run {
    let flow = flow::parse(REVIEW).expect("flow");
    Run {
        dir,
        state: RunState::new(&flow, "review", "p"),
        gone: false,
    }
}

#[test]
fn gate_with_options_shows_one_button_per_option_and_no_plain_approve() {
    let project = project_with_flows();
    let (mut app, _received, data) = app_with_project(project.path());
    let mut run = review_run(data.path().join("flows").join("review-1"));
    run.state.set("build", StepStatus::Done);
    run.state.set("review", StepStatus::Running);
    app.flows.runs = vec![run];
    let ctx = egui::Context::default();
    let text = painted(&ctx, |ui| app.flows_panel(ui));
    for label in ["Ship it?", "ship", "rework", "Reject"] {
        assert!(text.iter().any(|line| line == label), "{label}: {text:?}");
    }
    assert!(!text.iter().any(|line| line == "Approve"), "{text:?}");
}

#[test]
fn option_button_writes_only_the_chosen_option() {
    let project = project_with_flows();
    let (mut app, _received, data) = app_with_project(project.path());
    let dir = data.path().join("flows").join("review-1");
    std::fs::create_dir_all(&dir).expect("dir");
    let mut run = review_run(dir.clone());
    run.state.set("build", StepStatus::Done);
    run.state.set("review", StepStatus::Running);
    std::fs::write(
        dir.join("state.json"),
        serde_json::to_vec(&run.state).expect("json"),
    )
    .expect("state");
    let _runner = flow::control::hold_runner(&dir).expect("lock");
    let choice = GateChoice::Option("ship".into());
    app.apply_flow_act(panel::Act::Gate(dir.clone(), "review".into(), choice));
    assert_eq!(written(&dir.join("answers/review")), "option:ship");
}

#[test]
fn step_rows_show_the_choice_the_skipped_branch_and_the_round() {
    let mut run = review_run("/d/review-1".into());
    run.state.set("build", StepStatus::Done);
    run.state.set("review", StepStatus::Done);
    run.state.steps.get_mut("review").expect("gate").choice = Some("rework".into());
    run.state.set("release", StepStatus::Skipped);
    for id in ["build", "review", "fix"] {
        run.state.steps.get_mut(id).expect("step").round = 2;
    }
    let review = &run.state.steps["review"];
    assert_eq!(branch_labels(review), ["chose rework", "round 2 of 3"]);
    let release = &run.state.steps["release"];
    assert_eq!(branch_labels(release), ["when review.ship"]);
    assert_eq!(status::step_style(release).label, "Skipped");
    assert_eq!(status::step_style(release).tone, status::Tone::Dim);
    // A skipped step is not a failure: the ended run is done.
    run.state.set("fix", StepStatus::Done);
    run.state.ended = true;
    assert_eq!(status::run_chip(&run), ("done", status::Tone::Done));
}

#[test]
fn panel_rows_show_skipped_and_round_words() {
    let project = project_with_flows();
    let (mut app, _received, data) = app_with_project(project.path());
    let mut run = review_run(data.path().join("flows").join("review-1"));
    run.state.set("build", StepStatus::Done);
    run.state.set("review", StepStatus::Done);
    run.state.steps.get_mut("review").expect("gate").choice = Some("ship".into());
    run.state.set("fix", StepStatus::Skipped);
    run.state.set("release", StepStatus::Running);
    app.flows.runs = vec![run];
    let ctx = egui::Context::default();
    let text = painted(&ctx, |ui| app.flows_panel(ui));
    for label in [
        "skipped",
        "when review.rework · round 1 of 3",
        "chose ship · round 1 of 3",
    ] {
        assert!(text.iter().any(|line| line == label), "{label}: {text:?}");
    }
}
