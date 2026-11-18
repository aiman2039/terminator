use super::model::{Node, draft_of, free_id, graph_of, input, output, plan_text, starter};
use super::*;
use crate::flows::tests::{app_with_project, project_with_flows};
use terminator_core::flow::{self, FLOW_DIR, StepKind};

const FLOW: &str = r#"
version = 1
name = "Plan and build"
rules = "Obey AGENTS.md."

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

type Painted = Vec<(egui::Pos2, String)>;

/// One frame of the editor tab of `path` in a wide pane, with pointer
/// `events`; gives the painted text with its place.
fn frame(app: &mut App, ctx: &egui::Context, path: &Path, events: Vec<egui::Event>) -> Painted {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600.0, 1000.0),
            )),
            events,
            ..Default::default()
        },
        |ui| app.flow_editor_view(ui, path),
    );
    output.textures_delta.clear();
    crate::app::tests::support::painted_text(&output.shapes)
}

fn flow_path(root: &Path, name: &str) -> PathBuf {
    files::path_of(root, name).unwrap()
}

fn ready<'a>(app: &'a mut App, path: &Path) -> &'a mut FlowEditor {
    match app.flows.editors.get_mut(path) {
        Some(Slot::Ready(editor)) => editor,
        _ => panic!("the editor is not ready"),
    }
}

/// Frames until the worker thread gave the flow, then a few more so the
/// graph has its sizes.
fn settle(app: &mut App, ctx: &egui::Context, path: &Path) -> Painted {
    let started = Instant::now();
    while matches!(app.flows.editors.get(path), None | Some(Slot::Loading(_))) {
        assert!(started.elapsed() < Duration::from_secs(10), "worker thread");
        frame(app, ctx, path, Vec::new());
        std::thread::sleep(Duration::from_millis(5));
    }
    for _ in 0..4 {
        frame(app, ctx, path, Vec::new());
    }
    frame(app, ctx, path, Vec::new())
}

