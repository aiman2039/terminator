use super::model::{self, Gate};
use super::run::{Step, plan};
use super::*;
use crate::workspace::{Workspace, WorkspaceTab};
use egui_dock::DockState;
use std::path::Path;
use std::sync::mpsc;

const FILE: &str = r#"
[[tab]]
name = "Dev"
[[tab.pane]]
cwd = "web"
command = "npm run dev"
[[tab.pane]]
split = "down"
command = "claude --continue"
[[tab.pane]]
command = "cargo watch"
enter = false
"#;

fn words() -> Vec<String> {
    crate::tasks::agent_words(&[])
}

fn creates(received: &mpsc::Receiver<Job>) -> Vec<(Request, After)> {
    let mut out = Vec::new();
    while let Ok(job) = received.try_recv() {
        if let Job::Control(request, after) = job
            && matches!(*request, Request::Create { .. })
        {
            out.push((*request, after));
        }
    }
    out
}

fn app() -> (App, tempfile::TempDir, mpsc::Receiver<Job>) {
    let (mut app, _ctx, dir) = crate::app::fixture();
    let (jobs, received) = mpsc::channel();
    app.jobs = jobs.into();
    (app, dir, received)
}

#[test]
fn launch_file_parses_tabs_splits_folders_and_commands() {
    let file = model::parse(FILE).unwrap();
    let [tab] = file.tabs.as_slice() else {
        panic!("one tab");
    };
    assert_eq!(tab.name.as_deref(), Some("Dev"));
    assert_eq!(tab.panes.len(), 3);
    assert_eq!(tab.panes[0].cwd.as_deref(), Some("web"));
    assert_eq!(tab.panes[1].split, Some(Split::Down));
    assert!(model::parse("[[tab]]\n[[tab.pane]]\ncwd = \"../out\"\n").is_err());
    assert!(model::parse("[[tab]]\nname = \"x\"\n").is_err(), "no panes");
    let again = model::parse(&model::render(&file).unwrap()).unwrap();
    assert_eq!(again, file);
}

#[test]
fn launch_never_presses_enter_for_an_agent_command() {
    let file = model::parse(FILE).unwrap();
    let steps = plan(&file, Path::new("/a"), &words());
    let posts: Vec<_> = steps
        .iter()
        .map(|step| match step {
            Step::Tab { post, .. } | Step::Split { post, .. } => post.clone().unwrap(),
        })
        .collect();
    assert_eq!(posts[0].input, "npm run dev");
    assert!(posts[0].enter, "a shell command follows the task rule");
    assert_eq!(posts[1].input, "claude --continue");
    assert!(!posts[1].enter, "an agent command is typed, never sent");
    assert!(!posts[2].enter, "enter = false only types");
    assert!(matches!(&steps[0], Step::Tab { cwd: Some(cwd), .. } if cwd == Path::new("/a/web")));
    assert!(matches!(
        &steps[1],
        Step::Split {
            split: Split::Down,
            ..
        }
    ));
}

#[test]
fn launch_file_from_the_repository_opens_nothing_before_trust() {
    assert_eq!(model::gate(true, None), Gate::NeedsTrust);
    assert_eq!(model::gate(true, Some(false)), Gate::NotTrusted);
    let (mut app, _dir, received) = app();
    let file = model::parse(FILE).unwrap();
    app.start_launch("a", Path::new("/a"), &file);
    assert!(creates(&received).is_empty(), "no terminal before trust");
    assert!(app.launch.run.is_none());
    app.lsp.set_trust(Path::new("/a"), false);
    app.start_launch("a", Path::new("/a"), &file);
    assert!(
        creates(&received).is_empty(),
        "no terminal when not trusted"
    );
    app.lsp.set_trust(Path::new("/a"), true);
    app.start_launch("a", Path::new("/a"), &file);
    let sent = creates(&received);
    let [(Request::Create { project, cwd, .. }, After::Workspace(..))] = sent.as_slice() else {
        panic!("one new tab after trust: {}", sent.len());
    };
    assert_eq!(project, "a");
    assert_eq!(cwd.as_deref(), Some(Path::new("/a/web")));
    assert!(
        app.post_create.by_session.is_empty(),
        "nothing is typed before the terminal exists"
    );
}

#[test]
fn launch_splits_the_previous_pane_once_it_arrives_and_types_its_command() {
    let (mut app, _dir, received) = app();
    app.lsp.set_trust(Path::new("/a"), true);
    app.start_launch("a", Path::new("/a"), &model::parse(FILE).unwrap());
    let sent = creates(&received);
    let [(_, After::Workspace(tab, _))] = sent.as_slice() else {
        panic!("first pane");
    };
    app.advance_launch();
    assert!(creates(&received).is_empty(), "waits for the first pane");
    // What `Update::WorkspaceCreated` does with the daemon's reply.
    let mut workspace = Workspace::from_layout(DockState::new(Vec::new()));
    workspace.tabs = vec![WorkspaceTab {
        id: tab.clone(),
        name: None,
        pinned: false,
        primary: None,
        layout: DockState::new(vec![Tab::Terminal("p1".into())]),
    }];
    app.layouts.insert("a".into(), workspace);
    app.advance_launch();
    let post = &app.post_create.by_session["p1"];
    assert_eq!((post.input.as_str(), post.enter), ("npm run dev", true));
    assert_eq!(app.layouts["a"].tabs[0].name.as_deref(), Some("Dev"));
    let sent = creates(&received);
    let [(_, After::CreateAt(origin, anchors, split))] = sent.as_slice() else {
        panic!("a split next");
    };
    assert_eq!(origin.as_deref(), Some(tab.as_str()));
    assert_eq!(anchors, &[Tab::Terminal("p1".into())]);
    assert_eq!(split.as_deref(), Some("down"));
}

#[test]
fn saved_layout_keeps_splits_and_folders_without_commands() {
    let (mut app, _dir, _received) = app();
    let mut state = app.state.clone();
    let mut left =
        crate::app::tests::support::session_fixture("l", terminator_core::SessionKind::Shell);
    left.cwd = "/a".into();
    let mut right =
        crate::app::tests::support::session_fixture("r", terminator_core::SessionKind::Shell);
    right.cwd = "/a/server".into();
    state.sessions = vec![left, right];
    state.revision = state.revision.saturating_add(1);
    app.apply_state(state);
    let mut dock = DockState::new(vec![Tab::Terminal("l".into())]);
    dock.main_surface_mut().split_right(
        egui_dock::NodeIndex::root(),
        0.5,
        vec![Tab::Terminal("r".into())],
    );
    app.layouts.insert("a".into(), Workspace::from_layout(dock));
    let file = app.launch_file_for("a", Path::new("/a"));
    let [tab] = file.tabs.as_slice() else {
        panic!("one tab");
    };
    assert_eq!(tab.panes[0].cwd, None);
    assert_eq!(tab.panes[1].split, Some(Split::Right));
    assert_eq!(tab.panes[1].cwd.as_deref(), Some("server"));
    assert!(tab.panes.iter().all(|pane| pane.command.is_none()));
}
