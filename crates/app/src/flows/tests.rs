use super::*;
use std::path::Path;
use std::sync::mpsc;
use terminator_core::flow::{self, FLOW_DIR, StepRun, StepStatus};

const FLOW: &str = "version = 1\nname = \"Build\"\n[[step]]\nid = \"one\"\ncommand = \"claude\"\n[[step]]\nid = \"ok\"\ngate = \"Good?\"\nneeds = [\"one\"]\n";

pub(super) fn project_with_flows() -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("project");
    let flows = project.path().join(FLOW_DIR);
    std::fs::create_dir_all(&flows).expect("dir");
    std::fs::write(flows.join("build.toml"), FLOW).expect("flow");
    std::fs::write(
        flows.join("broken.toml"),
        "version = 1\nname = \"x\"\nextra = 1\n",
    )
    .expect("flow");
    std::fs::write(flows.join("bad name.toml"), FLOW).expect("flow");
    std::fs::write(flows.join("notes.md"), "not a flow").expect("file");
    project
}

pub(super) fn app_with_project(root: &Path) -> (App, mpsc::Receiver<Job>, tempfile::TempDir) {
    let data = tempfile::tempdir().expect("data dir");
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(data.path().into()));
    app.preferences_writable = false;
    app.apply_state(State {
        projects: vec![Project {
            id: "p".into(),
            name: "p".into(),
            path: root.to_path_buf(),
            layout: serde_json::Value::Null,
        }],
        selected_project: Some("p".into()),
        ..Default::default()
    });
    app.selected = Some("p".into());
    let (jobs, received) = mpsc::channel();
    app.jobs = jobs.into();
    (app, received, data)
}

#[test]
fn flow_files_are_listed_with_the_reason_a_file_is_refused() {
    let project = project_with_flows();
    let loaded = load_flows("p".into(), project.path().into());
    let names: Vec<&str> = loaded.files.iter().map(|file| file.file.as_str()).collect();
    assert_eq!(names, ["bad name", "broken", "build"]);
    assert!(loaded.files[0].flow.is_err(), "a name with a space");
    assert!(loaded.files[1].flow.is_err(), "an unknown key");
    assert_eq!(loaded.files[2].flow, Ok(("Build".into(), 2)));
    let none = tempfile::tempdir().expect("project");
    assert!(load_flows("p".into(), none.path().into()).files.is_empty());
}

#[test]
fn flows_window_has_run_edit_and_new_flow() {
    let project = project_with_flows();
    let (mut app, received, _data) = app_with_project(project.path());
    app.flows.loaded = Some(load_flows("p".into(), project.path().into()));
    app.flows.open = true;
    let ctx = egui::Context::default();
    // A screen wider than the window, so its right end is in view.
    let wide = |app: &mut App| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                ..Default::default()
            },
            |ui| app.flow_dialog(ui.ctx()),
        );
        output.textures_delta.clear();
        crate::app::tests::support::painted_strings(&output.shapes)
    };
    // An egui window sizes itself in its first frames and paints after.
    for _ in 0..3 {
        wide(&mut app);
    }
    let text = wide(&mut app);
    for label in [
        "Run flow",
        "Flows of this project",
        "Run",
        "Edit",
        "Build",
        "build.toml · 2 steps",
        "New flow",
        "Create",
    ] {
        assert!(text.iter().any(|line| line == label), "{label}: {text:?}");
    }
    // Painting the window makes no file and no terminal.
    assert!(received.try_recv().is_err());
    assert_eq!(flow::files::names(project.path()).len(), 3);
}

#[test]
fn run_command_quotes_each_word() {
    let line = run_command(Path::new("/Apps/My App/terminator-hook"), "p'1", "build");
    assert_eq!(
        line,
        "'/Apps/My App/terminator-hook' flow run 'p'\\''1' 'build'"
    );
}

