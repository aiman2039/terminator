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
        "Flows",
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
    let mut state = RunState::new(&flow::parse(FLOW).expect("flow"), "p");
    state.set("one", StepStatus::Running);
    state.steps.get_mut("one").expect("step").question = Some("A or B?".into());
    state.set("ok", StepStatus::Running);
    app.flows.runs = vec![Run {
        dir: data.path().join("flows").join("build-1"),
        state,
    }];
    let ctx = egui::Context::default();
    let text = painted(&ctx, |ui| app.flows_panel(ui));
    for label in [
        "Flows", "Build", "running", "one", "A or B?", "Send", "ok", "Good?", "Approve", "Reject",
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
    let icon = |step: &StepRun| panel::step_icon(step).0;
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
    let mut state = RunState::new(&flow::parse(FLOW).expect("flow"), "p");
    assert_eq!(panel::run_status(&state), "running");
    state.set("one", StepStatus::Done);
    state.ended = true;
    assert_eq!(panel::run_status(&state), "stopped");
    state.set("ok", StepStatus::Done);
    assert_eq!(panel::run_status(&state), "done");
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
    let mut state = RunState::new(&flow::parse(FLOW).expect("flow"), "p");
    state.set("one", StepStatus::Running);
    state.set("ok", StepStatus::Running);
    std::fs::write(
        dir.join("state.json"),
        serde_json::to_vec(&state).expect("json"),
    )
    .expect("state");

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
