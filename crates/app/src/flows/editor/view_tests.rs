//! The redesigned flow editor: a fit from the measured cards, a layout
//! with no overlap, the inspector, the check errors, the broken-flow card,
//! the tab identity and the editor keys.
use super::canvas::{NODE_SIZE, Sizes, fit_view};
use super::failed::failing_line;
use super::inspector::{Field, check_error};
use super::model::{self, Node, ROW, draft_of, graph_of, input, output, starter, would_cycle};
use super::*;
use crate::flows::tests::{app_with_project, project_with_flows};
use terminator_core::flow::{self, FLOW_DIR};

type Painted = Vec<(egui::Pos2, String)>;

/// Five steps; `build` and `docs` share a column.
const FIVE: &str = r#"
version = 1
name = "Plan, build and review"

[[step]]
id = "plan"
command = "claude"
prompt = "Read the issue and write a plan in PLAN.md. List the files that change and the tests to add."
worktree = "feature"

[[step]]
id = "approve-plan"
gate = "Is the plan in PLAN.md correct?"
needs = ["plan"]

[[step]]
id = "build"
command = "claude"
prompt = "Do the plan in PLAN.md. Run cargo test before you report done."
worktree = "build"
needs = ["approve-plan"]

[[step]]
id = "docs"
command = "codex"
prompt = "Update docs/ for the change in PLAN.md."
cwd = "docs"
needs = ["approve-plan"]

[[step]]
id = "review"
command = "claude"
prompt = "Review the diff of build and docs. List bugs only."
needs = ["build", "docs"]
"#;

/// One frame of the editor of `path` in a pane `width` wide.
fn frame(
    app: &mut App,
    ctx: &egui::Context,
    (path, width): (&Path, f32),
    events: Vec<egui::Event>,
) -> Painted {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 900.0),
            )),
            events,
            ..Default::default()
        },
        |ui| app.flow_editor_view(ui, path),
    );
    output.textures_delta.clear();
    crate::app::tests::support::painted_text(&output.shapes)
}

fn ready<'a>(app: &'a mut App, path: &Path) -> &'a mut FlowEditor {
    match app.flows.editors.get_mut(path) {
        Some(Slot::Ready(editor)) => editor,
        _ => panic!("the editor is not ready"),
    }
}

fn settle(app: &mut App, ctx: &egui::Context, (path, width): (&Path, f32)) -> Painted {
    let started = Instant::now();
    while matches!(app.flows.editors.get(path), None | Some(Slot::Loading(_))) {
        assert!(started.elapsed() < Duration::from_secs(10), "worker thread");
        frame(app, ctx, (path, width), Vec::new());
        std::thread::sleep(Duration::from_millis(5));
    }
    for _ in 0..5 {
        frame(app, ctx, (path, width), Vec::new());
    }
    frame(app, ctx, (path, width), Vec::new())
}

fn five(project: &tempfile::TempDir) -> PathBuf {
    let path = project.path().join(FLOW_DIR).join("five.toml");
    std::fs::write(&path, FIVE).unwrap();
    path
}

fn count(text: &Painted, label: &str) -> usize {
    text.iter().filter(|(_, line)| line == label).count()
}

fn at(text: &Painted, label: &str, near: egui::Pos2) -> egui::Pos2 {
    let pos = text
        .iter()
        .filter(|(_, line)| line == label)
        .map(|(pos, _)| *pos)
        .min_by(|a, b| a.distance(near).total_cmp(&b.distance(near)))
        .unwrap_or_else(|| panic!("{label}: {text:?}"));
    egui::pos2(pos.x + 3.0, pos.y + 3.0)
}

fn click(app: &mut App, ctx: &egui::Context, path: &Path, pos: egui::Pos2) {
    let event = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame(
        app,
        ctx,
        (path, 1600.0),
        vec![egui::Event::PointerMoved(pos)],
    );
    frame(app, ctx, (path, 1600.0), vec![event(true)]);
    frame(app, ctx, (path, 1600.0), vec![event(false)]);
}

/// The graph rectangle of each card from its measured size.
fn card_rects(editor: &FlowEditor) -> Vec<(String, egui::Rect)> {
    editor
        .snarl
        .nodes_pos_ids()
        .map(|(node, pos, value)| {
            let size = editor.sizes.get(&node).copied().unwrap_or(NODE_SIZE);
            (value.id().to_owned(), egui::Rect::from_min_size(pos, size))
        })
        .collect()
}