#[test]
fn run_flow_types_nothing_before_trust() {
    let project = project_with_flows();
    let (mut app, received, _data) = app_with_project(project.path());
    let loaded = load_flows("p".into(), project.path().into());

    app.run_flow(&loaded, "build");
    assert!(received.try_recv().is_err(), "no terminal before trust");
    assert!(app.post_create.by_tab.is_empty());

    app.answer_task_trust(project.path(), false);
    app.run_flow(&loaded, "build");
    assert!(received.try_recv().is_err(), "no terminal when not trusted");

    app.answer_task_trust(project.path(), true);
    app.run_flow(&loaded, "bad name");
    assert!(
        received.try_recv().is_err(),
        "a bad file name types nothing"
    );

    app.run_flow(&loaded, "build");
    let Ok(Job::Control(_, After::Workspace(tab, _))) = received.try_recv() else {
        panic!("expected a new terminal");
    };
    let post = app.post_create.by_tab.get(&tab).expect("typed text");
    assert!(post.enter, "the explicit Run presses Enter on the runner");
    assert!(
        post.input.ends_with(" flow run 'p' 'build'"),
        "{}",
        post.input
    );
    assert!(post.input.contains("terminator-hook"), "{}", post.input);
}

#[test]
fn flows_tool_window_shows_after_a_run_and_from_the_menu_action() {
    let project = project_with_flows();
    let (mut app, _received, _data) = app_with_project(project.path());
    let loaded = load_flows("p".into(), project.path().into());
    app.preferences.tool = SidebarTool::Explorer;
    app.preferences.visible = false;

    // A refused run changes nothing in the layout.
    app.run_flow(&loaded, "build");
    assert_eq!(app.preferences.tool, SidebarTool::Explorer);
    assert!(!app.preferences.visible);

    app.answer_task_trust(project.path(), true);
    app.run_flow(&loaded, "build");
    assert_eq!(app.preferences.tool, SidebarTool::Flows);
    assert!(app.preferences.visible, "the run is in view");

    // Show flows: also when the right sidebar holds the project list.
    app.preferences.tool = SidebarTool::Git;
    app.preferences.right_projects = true;
    app.open_flows_tool();
    assert_eq!(app.preferences.tool, SidebarTool::Flows);
    assert!(!app.preferences.right_projects);
}

/// Every text that one frame of `paint` draws.
pub(super) fn painted(ctx: &egui::Context, paint: impl FnMut(&mut egui::Ui)) -> Vec<String> {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(320.0, 600.0),
            )),
            ..Default::default()
        },
        paint,
    );
    output.textures_delta.clear();
    crate::app::tests::support::painted_strings(&output.shapes)
}

/// Every text of the third frame of `paint`: an egui window sizes itself
/// in its first frames and paints after.
pub(super) fn painted_frames(
    ctx: &egui::Context,
    mut paint: impl FnMut(&mut egui::Ui),
) -> Vec<String> {
    for _ in 0..2 {
        painted(ctx, &mut paint);
    }
    painted(ctx, paint)
}

#[test]
fn new_tab_menu_has_the_flow_rows() {
    let project = project_with_flows();
    let (mut app, received, _data) = app_with_project(project.path());
    let ctx = egui::Context::default();
    let text = painted(&ctx, |ui| app.flow_menu_items(ui));
    for label in ["Flows", "Run flow…", "New flow…", "Show flows"] {
        assert!(text.iter().any(|line| line == label), "{label}: {text:?}");
    }
    assert!(crate::icons::is_ui_icon("PanelRight"));
    // Painting the menu starts nothing.
    assert!(received.try_recv().is_err());
}

#[test]
fn flows_panel_shows_a_gate_with_approve_and_a_question_with_send() {
    let project = project_with_flows();
    let (mut app, _received, data) = app_with_project(project.path());
    let mut state = RunState::new(&flow::parse(FLOW).expect("flow"), "build", "p");
    state.set("one", StepStatus::Running);
    state.steps.get_mut("one").expect("step").question = Some("A or B?".into());
    state.set("ok", StepStatus::Running);
    app.flows.runs = vec![Run {
        dir: data.path().join("flows").join("build-1"),
        state,
        gone: false,
    }];
    let ctx = egui::Context::default();
    let text = painted(&ctx, |ui| app.flows_panel(ui));
    for label in [
        "Flows",
        "NEEDS YOU",
        "Build › one",
        "Build › ok",
        "Build",
        "waits",
        "0/2",
        "one",
        "A or B?",
        "Send",
        "ok",
        "Good?",
        "Approve",
        "Reject",
    ] {
        assert!(text.iter().any(|line| line == label), "{label}: {text:?}");
    }
    // A run of another project does not show.
    app.selected = Some("other".into());
    let text = painted(&ctx, |ui| app.flows_panel(ui));
    assert!(!text.iter().any(|line| line == "Approve"), "{text:?}");
}

