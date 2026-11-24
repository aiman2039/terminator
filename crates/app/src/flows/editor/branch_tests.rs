use super::inspector::{Field, error_place};
use super::model::{Node, draft_of, graph_of, plan_text};
use super::tests::{count, flow_path, settle};
use super::*;
use crate::flows::gates_tests::REVIEW;
use crate::flows::tests::{app_with_project, project_with_flows};
use terminator_core::flow::{self, FLOW_DIR, FlowError, RunState, StepStatus};

#[test]
fn graph_keeps_options_conditions_and_loops_through_save() {
    let flow = flow::parse(REVIEW).unwrap();
    let snarl = graph_of(&flow);
    let gate = snarl.nodes().find(|node| node.id() == "review").unwrap();
    assert_eq!(gate.options(), ["ship", "rework"]);
    let fix = snarl.nodes().find(|node| node.id() == "fix").unwrap();
    assert_eq!(fix.branch().loop_to, "build");
    let mut back = draft_of("Review", &snarl).check().unwrap();
    for step in &mut back.steps {
        step.at = None;
    }
    let mut back_steps = back.steps.clone();
    back_steps.sort_by(|a, b| a.id.cmp(&b.id));
    let mut steps = flow.steps.clone();
    steps.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(back_steps, steps);
    // A step without a loop writes no max_rounds.
    let text = draft_of("Review", &snarl).to_toml().unwrap();
    assert_eq!(text.matches("max_rounds").count(), 1, "{text}");
}

#[test]
fn branch_errors_point_to_their_inspector_field() {
    let at = |error: FlowError| error_place(&error).1;
    assert_eq!(at(FlowError::Options("g".into())), Field::Options);
    let problem = String::new();
    let when = FlowError::When {
        step: "s".into(),
        problem: problem.clone(),
    };
    assert_eq!(at(when), Field::When);
    let back = FlowError::Loop {
        step: "s".into(),
        problem,
    };
    assert_eq!(at(back), Field::Loop);
    // A typed option list with a bad name is refused on Save.
    let mut snarl = graph_of(&flow::parse(REVIEW).unwrap());
    if let Some(Node::Gate { options, .. }) = snarl.nodes_mut().find(|node| node.is_gate()) {
        *options = "ship".into();
    }
    let error = draft_of("Review", &snarl).check().unwrap_err();
    assert_eq!(error, FlowError::Options("review".into()));
}

#[test]
fn plan_lists_options_conditions_and_loops() {
    let draft = Draft::from(&flow::parse(REVIEW).unwrap());
    let plan = plan_text(&draft, "review");
    for part in [
        "- Options: ship, rework",
        "- Runs when: review.ship",
        "- Loop: back to build when done, 3 rounds at most",
        "n1 -->|ship| n2",
        "n3 -.->|loop, 3 rounds| n0",
    ] {
        assert!(plan.contains(part), "{part}: {plan}");
    }
}

#[test]
fn cards_show_options_conditions_loops_and_option_buttons_in_a_run() {
    let project = project_with_flows();
    let flows = project.path().join(FLOW_DIR);
    std::fs::write(flows.join("review.toml"), REVIEW).unwrap();
    let (mut app, _received, _data) = app_with_project(project.path());
    let ctx = egui::Context::default();
    let path = flow_path(project.path(), "review");
    let text = settle(&mut app, &ctx, &path);
    for label in [
        "ship",
        "rework",
        "when review.ship",
        "when review.rework",
        "loop to build · 3 rounds",
        "loop, 3 rounds",
    ] {
        assert_eq!(count(&text, label), 1, "{label}: {text:?}");
    }
    let flow = flow::files::load(project.path(), "review").unwrap();
    let mut state = RunState::new(&flow, "review", "p");
    state.set("build", StepStatus::Done);
    state.set("review", StepStatus::Running);
    app.flows.set_runs(vec![crate::flows::Run {
        dir: project.path().join("review-1"),
        state,
        gone: false,
    }]);
    let text = settle(&mut app, &ctx, &path);
    // The option chip, the card's button and the strip's button.
    for label in ["ship", "rework"] {
        assert_eq!(count(&text, label), 3, "{label}: {text:?}");
    }
    assert_eq!(count(&text, "Approve"), 0, "{text:?}");
    assert_eq!(count(&text, "Reject"), 2, "{text:?}");
}

#[test]
fn run_strip_counts_skipped_steps_as_done_and_duplicate_clears_branch_and_worktree() {
    let project = project_with_flows();
    let flows = project.path().join(FLOW_DIR);
    std::fs::write(flows.join("review.toml"), REVIEW).unwrap();
    let (mut app, _received, _data) = app_with_project(project.path());
    let ctx = egui::Context::default();
    let path = flow_path(project.path(), "review");
    settle(&mut app, &ctx, &path);

    let flow = flow::files::load(project.path(), "review").unwrap();
    let mut state = RunState::new(&flow, "review", "p");
    state.set("build", StepStatus::Done);
    state.set("review", StepStatus::Done);
    state.set("release", StepStatus::Done);
    state.set("fix", StepStatus::Skipped);
    state.ended = true;
    app.flows.set_runs(vec![crate::flows::Run {
        dir: project.path().join("review-1"),
        state,
        gone: false,
    }]);
    let text = settle(&mut app, &ctx, &path);
    assert!(count(&text, "4 of 4 done") >= 1, "{text:?}");

    // Duplicate clears worktree, when, and loop_to so it stays valid.
    let Slot::Ready(editor) = app.flows.editors.get_mut(&path).unwrap() else {
        panic!("editor not ready");
    };
    let fix_node = editor
        .snarl
        .node_ids()
        .find(|(_, n)| n.id() == "fix")
        .map(|(id, _)| id)
        .unwrap();
    if let super::model::Node::Step { worktree, .. } = editor.snarl.get_node_mut(fix_node).unwrap()
    {
        *worktree = "wt-fix".into();
    }
    let dup = super::canvas::duplicate(&mut editor.snarl, fix_node).unwrap();
    let copied = editor.snarl.get_node(dup).unwrap();
    assert!(copied.branch().when.is_empty());
    assert!(copied.branch().loop_to.is_empty());
    if let super::model::Node::Step { worktree, .. } = copied {
        assert!(worktree.is_empty());
    }
    // Undo restores the pre-duplicate state cleanly.
    editor.sync_history();
    assert!(editor.undo());
    assert!(editor.snarl.get_node(dup).is_none());
    assert!(editor.redo());
}