fn assert_no_overlap(rects: &[(String, egui::Rect)]) {
    for (index, (left, a)) in rects.iter().enumerate() {
        for (right, b) in rects.iter().skip(index.saturating_add(1)) {
            assert!(!a.intersects(*b), "{left} {a:?} overlaps {right} {b:?}");
        }
    }
}

#[test]
fn fit_uses_the_measured_card_sizes() {
    let snarl = starter();
    // Cards taller and wider than the estimate.
    let sizes: Sizes = snarl
        .node_ids()
        .map(|(node, _)| (node, egui::vec2(320.0, 420.0)))
        .collect();
    // A canvas where the measured cards fit at 0.9, above the zoom floor.
    let all = super::canvas::bounds(&snarl, &sizes).expand(40.0);
    let canvas = egui::Rect::from_min_size(egui::pos2(300.0, 200.0), all.size() * 0.9);
    let view = fit_view(&snarl, &sizes, (canvas, 0.0)).unwrap();
    assert!((view.scaling - 0.9).abs() < 0.01, "{}", view.scaling);
    for (node, position, _) in snarl.nodes_pos_ids() {
        let card = egui::Rect::from_min_size(position, sizes[&node]);
        assert!(
            canvas.contains_rect(view.mul_rect(card)),
            "{card:?} in {canvas:?}"
        );
    }
    // The estimate alone would cut these cards off.
    let guess = fit_view(&snarl, &Sizes::new(), (canvas, 0.0)).unwrap();
    assert!(guess.scaling > view.scaling);
    assert!(fit_view(&Snarl::new(), &Sizes::new(), (canvas, 0.0)).is_none());
}

#[test]
fn fit_beside_the_inspector_shows_every_card_of_a_flow_that_fits_the_pane() {
    let snarl = graph_of(&flow::parse(FIVE).unwrap());
    let all = super::canvas::bounds(&snarl, &Sizes::new()).expand(40.0);
    // A pane where the flow fits at the zoom floor, less the inspector.
    let pane = all.width() * super::canvas::FIT_MIN_ZOOM + 20.0;
    let inspector = super::inspector::WIDTH;
    let canvas =
        egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(pane - inspector, 800.0));
    let view = fit_view(&snarl, &Sizes::new(), (canvas, inspector)).unwrap();
    assert!(view.scaling < super::canvas::FIT_MIN_ZOOM);
    for (_, position, _) in snarl.nodes_pos_ids() {
        let card = view.mul_rect(egui::Rect::from_min_size(position, NODE_SIZE));
        assert!(canvas.contains_rect(card), "{card:?} in {canvas:?}");
    }
    // Without the inspector the same canvas keeps the zoom floor.
    let alone = fit_view(&snarl, &Sizes::new(), (canvas, 0.0)).unwrap();
    assert!((alone.scaling - super::canvas::FIT_MIN_ZOOM).abs() < f32::EPSILON);
    // A flow too wide for the whole pane keeps the floor and its first card.
    let narrow = egui::Rect::from_min_size(canvas.min, egui::vec2(400.0, 800.0));
    let wide = fit_view(&snarl, &Sizes::new(), (narrow, inspector)).unwrap();
    assert!((wide.scaling - super::canvas::FIT_MIN_ZOOM).abs() < f32::EPSILON);
}

#[test]
fn opening_the_inspector_fits_the_cards_beside_it() {
    let project = project_with_flows();
    let path = five(&project);
    let (mut app, _received, _data) = app_with_project(project.path());
    let ctx = egui::Context::default();
    // Narrower than SPLIT_WIDTH: no inspector until the rules open.
    let width = 1250.0;
    settle(&mut app, &ctx, (&path, width));
    ready(&mut app, &path).rules_open = true;
    let text = settle(&mut app, &ctx, (&path, width));
    assert!(count(&text, "Flow rules") >= 1, "{text:?}");
    let split = width - super::inspector::WIDTH;
    for id in ["plan", "approve-plan", "build", "docs", "review"] {
        let pos = at(&text, id, egui::Pos2::ZERO);
        assert!(pos.x > 0.0 && pos.x < split - 60.0, "{id}: {pos:?}");
    }
}