#[test]
fn step_icon_shows_what_waits_for_the_user() {
    let step = |status, gate: Option<&str>, question: Option<&str>| StepRun {
        status,
        gate: gate.map(str::to_owned),
        question: question.map(str::to_owned),
        ..StepRun::default()
    };
    let icon = |step: &StepRun| status::step_style(step).icon;
    assert_eq!(
        icon(&step(StepStatus::Waiting, Some("ok?"), None)),
        "Circle"
    );
    assert_eq!(
        icon(&step(StepStatus::Running, Some("ok?"), None)),
        "ShieldQuestion"
    );
    assert_eq!(
        icon(&step(StepStatus::Running, None, Some("A or B?"))),
        "MessageCircleQuestion"
    );
    assert_eq!(icon(&step(StepStatus::Running, None, None)), "LoaderCircle");
    assert_eq!(icon(&step(StepStatus::Done, None, None)), "CircleCheck");
    assert_eq!(icon(&step(StepStatus::Failed, None, None)), "CircleAlert");
    assert_eq!(icon(&step(StepStatus::Cancelled, None, None)), "CircleStop");
    for status in [
        StepStatus::Waiting,
        StepStatus::Running,
        StepStatus::Done,
        StepStatus::Failed,
        StepStatus::Cancelled,
    ] {
        assert!(crate::icons::is_ui_icon(icon(&step(status, None, None))));
    }
    assert!(crate::icons::is_ui_icon("ShieldQuestion"));
    assert!(crate::icons::is_ui_icon("MessageCircleQuestion"));
}

#[test]
fn run_status_is_done_only_when_every_step_is_done() {
    let state = RunState::new(&flow::parse(FLOW).expect("flow"), "build", "p");
    let mut run = Run {
        dir: PathBuf::from("/data/flows/build-1"),
        state,
        gone: false,
    };
    assert_eq!(panel::run_status(&run), "running");
    run.gone = true;
    assert_eq!(panel::run_status(&run), "stopped (runner gone)");
    assert!(run.ended());
    run.gone = false;
    run.state.set("one", StepStatus::Done);
    run.state.ended = true;
    assert_eq!(panel::run_status(&run), "stopped");
    run.state.set("ok", StepStatus::Done);
    assert_eq!(panel::run_status(&run), "done");
}

#[test]
fn a_run_whose_runner_is_gone_is_stopped_with_no_answer_buttons() {
    let project = project_with_flows();
    let (mut app, received, data) = app_with_project(project.path());
    let mut state = RunState::new(&flow::parse(FLOW).expect("flow"), "build", "p");
    state.set("one", StepStatus::Done);
    state.set("ok", StepStatus::Running);
    let dir = data.path().join("flows").join("build-1");
    let alive = Run {
        dir: dir.clone(),
        state: state.clone(),
        gone: false,
    };
    save(&alive);
    // The worker's read: no runner holds the lock.
    let run = Run::read(dir.clone(), state);
    assert!(run.gone && run.ended());
    assert_eq!(run.state.status("ok"), StepStatus::Cancelled);
    let runner = flow::control::hold_runner(&dir).expect("lock");
    assert!(!Run::read(dir.clone(), alive.state.clone()).gone);
    drop(runner);
    // An older ended run first: the gone run is not the newest.
    app.flows.runs = vec![run_of(data.path(), "build-2", Some(2)), run];
    app.flows.read_at = Some(Instant::now());
    let ctx = egui::Context::default();
    let text = painted(&ctx, |ui| app.flows_panel(ui));
    assert_eq!(count(&text, "runner gone"), 1, "{text:?}");
    // Closed by default like an ended run, and no answer goes to it.
    assert_eq!(count(&text, "one"), 1, "{text:?}");
    app.apply_flow_act(panel::Act::Toggle(dir.clone(), true));
    let text = painted(&ctx, |ui| app.flows_panel(ui));
    assert_eq!(count(&text, "Approve"), 0, "{text:?}");
    // Clear ended runs counts it, and Remove deletes it.
    app.apply_flow_act(panel::Act::AskClear);
    let text = painted_frames(&ctx, |ui| app.flows_panel(ui));
    assert_eq!(
        count(&text, "Remove 2 ended runs from the list?"),
        1,
        "{text:?}"
    );
    assert_eq!(count(&text, "Their terminals stay open."), 1, "{text:?}");
    app.apply_flow_act(panel::Act::Remove(dir.clone()));
    settle(&mut app);
    assert!(!dir.exists());
    assert!(received.try_recv().is_err(), "no terminal");
}

