use crate::app::tests::support::{fixture, session_fixture};
use crate::workspace::Workspace;
use crate::{After, App, Job, Tab};
use egui_dock::DockState;
use std::sync::mpsc;
use terminator_core::{
    Agent, AgentState, Lifecycle, Request, Resume, SessionKind, State, agent_workflow,
};

fn interrupted(sid: &str) -> terminator_core::Session {
    let mut session = session_fixture(sid, SessionKind::Shell);
    session.lifecycle = Lifecycle::Interrupted;
    session.pid = None;
    session.cwd = format!("/a/{sid}").into();
    session
}

/// An app after a reboot: two interrupted shells side by side in one tab,
/// one of them an agent's terminal with a resume command.
fn rebooted(restore: bool) -> (App, tempfile::TempDir, mpsc::Receiver<Job>) {
    let (mut app, _ctx, dir) = fixture();
    let (jobs, received) = mpsc::channel();
    app.jobs = jobs.into();
    let mut state: State = app.state.clone();
    state.sessions = vec![interrupted("left"), interrupted("right")];
    state.agents = vec![Agent {
        invocation_id: "claude:1:p".into(),
        session_id: "right".into(),
        kind: "claude".into(),
        provider_session_id: Some("p".into()),
        state: AgentState::Unknown,
        sequence: None,
        updated: 1,
        resume: Some(Resume {
            program: "claude".into(),
            args: vec!["--resume".into(), "p".into()],
        }),
        process: None,
    }];
    if restore {
        state.capabilities = vec![agent_workflow::RESTORE_CAPABILITY.into()];
    }
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(DockState::new(vec![
            Tab::Terminal("left".into()),
            Tab::Terminal("right".into()),
        ])),
    );
    state.revision = state.revision.saturating_add(1);
    app.apply_state(state);
    (app, dir, received)
}

fn sent(received: &mpsc::Receiver<Job>) -> Vec<(Request, Option<String>)> {
    let mut out = Vec::new();
    while let Ok(job) = received.try_recv() {
        if let Job::Control(request, after) = job {
            let restore = match after {
                After::Restore(old) | After::Text(old) => Some(old),
                _ => None,
            };
            out.push((*request, restore));
        }
    }
    out
}

#[test]
fn restore_never_runs_without_the_click() {
    let (app, _dir, received) = rebooted(true);
    assert_eq!(app.restorable_sessions("a").len(), 2);
    assert!(
        sent(&received).iter().all(|(request, _)| !matches!(
            request,
            Request::CreateRestored { .. } | Request::Create { .. } | Request::Input { .. }
        )),
        "a snapshot alone must not open any shell"
    );
}

#[test]
fn restore_click_opens_shells_with_saved_output_and_launches_no_agent() {
    let (mut app, _dir, received) = rebooted(true);
    assert_eq!(app.restore_workspace("a"), 2);
    let reads = sent(&received);
    assert!(
        reads
            .iter()
            .all(|(request, _)| matches!(request, Request::History { .. }))
    );
    // What `Update::Text` does when each saved output arrives.
    app.texts
        .insert("restore:left".into(), "make test\nok\n".into());
    app.continue_restores();
    app.texts
        .insert("restore:right".into(), "agent output\n".into());
    app.continue_restores();
    let creates = sent(&received);
    assert_eq!(creates.len(), 2);
    for (request, old) in &creates {
        let Request::CreateRestored {
            project,
            cwd,
            label,
            seed,
        } = request
        else {
            panic!("only CreateRestored after the saved output: {request:?}");
        };
        let old = old.as_deref().unwrap();
        assert_eq!(project, "a");
        assert_eq!(cwd, &std::path::PathBuf::from(format!("/a/{old}")));
        assert_eq!(label, old);
        assert!(seed.contains("Saved output"));
        assert!(!seed.contains("--resume"), "no agent command is typed");
    }
    assert!(app.triage.restores.is_empty());
}

#[test]
fn restored_shell_takes_the_same_pane_position() {
    let (mut app, _dir, _received) = rebooted(true);
    let mut fresh = session_fixture("fresh", SessionKind::Shell);
    fresh.project_id = "a".into();
    app.handle_restored(fresh, "right");
    let tabs: Vec<Tab> = app.layouts["a"].tabs[0]
        .layout
        .iter_all_tabs()
        .map(|(_, tab)| tab.clone())
        .collect();
    assert_eq!(
        tabs,
        [Tab::Terminal("left".into()), Tab::Terminal("fresh".into())]
    );
    assert!(app.pending_layout_save);
}

#[test]
fn restore_with_an_older_daemon_sends_nothing() {
    let (mut app, _dir, received) = rebooted(false);
    assert_eq!(app.restore_workspace("a"), 0);
    assert!(sent(&received).is_empty());
    assert!(app.info.is_some());
}