#[test]
fn fit_keeps_cards_readable_and_shows_the_first_card_of_a_large_flow() {
    let snarl = graph_of(&flow::parse(FIVE).unwrap());
    let canvas = egui::Rect::from_min_size(egui::pos2(300.0, 200.0), egui::vec2(600.0, 400.0));
    let view = fit_view(&snarl, &Sizes::new(), (canvas, 0.0)).unwrap();
    assert!((view.scaling - super::canvas::FIT_MIN_ZOOM).abs() < f32::EPSILON);
    // The leftmost card starts inside the canvas; the user pans to the rest.
    let first = super::canvas::bounds(&snarl, &Sizes::new());
    let shown = view.mul_pos(first.left_top());
    assert!(
        canvas.x_range().contains(shown.x),
        "{shown:?} in {canvas:?}"
    );
    assert!(
        canvas.y_range().contains(shown.y),
        "{shown:?} in {canvas:?}"
    );
}

#[test]
fn auto_layout_and_tidy_layout_never_overlap_cards() {
    let flow = flow::parse(FIVE).unwrap();
    let mut snarl = graph_of(&flow);
    let draft = draft_of(&flow.name, &snarl);
    // `build` and `docs` share a column, one ROW apart.
    let (build, docs) = (draft.steps[2].at.unwrap(), draft.steps[3].at.unwrap());
    assert_eq!((build[0], docs[1] - build[1]), (docs[0], ROW));
    // Tidy stacks the measured cards of a column with a gap.
    let tall: Sizes = snarl
        .node_ids()
        .map(|(node, _)| (node, egui::vec2(220.0, 400.0)))
        .collect();
    model::tidy(&mut snarl, |node| tall.get(&node).copied());
    let rects: Vec<_> = snarl
        .nodes_pos_ids()
        .map(|(node, pos, value)| {
            (
                value.id().to_owned(),
                egui::Rect::from_min_size(pos, tall[&node]),
            )
        })
        .collect();
    assert_no_overlap(&rects);
    // The wires and the content are the same.
    let after = draft_of(&flow.name, &snarl);
    assert_eq!(model::content_of(after), model::content_of(draft));
}

#[test]
fn painted_cards_fit_their_rows_and_the_first_view() {
    let project = project_with_flows();
    let path = five(&project);
    let (mut app, _received, _data) = app_with_project(project.path());
    let ctx = egui::Context::default();
    let text = settle(&mut app, &ctx, (&path, 1600.0));
    let editor = ready(&mut app, &path);
    assert!(canvas::measured(&editor.snarl, &editor.sizes));
    for (id, rect) in card_rects(editor) {
        assert!(
            rect.height() < f32::from(u16::try_from(ROW).unwrap()),
            "{id}: {rect:?}"
        );
        assert!(rect.width() < 300.0, "{id}: {rect:?}");
    }
    assert_no_overlap(&card_rects(editor));
    // Every card shows in the canvas: none is cut off at the right.
    for id in ["plan", "approve-plan", "build", "docs", "review"] {
        let pos = at(&text, id, egui::Pos2::ZERO);
        assert!(
            pos.x > 0.0 && pos.x < 1600.0 - 300.0 && pos.y < 900.0,
            "{id}: {pos:?}"
        );
    }
    // Tidy layout moves cards: a layout change only.
    editor
        .snarl
        .get_node_info_mut(editor.snarl.node_ids().next().unwrap().0)
        .unwrap()
        .pos = egui::pos2(5.0, 700.0);
    let sizes = editor.sizes.clone();
    model::tidy(&mut editor.snarl, |node| sizes.get(&node).copied());
    assert_no_overlap(&card_rects(editor));
    assert!(editor.dirty() && !editor.changed());
}

