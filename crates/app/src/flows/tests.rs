use super::*;
use std::path::Path;
use std::sync::mpsc;
use terminator_core::flow::{StepRun, StepStatus};

const FLOW: &str = "version = 1\nname = \"Build\"\n[[step]]\nid = \"one\"\ncommand = \"claude\"\n[[step]]\nid = \"ok\"\ngate = \"Good?\"\nneeds = [\"one\"]\n";

fn project_with_flows() -> tempfile::TempDir {
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

fn app_with_project(root: &Path) -> (App, mpsc::Receiver<Job>, tempfile::TempDir) {
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