/// Frames until the save of the editor ended.
fn saved(app: &mut App, ctx: &egui::Context, path: &Path) {
    let started = Instant::now();
    while ready(app, path).saving.is_some() {
        assert!(started.elapsed() < Duration::from_secs(10), "worker thread");
        frame(app, ctx, path, Vec::new());
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn click(
    app: &mut App,
    ctx: &egui::Context,
    path: &Path,
    (pos, button): (egui::Pos2, egui::PointerButton),
) {
    let event = |pressed| egui::Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame(app, ctx, path, vec![egui::Event::PointerMoved(pos)]);
    frame(app, ctx, path, vec![event(true)]);
    frame(app, ctx, path, vec![event(false)]);
}

fn count(text: &Painted, label: &str) -> usize {
    text.iter().filter(|(_, line)| line == label).count()
}

/// The place of the `label` nearest to `to`, a few points inside it.
fn near(text: &Painted, label: &str, to: egui::Pos2) -> egui::Pos2 {
    let pos = text
        .iter()
        .filter(|(_, line)| line == label)
        .map(|(pos, _)| *pos)
        .min_by(|a, b| a.distance(to).total_cmp(&b.distance(to)))
        .unwrap_or_else(|| panic!("{label}: {text:?}"));
    egui::pos2(pos.x + 3.0, pos.y + 3.0)
}

fn at(text: &Painted, label: &str) -> egui::Pos2 {
    near(text, label, egui::Pos2::ZERO)
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
    assert_eq!(back.step("approve").unwrap().at, Some([380, 0]));
    // The editor keeps the rules text of the file.
    let editor = FlowEditor::of_flow("/p".into(), "f", &flow);
    assert!(!editor.dirty());
    assert_eq!(editor.draft().check().unwrap(), {
        let mut same = flow.clone();
        for step in &mut same.steps {
            step.at = editor.draft().check().unwrap().step(&step.id).unwrap().at;
        }
        same
    });
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
    for absent in ["cwd", "worktree", "rules"] {
        assert!(!text.contains(absent), "{absent}: {text}");
    }
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
fn plan_text_lists_the_steps_in_run_order_with_the_rules() {
    let flow = flow::parse(FLOW).unwrap();
    let mut draft = Draft::from(&flow);
    draft.steps.reverse();
    let plan = plan_text(&draft, "feature");
    let order: Vec<usize> = [
        "# Plan and build",
        "Obey AGENTS.md.",
        "## 1. plan (step)",
        "## 2. approve (gate)",
        "## 3. build (step)",
    ]
    .iter()
    .map(|part| plan.find(part).unwrap_or_else(|| panic!("{part}: {plan}")))
    .collect();
    assert!(order.is_sorted(), "{plan}");
    // The flow file's place and the order as a Mermaid flowchart: a box
    // is a step, a diamond is a gate.
    for part in [
        "Flow file: `.terminator/flows/feature.toml`",
        "```mermaid\nflowchart LR\n",
        "    n0[\"plan\"]\n    n1{\"approve\"}\n    n2[\"build\"]\n",
        "    n0 --> n1\n    n1 --> n2\n```",
        "## Rules for every step",
        "- After: approve",
        "- Command: `claude`",
        "- You approve: Is the plan correct?",
        "- Folder: crates/app",
        "Write the plan.",
    ] {
        assert!(plan.contains(part), "{part}: {plan}");
    }
}

#[test]
fn flow_tab_opens_only_for_a_flow_name() {
    let project = project_with_flows();
    let (mut app, received, _data) = app_with_project(project.path());
    assert!(!app.open_flow_tab("p", project.path(), "bad name"));
    assert!(!app.open_flow_tab("other", project.path(), "feature"));
    assert!(app.open_flow_tab("p", project.path(), "feature"));
    // No editor, no file and no terminal before the tab paints.
    assert!(app.flows.editors.is_empty());
    assert!(!flow_path(project.path(), "feature").exists());
    assert!(received.try_recv().is_err());
}

#[test]
fn new_flow_writes_nothing_until_save_and_never_replaces_a_file() {
    let project = project_with_flows();
    let (mut app, received, _data) = app_with_project(project.path());
    let ctx = egui::Context::default();
    let path = flow_path(project.path(), "feature");
    settle(&mut app, &ctx, &path);
    assert!(!path.exists(), "nothing is written before Save");
    assert!(ready(&mut app, &path).dirty());

    ready(&mut app, &path).rules = " Obey AGENTS.md. ".into();
    ready(&mut app, &path).save();
    saved(&mut app, &ctx, &path);
    let editor = ready(&mut app, &path);
    assert_eq!(editor.error, None);
    assert!(!editor.dirty());
    let flow = flow::files::load(project.path(), "feature").unwrap();
    assert_eq!(flow.steps.len(), 3);
    assert_eq!(flow.rules.as_deref(), Some("Obey AGENTS.md."));

    // A new flow with the name of a flow that is there: Save refuses.
    let build = project.path().join(FLOW_DIR).join("build.toml");
    let before = std::fs::read_to_string(&build).unwrap();
    let editor = FlowEditor::new(project.path().into(), "build");
    app.flows
        .editors
        .insert(build.clone(), Slot::Ready(Box::new(editor)));
    ready(&mut app, &build).save();
    saved(&mut app, &ctx, &build);
    assert!(ready(&mut app, &build).error.is_some());
    assert_eq!(std::fs::read_to_string(&build).unwrap(), before);
    // The editor makes no terminal and types nothing.
    assert!(received.try_recv().is_err());
    assert!(app.post_create.by_tab.is_empty());
}

#[test]
fn save_of_a_wrong_graph_shows_the_error_and_writes_nothing() {
    let project = project_with_flows();
    let (mut app, _received, _data) = app_with_project(project.path());
    let ctx = egui::Context::default();
    let path = flow_path(project.path(), "build");
    settle(&mut app, &ctx, &path);
    let before = std::fs::read_to_string(&path).unwrap();
    let editor = ready(&mut app, &path);
    assert!(!editor.dirty(), "an opened flow has no changes");
    // Two steps with one ID.
    editor
        .snarl
        .insert_node(egui::Pos2::ZERO, Node::step("one", "claude", ""));
    assert!(editor.dirty());
    editor.save();
    assert!(editor.saving.is_none(), "a wrong flow starts no write");
    assert!(editor.error.as_deref().unwrap().contains("used twice"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
}

#[test]
fn closed_tab_keeps_a_flow_with_changes_and_drops_a_clean_one() {
    let project = project_with_flows();
    let (mut app, _received, _data) = app_with_project(project.path());
    let ctx = egui::Context::default();
    let (clean, changed) = (
        flow_path(project.path(), "build"),
        flow_path(project.path(), "feature"),
    );
    settle(&mut app, &ctx, &clean);
    settle(&mut app, &ctx, &changed);
    // A frame in which no pane paints an editor: the tabs are closed.
    app.flows.begin_frame();
    app.info = None;
    app.flow_editors_end_frame();
    assert!(!app.flows.editors.contains_key(&clean));
    assert!(ready(&mut app, &changed).dirty());
    assert!(
        app.info.as_deref().unwrap().contains("not saved"),
        "{:?}",
        app.info
    );
    // The same flow opens again with its changes.
    ready(&mut app, &changed).title = "Renamed".into();
    let text = settle(&mut app, &ctx, &changed);
    assert_eq!(count(&text, "Renamed"), 1, "{text:?}");
}

#[test]
fn wrong_flow_file_shows_its_error_and_open_as_text() {
    let project = project_with_flows();
    let (mut app, _received, _data) = app_with_project(project.path());
    let ctx = egui::Context::default();
    let path = flow_path(project.path(), "broken");
    let text = settle(&mut app, &ctx, &path);
    assert!(matches!(
        app.flows.editors.get(&path),
        Some(Slot::Failed(_))
    ));
    for label in ["Open as text", "Read again"] {
        assert_eq!(count(&text, label), 1, "{label}: {text:?}");
    }
}

#[test]
fn editor_tab_shows_the_toolbar_the_steps_and_the_gate() {
    let project = project_with_flows();
    let (mut app, _received, _data) = app_with_project(project.path());
    let ctx = egui::Context::default();
    let path = flow_path(project.path(), "feature");
    let text = settle(&mut app, &ctx, &path);
    for label in [
        "feature",
        "Step",
        "Gate",
        "Save",
        "Rules",
        "Preview",
        "Export",
        "3 steps · New flow, not saved",
        "STEP",
        "GATE",
        "plan",
        "approve",
        "build",
        "claude",
        "Write the plan in PLAN.md.",
        "Is the plan correct?",
        "After",
        "the start",
        "plan ×",
        "approve ×",
    ] {
        assert!(count(&text, label) > 0, "{label}: {text:?}");
    }
    // No menu and no dialog shows before a click.
    for label in ["New step here", "Duplicate", "Remove", "Flow rules"] {
        assert_eq!(count(&text, label), 0, "{label}: {text:?}");
    }
}

#[test]
fn new_flow_nodes_show_in_one_row_from_left_to_right() {
    let project = project_with_flows();
    let (mut app, _received, _data) = app_with_project(project.path());
    let ctx = egui::Context::default();
    let path = flow_path(project.path(), "feature");
    let text = settle(&mut app, &ctx, &path);
    let (plan, approve, build) = (at(&text, "plan"), at(&text, "approve"), at(&text, "build"));
    assert!(
        (plan.y - approve.y).abs() < 1.0 && (approve.y - build.y).abs() < 1.0,
        "one row: {plan:?} {approve:?} {build:?}"
    );
    let (first, second) = (approve.x - plan.x, build.x - approve.x);
    assert!(
        first > 100.0 && (first - second).abs() < 8.0,
        "equal column steps: {plan:?} {approve:?} {build:?}"
    );
    // The first view shows all nodes: the last one is not cut off.
    assert!(plan.x > 0.0 && build.x < 1300.0, "{plan:?} {build:?}");
}

#[test]
fn right_click_shows_the_canvas_menu_and_the_card_menu() {
    let project = project_with_flows();
    let (mut app, _received, _data) = app_with_project(project.path());
    let ctx = egui::Context::default();
    let path = flow_path(project.path(), "feature");
    let text = settle(&mut app, &ctx, &path);
    assert_eq!(count(&text, "Add step"), 0);
    let secondary = egui::PointerButton::Secondary;

    // Empty canvas above the cards.
    let empty = at(&text, "approve") - egui::vec2(0.0, 200.0);
    click(&mut app, &ctx, &path, (empty, secondary));
    let text = frame(&mut app, &ctx, &path, Vec::new());
    for label in ["Add step", "Add gate", "Fit to view"] {
        assert_eq!(count(&text, label), 1, "{label}: {text:?}");
    }
    let escape = egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    frame(&mut app, &ctx, &path, vec![escape]);

    // The type label of a card.
    click(&mut app, &ctx, &path, (at(&text, "GATE"), secondary));
    let text = frame(&mut app, &ctx, &path, Vec::new());
    for label in ["Edit…", "Duplicate", "Remove wires", "Remove"] {
        assert_eq!(count(&text, label), 1, "{label}: {text:?}");
    }
}

#[test]
fn after_chip_removes_a_wire_and_plus_adds_one() {
    let project = project_with_flows();
    let (mut app, _received, _data) = app_with_project(project.path());
    let ctx = egui::Context::default();
    let path = flow_path(project.path(), "feature");
    let text = settle(&mut app, &ctx, &path);
    let needs = |app: &mut App, id: &str| {
        let draft = ready(app, &path).draft();
        draft
            .steps
            .iter()
            .find(|step| step.id == id)
            .unwrap()
            .needs
            .clone()
    };
    let primary = egui::PointerButton::Primary;
    assert_eq!(needs(&mut app, "build"), ["approve"]);
    click(&mut app, &ctx, &path, (at(&text, "approve ×"), primary));
    assert!(needs(&mut app, "build").is_empty());

    // The gate's "+" is the one next to its "plan ×" chip. Its menu has
    // the step that the gate does not need yet, "build", below the "+".
    let text = frame(&mut app, &ctx, &path, Vec::new());
    let plus = near(&text, "+", at(&text, "plan ×"));
    click(&mut app, &ctx, &path, (plus, primary));
    let text = frame(&mut app, &ctx, &path, Vec::new());
    click(&mut app, &ctx, &path, (near(&text, "build", plus), primary));
    assert_eq!(needs(&mut app, "approve"), ["build", "plan"]);
}

#[test]
fn click_on_a_prompt_opens_the_card_dialog_with_large_fields() {
    let project = project_with_flows();
    let (mut app, _received, _data) = app_with_project(project.path());
    let ctx = egui::Context::default();
    let path = flow_path(project.path(), "feature");
    let text = settle(&mut app, &ctx, &path);
    let prompt = at(&text, "Write the plan in PLAN.md.");
    click(
        &mut app,
        &ctx,
        &path,
        (prompt, egui::PointerButton::Primary),
    );
    assert!(matches!(
        ready(&mut app, &path).dialog,
        Some(Dialog::Node(_))
    ));
    for _ in 0..3 {
        frame(&mut app, &ctx, &path, Vec::new());
    }
    let text = frame(&mut app, &ctx, &path, Vec::new());
    for label in [
        "Edit step",
        "Command",
        "Prompt",
        "Folder",
        "Worktree",
        "Done",
    ] {
        assert!(count(&text, label) > 0, "{label}: {text:?}");
    }

    // The rules dialog and the preview.
    ready(&mut app, &path).dialog = Some(Dialog::Rules);
    for _ in 0..3 {
        frame(&mut app, &ctx, &path, Vec::new());
    }
    let text = frame(&mut app, &ctx, &path, Vec::new());
    for label in ["Flow rules", "Rules for every step"] {
        assert!(count(&text, label) > 0, "{label}: {text:?}");
    }
    ready(&mut app, &path).dialog = Some(Dialog::Preview);
    for _ in 0..3 {
        frame(&mut app, &ctx, &path, Vec::new());
    }
    let text = frame(&mut app, &ctx, &path, Vec::new());
    for label in ["Flow plan", "Copy plan", "Save as feature.md"] {
        assert!(count(&text, label) > 0, "{label}: {text:?}");
    }
}

#[test]
fn cards_show_the_newest_run_with_approve_on_a_gate_that_waits() {
    use terminator_core::flow::{RunState, StepStatus};
    let project = project_with_flows();
    let (mut app, _received, _data) = app_with_project(project.path());
    let ctx = egui::Context::default();
    let path = flow_path(project.path(), "build");
    let text = settle(&mut app, &ctx, &path);
    assert_eq!(count(&text, "Approve"), 0, "no run: {text:?}");

    let flow = flow::files::load(project.path(), "build").unwrap();
    let mut state = RunState::new(&flow, "p");
    state.steps.get_mut("one").unwrap().status = StepStatus::Done;
    state.steps.get_mut("one").unwrap().session = Some("s1".into());
    state.steps.get_mut("ok").unwrap().status = StepStatus::Running;
    app.flows.runs = vec![crate::flows::Run {
        dir: project.path().join("run"),
        state,
    }];
    // No read of the run folders replaces this run in the next frames.
    app.flows.read_at = Some(Instant::now());
    let text = settle(&mut app, &ctx, &path);
    for label in [
        "Done",
        "Show terminal",
        "Waits for your approval",
        "Approve",
        "Reject",
        "2 steps · Saved · Run: running",
    ] {
        assert_eq!(count(&text, label), 1, "{label}: {text:?}");
    }
}

#[test]
fn run_keeps_the_app_on_the_editor_until_the_user_goes_away() {
    let project = project_with_flows();
    let (mut app, _received, _data) = app_with_project(project.path());
    let path = flow_path(project.path(), "build");
    let stay = |app: &App| {
        app.flows
            .stay
            .as_ref()
            .map(|stay| (stay.pending, stay.sessions))
    };

    // The editor is in view: nothing to do, the wait for the runner stays.
    app.flows.stay = Some(Stay::after_run(&path, &[]));
    app.flows.begin_frame();
    app.flows.mark_visible(&path);
    app.flow_editors_end_frame();
    assert_eq!(stay(&app), Some((true, 0)));

    // Another tab took the focus with no new step terminal (the user went
    // there): the editor waits a moment for a late read, and takes nothing.
    app.flows.stay.as_mut().unwrap().pending = false;
    app.flows.begin_frame();
    app.flow_editors_end_frame();
    assert_eq!(stay(&app), Some((false, 0)));
    assert!(app.flows.stay.as_ref().unwrap().away.is_some());
    app.flows.stay.as_mut().unwrap().away =
        Instant::now().checked_sub(STAY_GRACE.saturating_mul(2));
    app.flows.begin_frame();
    app.flow_editors_end_frame();
    assert_eq!(stay(&app), None);

    // The runner's terminal took the focus: the editor's tab comes back.
    // Here the editor has no tab, so the wait ends.
    app.flows.stay = Some(Stay::after_run(&path, &[]));
    app.flows.begin_frame();
    app.flow_editors_end_frame();
    assert_eq!(stay(&app), None);
}

#[test]
fn export_writes_the_plan_beside_the_flow_file() {
    let project = project_with_flows();
    let mut editor = FlowEditor::new(project.path().into(), "feature");
    editor.export_plan();
    let started = Instant::now();
    let plan = loop {
        assert!(started.elapsed() < Duration::from_secs(10), "worker thread");
        if let Some(plan) = editor.poll_export() {
            break plan;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(plan, project.path().join(FLOW_DIR).join("feature.md"));
    let text = std::fs::read_to_string(plan).unwrap();
    assert!(text.contains("## 2. approve (gate)"), "{text}");
    // The export makes no flow file.
    assert!(!flow_path(project.path(), "feature").exists());
}

#[test]
fn first_view_shows_all_nodes_in_a_small_canvas() {
    let canvas = egui::Rect::from_min_size(egui::pos2(300.0, 200.0), egui::vec2(600.0, 500.0));
    let view = viewer::fit_view(&starter(), canvas).unwrap();
    assert!(view.scaling < 1.0);
    for (position, _) in starter().nodes_pos() {
        let node = egui::Rect::from_min_size(position, egui::vec2(300.0, 250.0));
        assert!(canvas.contains_rect(view * node), "{node:?}");
    }
    assert!(viewer::fit_view(&Snarl::new(), canvas).is_none());
}