#[test]
fn inspector_edits_the_selected_card_and_refuses_a_cycle() {
    let project = project_with_flows();
    let (mut app, _received, _data) = app_with_project(project.path());
    let ctx = egui::Context::default();
    let path = files::path_of(project.path(), "feature").unwrap();
    let text = settle(&mut app, &ctx, (&path, 1600.0));
    assert_eq!(count(&text, "Flow rules"), 1, "rules with no selection");
    // A click on a card selects it; the inspector shows its fields.
    click(&mut app, &ctx, &path, at(&text, "build", egui::Pos2::ZERO));
    let text = frame(&mut app, &ctx, (&path, 1600.0), Vec::new());
    for label in [
        "Step",
        "ID",
        "Command",
        "Prompt",
        "Worktree",
        "Folder",
        "Runs after",
    ] {
        assert!(count(&text, label) > 0, "{label}: {text:?}");
    }
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
    // build runs after approve; a click on plan adds that need.
    let list = at(&text, "Runs after", egui::Pos2::ZERO);
    click(&mut app, &ctx, &path, at(&text, "plan", list));
    assert_eq!(needs(&mut app, "build"), ["approve", "plan"]);

    // plan: build and approve run after it, so both would close a cycle.
    let editor = ready(&mut app, &path);
    let ids: std::collections::HashMap<String, NodeId> = editor
        .snarl
        .node_ids()
        .map(|(node, value)| (value.id().to_owned(), node))
        .collect();
    assert!(would_cycle(&editor.snarl, ids["build"], ids["plan"]));
    assert!(would_cycle(&editor.snarl, ids["plan"], ids["plan"]));
    assert!(!would_cycle(&editor.snarl, ids["plan"], ids["build"]));
    editor.selected = Some(ids["plan"]);
    let text = frame(&mut app, &ctx, (&path, 1600.0), Vec::new());
    let list = at(&text, "Runs after", egui::Pos2::ZERO);
    click(&mut app, &ctx, &path, at(&text, "build", list));
    assert!(needs(&mut app, "plan").is_empty(), "no cycle is made");
    // A click on the empty canvas shows the rules again.
    click(&mut app, &ctx, &path, egui::pos2(40.0, 700.0));
    assert_eq!(ready(&mut app, &path).selected, None);
}

#[test]
fn check_errors_name_their_step_and_field() {
    let mut snarl = starter();
    snarl.insert_node(egui::Pos2::ZERO, Node::step("plan", "claude", ""));
    let error = check_error(&draft_of("f", &snarl)).unwrap();
    assert_eq!(
        (error.step.as_deref(), error.field),
        (Some("plan"), Field::Id)
    );
    let mut snarl = starter();
    for (_, value) in snarl.nodes_ids_mut() {
        if let Node::Step { worktree, .. } = value {
            *worktree = "same".into();
        }
    }
    let error = check_error(&draft_of("f", &snarl)).unwrap();
    assert_eq!(error.field, Field::Worktree);
    assert!(check_error(&draft_of("f", &starter())).is_none());
    // A wire that would close a cycle: the check names the step.
    let mut snarl = starter();
    let (first, last) = (
        snarl.node_ids().next().unwrap().0,
        snarl.node_ids().last().unwrap().0,
    );
    snarl.connect(output(last), input(first));
    assert_eq!(
        check_error(&draft_of("f", &snarl)).unwrap().field,
        Field::RunsAfter
    );
}

#[test]
fn broken_flow_shows_a_card_that_opens_the_failing_line() {
    let text = "version = 1\nname = \"x\"\n\n[[step]]\nid = \"build\"\ncommand = \"\"\n";
    let error = flow::files::LoadError::Flow(flow::parse(text).unwrap_err());
    assert_eq!(failing_line(text, &error), Some(5));
    let syntax = "version = 1\nname = \"x\"\nextra = 1\n";
    let error = flow::files::LoadError::Flow(flow::parse(syntax).unwrap_err());
    assert!(failing_line(syntax, &error).is_some(), "{error}");

    let project = project_with_flows();
    let (mut app, _received, _data) = app_with_project(project.path());
    let ctx = egui::Context::default();
    let path = files::path_of(project.path(), "broken").unwrap();
    let text = settle(&mut app, &ctx, (&path, 1200.0));
    for label in [
        "This flow cannot open in the editor",
        "Open as text",
        "Read again",
    ] {
        assert_eq!(count(&text, label), 1, "{label}: {text:?}");
    }
    let Some(Slot::Failed(failure)) = app.flows.editors.get(&path) else {
        panic!("a failed flow");
    };
    assert!(failure.line.is_some(), "{failure:?}");
}

#[test]
fn narrow_toolbar_keeps_the_title_state_save_and_run_in_view() {
    let project = project_with_flows();
    let (mut app, _received, _data) = app_with_project(project.path());
    let ctx = egui::Context::default();
    let path = files::path_of(project.path(), "build").unwrap();
    let text = settle(&mut app, &ctx, (&path, 1600.0));
    for label in ["Build", "Saved", "Step", "Gate", "Save", "Run"] {
        assert_eq!(count(&text, label), 1, "{label}: {text:?}");
    }
    let text = settle(&mut app, &ctx, (&path, 520.0));
    for label in ["Build", "Saved", "Save", "Run"] {
        let pos = at(&text, label, egui::Pos2::ZERO);
        assert!(pos.x < 520.0, "{label} is cut off: {pos:?}");
    }
    assert_eq!(
        count(&text, "Step"),
        0,
        "Step is in the More menu: {text:?}"
    );
}

