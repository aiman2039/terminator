use super::model::{Node, draft_of, free_id, graph_of, input, output, starter};
use super::*;
use crate::flows::tests::{app_with_project, project_with_flows};
use terminator_core::flow::{self, FLOW_DIR, StepKind};

const FLOW: &str = r#"
version = 1
name = "Plan and build"

[[step]]
id = "plan"
command = "claude"
prompt = "Write the plan."
worktree = "plan"

[[step]]
id = "approve"
gate = "Is the plan correct?"
needs = ["plan"]

[[step]]
id = "build"
command = "codex 'Do it.'"
cwd = "crates/app"
needs = ["approve"]
at = [900, 40]
"#;

/// Every text that one frame of `paint` draws in a window wide enough for
/// the editor's bar and three nodes side by side.
fn wide(ctx: &egui::Context, paint: impl FnMut(&mut egui::Ui)) -> Vec<String> {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600.0, 1000.0),
            )),
            ..Default::default()
        },
        paint,
    );
    output.textures_delta.clear();
    crate::app::tests::support::painted_strings(&output.shapes)
}

/// Waits for a worker thread: runs frames of the hidden poll until `done`.
fn wait(app: &mut App, done: impl Fn(&App) -> bool) {
    let ctx = egui::Context::default();
    let started = Instant::now();
    while !done(app) {
        assert!(started.elapsed() < Duration::from_secs(10), "worker thread");
        app.poll_flow_editor(&ctx);
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn graph_and_file_give_the_same_flow_both_ways() {
    let flow = flow::parse(FLOW).unwrap();
    let snarl = graph_of(&flow);
    let back = draft_of(&flow.name, &snarl).check().unwrap();
    assert_eq!(back.name, flow.name);
    for step in &flow.steps {
        let same = back.step(&step.id).unwrap();
        assert_eq!((&same.needs, &same.kind), (&step.needs, &step.kind));
    }
    // A saved place is kept; a step with none gets a column by its depth.
    assert_eq!(back.step("build").unwrap().at, Some([900, 40]));
    assert_eq!(back.step("plan").unwrap().at, Some([0, 0]));
    assert_eq!(back.step("approve").unwrap().at, Some([460, 0]));
}

#[test]
fn new_flow_starts_as_a_correct_flow_of_plan_gate_and_build() {
    let flow = draft_of("feature", &starter()).check().unwrap();
    let ids: Vec<_> = flow.steps.iter().map(|step| step.id.as_str()).collect();
    assert_eq!(ids, ["plan", "approve", "build"]);
    assert_eq!(flow.step("approve").unwrap().needs, ["plan"]);
    assert_eq!(flow.step("build").unwrap().needs, ["approve"]);
    assert!(matches!(
        flow.step("approve").unwrap().kind,
        StepKind::Gate { .. }
    ));
}

#[test]
fn wires_are_needs_and_empty_optional_fields_stay_out_of_the_file() {
    let mut snarl = Snarl::new();
    let a = snarl.insert_node(egui::Pos2::ZERO, Node::step(" one ", " claude ", "  "));
    let b = snarl.insert_node(egui::pos2(300.0, 0.0), Node::gate("ok", "Good?"));
    let c = snarl.insert_node(egui::pos2(600.0, 0.0), Node::step("two", "codex", "Go."));
    snarl.connect(output(a), input(b));
    snarl.connect(output(a), input(c));
    snarl.connect(output(b), input(c));
    let draft = draft_of("n", &snarl);
    assert_eq!(draft.steps[0].id, "one");
    assert_eq!(draft.steps[0].command.as_deref(), Some("claude"));
    assert_eq!(draft.steps[0].prompt, None);
    assert_eq!(draft.steps[0].cwd, None);
    assert_eq!(draft.steps[2].needs, ["ok", "one"]);
    let text = draft.to_toml().unwrap();
    assert!(!text.contains("cwd"), "{text}");
    assert!(!text.contains("worktree"), "{text}");
    // A removed wire is a removed need.
    snarl.disconnect(output(a), input(c));
    assert_eq!(draft_of("n", &snarl).steps[2].needs, ["ok"]);
}

#[test]
fn new_node_ids_do_not_repeat() {
    let mut snarl = starter();
    assert_eq!(free_id(&snarl, "step"), "step");
    snarl.insert_node(egui::Pos2::ZERO, Node::step("step", "claude", ""));
    assert_eq!(free_id(&snarl, "step"), "step-2");
    snarl.insert_node(egui::Pos2::ZERO, Node::step("step-2", "claude", ""));
    assert_eq!(free_id(&snarl, "step"), "step-3");
    assert_eq!(free_id(&snarl, "plan"), "plan-2");
}

#[test]
fn new_flow_writes_nothing_until_save_and_never_replaces_a_file() {
    let project = project_with_flows();
    let (mut app, received, _data) = app_with_project(project.path());
    app.new_flow_editor(project.path().into(), "bad name");
    assert!(app.flows.editor.is_none(), "a bad name opens nothing");
    app.new_flow_editor(project.path().into(), "feature");
    assert_eq!(app.center, CenterPage::FlowEditor);
    let file = project.path().join(FLOW_DIR).join("feature.toml");
    assert!(!file.exists(), "nothing is written before Save");
    assert!(app.flows.editor.as_ref().unwrap().dirty());

    app.save_flow_editor();
    wait(&mut app, |app| {
        app.flows.editor.as_ref().unwrap().saving.is_none()
    });
    let editor = app.flows.editor.as_ref().unwrap();
    assert_eq!(editor.error, None);
    assert!(!editor.dirty());
    assert_eq!(
        flow::files::load(project.path(), "feature")
            .unwrap()
            .steps
            .len(),
        3
    );

    // A new flow with the name of a flow that is there: Save refuses.
    app.close_flow_editor(false);
    assert!(app.flows.editor.is_none());
    let before = std::fs::read_to_string(project.path().join(FLOW_DIR).join("build.toml")).unwrap();
    app.new_flow_editor(project.path().into(), "build");
    app.save_flow_editor();
    wait(&mut app, |app| {
        app.flows.editor.as_ref().unwrap().saving.is_none()
    });
    assert!(app.flows.editor.as_ref().unwrap().error.is_some());
    assert_eq!(
        std::fs::read_to_string(project.path().join(FLOW_DIR).join("build.toml")).unwrap(),
        before
    );
    // The editor makes no terminal and types nothing.
    assert!(received.try_recv().is_err());
    assert!(app.post_create.by_tab.is_empty());
}

#[test]
fn save_of_a_wrong_graph_shows_the_error_and_writes_nothing() {
    let project = project_with_flows();
    let (mut app, _received, _data) = app_with_project(project.path());
    app.edit_flow_editor("p", project.path().into(), "build");
    wait(&mut app, |app| app.flows.editor.is_some());
    assert_eq!(app.center, CenterPage::FlowEditor);
    let path = project.path().join(FLOW_DIR).join("build.toml");
    let before = std::fs::read_to_string(&path).unwrap();
    let editor = app.flows.editor.as_mut().unwrap();
    assert!(!editor.dirty(), "an opened flow has no changes");
    // Two steps with one ID.
    editor
        .snarl
        .insert_node(egui::Pos2::ZERO, Node::step("one", "claude", ""));
    assert!(editor.dirty());
    app.save_flow_editor();
    let editor = app.flows.editor.as_ref().unwrap();
    assert!(editor.saving.is_none(), "a wrong flow starts no write");
    assert!(editor.error.as_deref().unwrap().contains("used twice"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
}

#[test]
fn close_with_changes_asks_first_and_a_hidden_page_keeps_the_flow() {
    let project = project_with_flows();
    let (mut app, _received, _data) = app_with_project(project.path());
    app.new_flow_editor(project.path().into(), "feature");
    app.close_flow_editor(false);
    assert!(app.flows.editor.as_ref().unwrap().confirm_discard);
    assert_eq!(app.center, CenterPage::FlowEditor);
    // Another page hides the editor; the flow stays.
    app.hide_center_overlay();
    assert!(app.flows.editor.is_some());
    // A second flow cannot open over the changes.
    app.new_flow_editor(project.path().into(), "other");
    assert_eq!(app.flows.editor.as_ref().unwrap().file(), "feature");
    assert_eq!(app.center, CenterPage::FlowEditor);
    app.close_flow_editor(true);
    assert!(app.flows.editor.is_none());
    assert_eq!(app.center, CenterPage::Workspace);
}

#[test]
fn wrong_flow_file_does_not_open_in_the_editor() {
    let project = project_with_flows();
    let (mut app, _received, _data) = app_with_project(project.path());
    app.edit_flow_editor("p", project.path().into(), "broken");
    wait(&mut app, |app| app.flows.opening.is_none());
    assert!(app.flows.editor.is_none());
    assert_ne!(app.center, CenterPage::FlowEditor);
}

#[test]
fn editor_page_shows_the_steps_the_gate_and_the_bar() {
    let project = project_with_flows();
    let (mut app, _received, _data) = app_with_project(project.path());
    app.new_flow_editor(project.path().into(), "feature");
    let ctx = egui::Context::default();
    for _ in 0..3 {
        wide(&ctx, |ui| app.flow_editor_center(ui));
    }
    let text = wide(&ctx, |ui| app.flow_editor_center(ui));
    for label in [
        "Flow feature.toml",
        "Add step",
        "Add gate",
        "Save",
        "Close",
        "Step",
        "Gate",
        "plan",
        "approve",
        "build",
        "claude",
        "Is the plan correct?",
        "after",
        "then",
    ] {
        assert!(text.iter().any(|line| line == label), "{label}: {text:?}");
    }
}

#[test]
fn new_flow_nodes_show_in_one_row_from_left_to_right() {
    let project = project_with_flows();
    let (mut app, _received, _data) = app_with_project(project.path());
    app.new_flow_editor(project.path().into(), "feature");
    let ctx = egui::Context::default();
    let frame = |app: &mut App| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1600.0, 1000.0),
                )),
                ..Default::default()
            },
            |ui| app.flow_editor_center(ui),
        );
        output.textures_delta.clear();
        crate::app::tests::support::painted_text(&output.shapes)
    };
    for _ in 0..4 {
        frame(&mut app);
    }
    let text = frame(&mut app);
    let at = |id: &str| {
        text.iter()
            .find(|(_, line)| line == id)
            .map(|(position, _)| *position)
            .unwrap_or_else(|| panic!("{id}: {text:?}"))
    };
    let (plan, approve, build) = (at("plan"), at("approve"), at("build"));
    assert!(
        (plan.y - approve.y).abs() < 1.0 && (approve.y - build.y).abs() < 1.0,
        "one row: {plan:?} {approve:?} {build:?}"
    );
    // The view has its own zoom, so check the order and equal steps. The
    // "Step" and "Gate" labels before the ID differ by a few points.
    let (first, second) = (approve.x - plan.x, build.x - approve.x);
    assert!(
        first > 100.0 && (first - second).abs() < 8.0,
        "equal column steps: {plan:?} {approve:?} {build:?}"
    );
    // The first view shows all nodes: the last one is not cut off.
    assert!(plan.x > 0.0 && build.x < 1300.0, "{plan:?} {build:?}");
}

#[test]
fn first_view_shows_all_nodes_in_a_small_canvas() {
    let canvas = egui::Rect::from_min_size(egui::pos2(300.0, 200.0), egui::vec2(600.0, 500.0));
    let view = viewer::fit_view(&starter(), canvas).unwrap();
    assert!(view.scaling < 1.0);
    for (position, _) in starter().nodes_pos() {
        let node = egui::Rect::from_min_size(position, egui::vec2(400.0, 280.0));
        assert!(canvas.contains_rect(view * node), "{node:?}");
    }
    assert!(viewer::fit_view(&Snarl::new(), canvas).is_none());
}