/// Waits for a file that a worker thread writes, and gives its text.
fn written(file: &Path) -> String {
    let started = Instant::now();
    loop {
        if let Ok(text) = std::fs::read_to_string(file) {
            return text;
        }
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{} was not written",
            file.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn panel_buttons_write_only_the_users_answers() {
    let project = project_with_flows();
    let (mut app, received, data) = app_with_project(project.path());
    let dir = data.path().join("flows").join("build-1");
    std::fs::create_dir_all(dir.join("questions")).expect("dir");
    let mut state = RunState::new(&flow::parse(FLOW).expect("flow"), "build", "p");
    state.set("one", StepStatus::Running);
    state.set("ok", StepStatus::Running);
    std::fs::write(
        dir.join("state.json"),
        serde_json::to_vec(&state).expect("json"),
    )
    .expect("state");
    let _runner = flow::control::hold_runner(&dir).expect("lock");

    // Approve: the gate's answer file.
    app.apply_flow_act(panel::Act::Gate(dir.clone(), "ok".into(), true));
    assert_eq!(written(&dir.join("answers/ok")), "approve");

    // Send: the typed answer to the step's open question.
    std::fs::write(dir.join("questions/one"), "A or B?").expect("question");
    app.flows
        .drafts
        .insert((dir.clone(), "one".into()), " Use A ".into());
    app.apply_flow_act(panel::Act::Answer(dir.clone(), "one".into()));
    assert_eq!(written(&dir.join("replies/one")), "Use A");
    assert!(app.flows.drafts.is_empty(), "the sent draft is cleared");

    // Cancel run: nothing is written before the confirmation.
    app.apply_flow_act(panel::Act::AskCancel(dir.clone()));
    assert!(!dir.join("cancel").exists());
    app.apply_flow_act(panel::Act::KeepRun);
    assert!(!dir.join("cancel").exists());
    app.apply_flow_act(panel::Act::AskCancel(dir.clone()));
    app.apply_flow_act(panel::Act::Cancel(dir.clone()));
    written(&dir.join("cancel"));

    // No button makes a terminal or types into one.
    assert!(received.try_recv().is_err());
    assert!(app.post_create.by_tab.is_empty());
}

/// A run of FLOW in folder `name`: running, or ended with `done` steps done.
fn run_of(data: &Path, name: &str, ended: Option<usize>) -> Run {
    let mut state = RunState::new(&flow::parse(FLOW).expect("flow"), "build", "p");
    if let Some(done) = ended {
        for id in ["one", "ok"].into_iter().take(done) {
            state.set(id, StepStatus::Done);
        }
        state.ended = true;
    }
    Run {
        dir: data.join("flows").join(name),
        state,
        gone: false,
    }
}

fn count(text: &[String], label: &str) -> usize {
    text.iter().filter(|line| *line == label).count()
}

#[test]
fn only_the_newest_and_running_runs_show_their_steps() {
    let project = project_with_flows();
    let (mut app, received, data) = app_with_project(project.path());
    app.flows.runs = vec![
        run_of(data.path(), "build-3", Some(2)),
        run_of(data.path(), "build-2", Some(0)),
        run_of(data.path(), "build-1", None),
    ];
    // No folder read replaces these runs during the test.
    app.flows.read_at = Some(Instant::now());
    let ctx = egui::Context::default();
    let text = painted(&ctx, |ui| app.flows_panel(ui));
    // The newest (done) and the running run show their steps; the older
    // stopped run is one line with its summary.
    assert_eq!(count(&text, "one"), 2, "{text:?}");
    for label in ["done", "stopped", "running", "0/2"] {
        assert!(text.iter().any(|line| line == label), "{label}: {text:?}");
    }
    // A click opens the stopped run, a second one closes it again.
    let stopped = data.path().join("flows/build-2");
    app.apply_flow_act(panel::Act::Toggle(stopped.clone(), true));
    let text = painted(&ctx, |ui| app.flows_panel(ui));
    assert_eq!(count(&text, "one"), 3, "{text:?}");
    app.apply_flow_act(panel::Act::Toggle(stopped, false));
    app.apply_flow_act(panel::Act::Toggle(data.path().join("flows/build-1"), false));
    let text = painted(&ctx, |ui| app.flows_panel(ui));
    assert_eq!(count(&text, "one"), 1, "{text:?}");
    // Opening and closing writes nothing and starts nothing.
    assert!(received.try_recv().is_err());
    assert!(!data.path().join("flows").exists());
    for icon in ["Trash", "ChevronRight", "ChevronDown", "EllipsisVertical"] {
        assert!(crate::icons::is_ui_icon(icon), "{icon}");
    }
}

#[test]
fn running_run_shows_behind_many_newer_ended_runs() {
    let project = project_with_flows();
    let (mut app, _received, data) = app_with_project(project.path());
    // A gate that waits in an older run: its state file did not change
    // while the user ran the flow again and again.
    let mut waiting = run_of(data.path(), "build-0", None);
    waiting.state.set("one", StepStatus::Done);
    waiting.state.set("ok", StepStatus::Running);
    let mut runs: Vec<Run> = (1..=12)
        .rev()
        .map(|n| run_of(data.path(), &format!("build-{n}"), Some(2)))
        .collect();
    runs.push(waiting);
    let shown = panel::shown_runs(&runs, Some("p"));
    assert_eq!(shown.len(), 11, "the 10 newest and the running one");
    assert!(
        shown.iter().any(|run| run.dir.ends_with("build-0")),
        "the run that waits for the user is hidden"
    );
    app.flows.runs = runs;
    app.flows.read_at = Some(Instant::now());
    let ctx = egui::Context::default();
    let text = painted(&ctx, |ui| app.flows_panel(ui));
    // Its gate is at the top, in Needs you, above the long list of runs.
    assert_eq!(count(&text, "Approve"), 1, "{text:?}");
    assert_eq!(count(&text, "11"), 1, "the Runs count: {text:?}");
}

#[test]
fn a_long_step_note_is_clipped_to_two_rows() {
    let ctx = egui::Context::default();
    let note = "The agent did not report. ".repeat(20);
    let mut shown = None;
    painted(&ctx, |ui| {
        shown = Some(runs_view::note_galley(ui, &note, 300.0));
    });
    let galley = shown.expect("galley");
    assert_eq!(galley.rows.len(), 2);
    assert!(galley.elided, "the full text is on hover");
    let short = painted(&ctx, |_| {});
    assert!(short.is_empty());
}

/// Saves `run` as its state file.
fn save(run: &Run) {
    std::fs::create_dir_all(&run.dir).expect("dir");
    std::fs::write(
        run.dir.join("state.json"),
        serde_json::to_vec(&run.state).expect("json"),
    )
    .expect("state");
}

/// Reads the panel's answers until no worker writes.
fn settle(app: &mut App) {
    let ctx = egui::Context::default();
    let started = Instant::now();
    while !app.flows.acting.is_empty() {
        app.poll_flow_runs(&ctx);
        assert!(started.elapsed() < Duration::from_secs(10), "worker hangs");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn remove_and_clear_delete_only_ended_runs_after_a_click() {
    let project = project_with_flows();
    let (mut app, received, data) = app_with_project(project.path());
    let running = run_of(data.path(), "build-1", None);
    let stopped = run_of(data.path(), "build-2", Some(1));
    let done = run_of(data.path(), "build-3", Some(2));
    let other = Run {
        state: RunState {
            project: "other".into(),
            ..done.state.clone()
        },
        dir: data.path().join("flows/build-4"),
        gone: false,
    };
    for run in [&running, &stopped, &done, &other] {
        save(run);
    }
    let _runner = flow::control::hold_runner(&running.dir).expect("lock");
    // Remove of a running run is refused and deletes nothing.
    app.apply_flow_act(panel::Act::Remove(running.dir.clone()));
    settle(&mut app);
    assert!(running.dir.join("state.json").is_file());
    assert!(
        app.error
            .text()
            .is_some_and(|text| text.contains("has not ended")),
        "{:?}",
        app.error.text()
    );
    app.apply_flow_act(panel::Act::Remove(done.dir.clone()));
    settle(&mut app);
    assert!(!done.dir.exists());

    // Clear ended runs: nothing goes before the confirmation.
    app.flows.runs = vec![running, stopped, other];
    let ctx = egui::Context::default();
    app.apply_flow_act(panel::Act::AskClear);
    let text = painted_frames(&ctx, |ui| app.flows_panel(ui));
    assert_eq!(
        count(&text, "Remove 1 ended run from the list?"),
        1,
        "{text:?}"
    );
    app.apply_flow_act(panel::Act::KeepRuns);
    assert!(!app.flows.confirm_clear);
    let stopped_dir = data.path().join("flows/build-2");
    assert!(stopped_dir.is_dir());
    app.apply_flow_act(panel::Act::Clear(vec![stopped_dir.clone()]));
    settle(&mut app);
    assert!(!stopped_dir.exists());
    assert!(data.path().join("flows/build-1/state.json").is_file());
    assert!(data.path().join("flows/build-4/state.json").is_file());
    assert!(received.try_recv().is_err(), "no terminal");
}

#[test]
fn needs_you_collects_the_gates_and_questions_of_every_running_run() {
    let project = project_with_flows();
    let (mut app, received, data) = app_with_project(project.path());
    let mut gate = run_of(data.path(), "build-3", None);
    gate.state.set("one", StepStatus::Done);
    gate.state.set("ok", StepStatus::Running);
    let mut asks = run_of(data.path(), "build-2", None);
    asks.state.set("one", StepStatus::Running);
    asks.state.steps.get_mut("one").expect("step").question = Some("A or B?".into());
    // An ended run waits for nothing, even with a question left in it.
    let mut ended = run_of(data.path(), "build-1", Some(1));
    ended.state.steps.get_mut("ok").expect("step").question = Some("Old?".into());
    let runs = [&gate, &asks, &ended];
    let needs = runs_view::needs_you(&runs);
    let found: Vec<(&str, &str)> = needs
        .iter()
        .map(|need| {
            (
                need.run
                    .dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(""),
                need.step,
            )
        })
        .collect();
    assert_eq!(found, [("build-3", "ok"), ("build-2", "one")]);

    app.flows.runs = vec![gate, asks, ended];
    app.flows.read_at = Some(Instant::now());
    let ctx = egui::Context::default();
    let text = painted(&ctx, |ui| app.flows_panel(ui));
    for label in ["NEEDS YOU", "Build › ok", "Good?", "Build › one", "A or B?"] {
        assert_eq!(count(&text, label), 1, "{label}: {text:?}");
    }
    // Each answer shows once: in Needs you, not again under the run.
    assert_eq!(count(&text, "Approve"), 1, "{text:?}");
    assert_eq!(count(&text, "Reject"), 1, "{text:?}");
    assert_eq!(count(&text, "Send"), 1, "{text:?}");
    assert_eq!(count(&text, "Old?"), 0, "{text:?}");
    // Painting answers nothing and starts nothing.
    assert!(!data.path().join("flows").exists());
    assert!(received.try_recv().is_err());
}

#[test]
fn step_style_colors_each_state_and_spins_only_while_running() {
    use status::{Tone, step_style};
    let step = |status, gate: Option<&str>, question: Option<&str>| StepRun {
        status,
        gate: gate.map(str::to_owned),
        question: question.map(str::to_owned),
        ..StepRun::default()
    };
    let style = |step: StepRun| {
        let style = step_style(&step);
        (style.icon, style.tone, style.spin)
    };
    assert_eq!(
        style(step(StepStatus::Done, None, None)),
        ("CircleCheck", Tone::Good, false)
    );
    assert_eq!(
        style(step(StepStatus::Running, None, None)),
        ("LoaderCircle", Tone::Good, true)
    );
    assert_eq!(
        style(step(StepStatus::Running, Some("ok?"), None)),
        ("ShieldQuestion", Tone::Waits, false)
    );
    assert_eq!(
        style(step(StepStatus::Running, None, Some("A?"))),
        ("MessageCircleQuestion", Tone::Waits, false)
    );
    assert_eq!(
        style(step(StepStatus::Failed, None, None)),
        ("CircleAlert", Tone::Failed, false)
    );
    assert_eq!(style(step(StepStatus::Cancelled, None, None)).1, Tone::Dim);
    assert_eq!(
        style(step(StepStatus::Waiting, Some("ok?"), None)).1,
        Tone::Dim
    );
    // Each tone has its own theme color.
    let theme = terminator_core::appearance::AppearanceConfig::default();
    let colors: std::collections::HashSet<_> = [Tone::Good, Tone::Waits, Tone::Failed, Tone::Dim]
        .into_iter()
        .map(|tone| tone.color(&theme))
        .collect();
    assert_eq!(colors.len(), 4);
    assert!(crate::icons::is_ui_icon("Workflow"));
    assert!(crate::icons::is_ui_icon("Square"));
}

#[test]
fn run_chip_and_time_show_the_state_of_a_run() {
    let data = Path::new("/data");
    let mut run = run_of(data, "build-1700000000", None);
    assert_eq!(status::run_chip(&run), ("running", status::Tone::Good));
    assert_eq!(runs_view::run_trailing(&run, 1_700_007_200), "0/2");
    run.state.set("ok", StepStatus::Running);
    assert_eq!(status::run_chip(&run).0, "waits");
    run.state.set("ok", StepStatus::Failed);
    run.state.ended = true;
    assert_eq!(status::run_chip(&run), ("failed", status::Tone::Failed));
    assert_eq!(runs_view::run_trailing(&run, 1_700_007_200), "2h ago");
    let done = run_of(data, "build-2", Some(2));
    assert_eq!(status::run_chip(&done).0, "done");
    assert_eq!(runs_view::run_trailing(&done, 1_700_007_200), "");
    assert_eq!(
        status::segments(&run),
        [status::Tone::Dim, status::Tone::Failed]
    );
}

#[test]
fn empty_project_offers_new_flow_and_says_where_flows_live() {
    let project = tempfile::tempdir().expect("project");
    let (mut app, received, _data) = app_with_project(project.path());
    app.flows.tool_flows = Some(load_flows("p".into(), project.path().into()));
    app.flows.tool_read_at = Some(Instant::now());
    app.flows.read_at = Some(Instant::now());
    let ctx = egui::Context::default();
    let text = painted(&ctx, |ui| app.flows_panel(ui));
    for label in [
        "No flows in this project",
        "New flow",
        "Flows are stored in .terminator/flows/.",
    ] {
        assert_eq!(count(&text, label), 1, "{label}: {text:?}");
    }
    app.apply_flow_act(panel::Act::NewFlow);
    assert!(app.flows.open, "New flow opens the Run flow window");
    assert!(received.try_recv().is_err());
    assert!(
        !project.path().join(FLOW_DIR).exists(),
        "nothing is written"
    );
}

#[test]
fn flows_section_lists_files_and_runs_only_after_trust() {
    let project = project_with_flows();
    let (mut app, received, _data) = app_with_project(project.path());
    app.flows.tool_flows = Some(load_flows("p".into(), project.path().into()));
    app.flows.tool_read_at = Some(Instant::now());
    app.flows.read_at = Some(Instant::now());
    let ctx = egui::Context::default();
    let text = painted(&ctx, |ui| app.flows_panel(ui));
    for label in ["RUNS", "FLOWS", "Build", "2 steps", "broken", "New flow…"] {
        assert!(text.iter().any(|line| line == label), "{label}: {text:?}");
    }
    assert!(
        text.iter()
            .any(|line| line.starts_with("Trust this project?")),
        "{text:?}"
    );
    // A broken file shows why, on its own line.
    let loaded = app.flows.tool_flows.clone().expect("flows");
    let error = loaded.files[1].flow.clone().expect_err("broken");
    let first = error.lines().next().expect("error line");
    assert!(text.iter().any(|line| line == first), "{first}: {text:?}");
    // Run before the trust answer types nothing.
    app.apply_flow_act(panel::Act::Run("build".into()));
    assert!(received.try_recv().is_err(), "no terminal before trust");
    assert!(app.post_create.by_tab.is_empty());
}