#[test]
fn flow_tab_shows_the_flow_name_and_the_unsaved_dot() {
    let project = project_with_flows();
    let (mut app, _received, _data) = app_with_project(project.path());
    let ctx = egui::Context::default();
    let path = files::path_of(project.path(), "build").unwrap();
    assert_eq!(app.flow_tab_title(&path), "build", "before the read");
    settle(&mut app, &ctx, (&path, 1600.0));
    assert_eq!(app.flow_tab_title(&path), "Build");
    assert!(!app.flow_tab_dirty(&path));
    ready(&mut app, &path).title = "Build 2".into();
    assert!(app.flow_tab_dirty(&path));
}

fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

/// Runs the shortcut dispatch on `events`; gives the events left over.
fn dispatch(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) -> usize {
    let mut left = 0;
    let mut output = ctx.run_ui(
        egui::RawInput {
            events,
            ..Default::default()
        },
        |ui| {
            app.dispatch_shortcuts(ui.ctx(), false);
            left = ui.input(|input| input.events.len());
        },
    );
    output.textures_delta.clear();
    left
}

#[test]
fn editor_keys_act_on_the_focused_flow_canvas_only() {
    use crate::app::tests::support::drain_updates;
    let project = project_with_flows();
    let (mut app, _received, _data) = app_with_project(project.path());
    let ctx = egui::Context::default();
    let path = files::path_of(project.path(), "build").unwrap();
    // No flow editor has the focus: F is no key of the app.
    assert_eq!(
        dispatch(
            &mut app,
            &ctx,
            vec![key(egui::Key::F, egui::Modifiers::NONE)]
        ),
        1
    );
    assert!(app.open_flow_tab("p", project.path(), "build"));
    drain_updates(&mut app, &ctx);
    settle(&mut app, &ctx, (&path, 1600.0));
    assert_eq!(app.active_flow_editor().as_deref(), Some(path.as_path()));
    let nodes = |app: &mut App| ready(app, &path).snarl.nodes().count();

    // F fits the view; the key never goes on to a pane or a PTY.
    assert_eq!(
        dispatch(
            &mut app,
            &ctx,
            vec![key(egui::Key::F, egui::Modifiers::NONE)]
        ),
        0
    );
    assert!(ready(&mut app, &path).fit);
    // Duplicate and Remove act on the selected card.
    let first = ready(&mut app, &path).snarl.node_ids().next().unwrap().0;
    ready(&mut app, &path).selected = Some(first);
    let duplicate = shortcuts::parse(&shortcuts::binding(
        &app.state.settings.keybindings,
        Action::FlowDuplicate.id(),
    ))
    .unwrap();
    dispatch(
        &mut app,
        &ctx,
        vec![key(duplicate.key, duplicate.modifiers)],
    );
    assert_eq!(nodes(&mut app), 3);
    let copy = ready(&mut app, &path).selected.unwrap();
    assert_ne!(copy, first, "the copy is selected");
    // The bound remove key goes through the keymap; the other delete key
    // (Backspace or Delete) works on the canvas too.
    let remove = shortcuts::parse(&shortcuts::binding(
        &app.state.settings.keybindings,
        Action::FlowRemove.id(),
    ))
    .unwrap();
    dispatch(&mut app, &ctx, vec![key(remove.key, remove.modifiers)]);
    assert_eq!(nodes(&mut app), 2);
    let other = if remove.key == egui::Key::Delete {
        egui::Key::Backspace
    } else {
        egui::Key::Delete
    };
    ready(&mut app, &path).selected = Some(first);
    frame(
        &mut app,
        &ctx,
        (&path, 1600.0),
        vec![key(other, egui::Modifiers::NONE)],
    );
    assert_eq!(nodes(&mut app), 1);
    // The Save key saves the flow: it is a change now.
    let save = shortcuts::parse(&shortcuts::binding(
        &app.state.settings.keybindings,
        Action::EditorSave.id(),
    ))
    .unwrap();
    assert!(ready(&mut app, &path).dirty());
    dispatch(&mut app, &ctx, vec![key(save.key, save.modifiers)]);
    let editor = ready(&mut app, &path);
    assert!(
        editor.saving.is_some() || editor.error.is_some(),
        "Save ran"
    );
}
