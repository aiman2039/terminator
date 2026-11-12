use super::model::{self, parse_tasks, plan};
use super::*;
use std::sync::mpsc;

fn task(command: &str, source: TaskSource) -> TaskDef {
    TaskDef {
        name: "t".into(),
        command: command.into(),
        cwd: None,
        enter: true,
        source,
    }
}

fn words() -> Vec<String> {
    model::agent_words(&[])
}

#[test]
fn task_file_lists_named_commands_with_optional_folder_and_enter() {
    let text = r#"
        # comment
        [[task]]
        name = "Test"
        command = "cargo test"
        cwd = "crates/app"

        [[task]]
        name = "Serve"
        command = "npm run dev"
        enter = false

        [[task]]
        name = ""
        command = "skipped"
    "#;
    let tasks = parse_tasks(text, TaskSource::Project).expect("valid file");
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0].cwd.as_deref(), Some("crates/app"));
    assert!(tasks[0].enter);
    assert!(!tasks[1].enter);
}

#[test]
fn project_task_folder_cannot_leave_the_project() {
    let text = "[[task]]\nname = \"x\"\ncommand = \"ls\"\ncwd = \"../..\"\n";
    assert!(parse_tasks(text, TaskSource::Project).is_err());
    assert!(parse_tasks(text, TaskSource::User).is_ok());
    assert!(parse_tasks("not toml [", TaskSource::User).is_err());
}

#[test]
fn project_task_types_nothing_before_trust() {
    let task = task("cargo test", TaskSource::Project);
    assert_eq!(plan(&task, None, &words()), RunPlan::NeedsTrust);
    assert_eq!(plan(&task, Some(false), &words()), RunPlan::NotTrusted);
    assert!(matches!(
        plan(&task, Some(true), &words()),
        RunPlan::Type(post) if post.enter
    ));
    // The user's own tasks need no trust answer.
    let user = TaskDef {
        source: TaskSource::User,
        ..task
    };
    assert!(matches!(plan(&user, None, &words()), RunPlan::Type(_)));
}

#[test]
fn run_presses_enter_for_shell_commands_but_never_for_agents() {
    let enter = |command: &str| match plan(&task(command, TaskSource::User), None, &words()) {
        RunPlan::Type(post) => post.enter,
        other => panic!("unexpected {other:?}"),
    };
    assert!(enter("cargo test --workspace"));
    assert!(enter("npm run dev"));
    for agent in [
        "claude",
        "codex --full-auto",
        "cd app && /usr/local/bin/claude",
        "npx @openai/codex",
        "npx @anthropic-ai/claude-code",
        "node codex.js",
        "CLAUDE -p hi",
        "cursor-agent",
    ] {
        assert!(!enter(agent), "{agent} must be typed without Enter");
    }
    let quiet = TaskDef {
        enter: false,
        ..task("cargo test", TaskSource::User)
    };
    assert!(matches!(plan(&quiet, None, &words()), RunPlan::Type(post) if !post.enter));
}

#[test]
fn a_multi_line_command_typed_without_enter_runs_no_line() {
    for task in [
        task("echo hi\nclaude", TaskSource::User),
        TaskDef {
            enter: false,
            ..task("make\r\nmake test", TaskSource::User)
        },
    ] {
        let RunPlan::Type(post) = plan(&task, None, &words()) else {
            panic!("a user task types");
        };
        assert!(!post.enter);
        let bytes = egui_term::paste_input(&post.input, egui_term::TerminalMode::empty());
        assert!(
            !bytes.contains(&b'\r') && !bytes.contains(&b'\n'),
            "{:?}",
            post.input
        );
    }
}

#[test]
fn agent_presets_count_as_agent_programs() {
    let preset = crate::preferences::AgentPreset {
        name: "Mine".into(),
        kind: "custom".into(),
        program: "/opt/bin/my-agent".into(),
        args: vec![],
    };
    let words = model::agent_words(&[preset]);
    assert!(model::names_agent("my-agent --go", &words));
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
fn repository_task_file_runs_nothing_before_trust() {
    let project = tempfile::tempdir().expect("project");
    std::fs::create_dir_all(project.path().join(".terminator")).expect("dir");
    std::fs::write(
        project.path().join(model::PROJECT_TASK_FILE),
        "[[task]]\nname = \"Build\"\ncommand = \"make\"\n",
    )
    .expect("task file");
    let (mut app, received, _data) = app_with_project(project.path());
    let loaded = model::load_tasks("p".into(), project.path().into(), None);
    let build = loaded.tasks.first().expect("one task").clone();
    assert_eq!(build.source, TaskSource::Project);

    app.run_task("p", project.path(), &build);
    assert!(received.try_recv().is_err(), "no terminal before trust");
    assert!(app.post_create.by_tab.is_empty());
    app.rerun_last_task();
    assert!(
        received.try_recv().is_err(),
        "nothing ran, nothing to rerun"
    );

    app.answer_task_trust(project.path(), true);
    app.run_task("p", project.path(), &build);
    let Ok(Job::Control(_, After::Workspace(tab, _))) = received.try_recv() else {
        panic!("expected a new terminal");
    };
    assert_eq!(
        app.post_create.by_tab.get(&tab),
        Some(&PostCreate {
            input: "make".into(),
            enter: true,
        })
    );
    app.rerun_last_task();
    assert!(received.try_recv().is_ok(), "rerun opens another terminal");
}
