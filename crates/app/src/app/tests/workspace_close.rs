use super::*;

#[test]
fn file_only_views_are_distinguished_from_shell_and_mixed_tabs() {
    let (mut app, _, _dir) = fixture();
    app.state.sessions.extend([
        session_fixture("file", SessionKind::Editor),
        session_fixture("shell", SessionKind::Shell),
    ]);
    assert!(app.editors_only(&["file".into()]));
    assert!(!app.editors_only(&["shell".into()]));
    assert!(!app.editors_only(&["file".into(), "shell".into()]));
    assert!(!app.editors_only(&[]));
}

#[test]
fn extra_close_while_prompting_does_not_queue_another_check() {
    let (mut app, _, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.upsert_unsaved_close(
        editor_close::Target::Pane("file".into()),
        vec!["file".into()],
        "Unsaved changes".into(),
    );
    assert!(app.skip_editor_close_request(&["file".into()]));
    assert!(!app.skip_editor_close_request(&["other".into()]));
    app.close_editors(
        editor_close::Target::Pane("file".into()),
        vec!["file".into()],
        editor_close::Mode::Discard,
    );
    assert!(matches!(
        requests.try_recv().unwrap(),
        Job::CloseEditors(_, _, editor_close::Mode::Discard, _)
    ));
    app.close_editors(
        editor_close::Target::Pane("file".into()),
        vec!["file".into()],
        editor_close::Mode::Discard,
    );
    assert!(requests.try_recv().is_err());
}

#[test]
fn failed_close_updates_the_same_prompt() {
    let (mut app, _, _dir) = fixture();
    app.upsert_unsaved_close(
        editor_close::Target::Pane("file".into()),
        vec!["file".into()],
        "Unsaved changes".into(),
    );
    app.editors_closed(
        editor_close::Target::Pane("file".into()),
        vec!["file".into()],
        Err("Editor did not close. Check for unsaved buffers or running editor jobs.".into()),
    );
    assert_eq!(app.editor_close_prompts.len(), 1);
    assert!(
        app.editor_close_prompts[0]
            .2
            .contains("Editor did not close")
    );
    app.editors_closed(
        editor_close::Target::Pane("other".into()),
        vec!["other".into()],
        Err("Unsaved changes".into()),
    );
    assert_eq!(app.editor_close_prompts.len(), 2);
    assert!(app.unsaved_close_prompt("file").is_some());
    assert!(app.unsaved_close_prompt("other").is_some());
}

#[test]
fn idle_close_deduplicates_and_preserves_new_tab_contents() {
    let (mut app, _, _dir) = fixture();
    let (jobs, received) = mpsc::channel();
    app.jobs = jobs.into();
    app.state
        .sessions
        .push(session_fixture("shell", SessionKind::Shell));
    app.state
        .capabilities
        .push(terminator_core::idle_close::CAPABILITY.into());
    app.insert("a", Tab::Terminal("shell".into()), None);
    let tab = app.layouts["a"].active.clone();
    let target = editor_close::Target::Workspace("a".into(), tab.clone());
    assert!(app.check_idle_close(target.clone(), vec!["shell".into()]));
    assert!(app.check_idle_close(target.clone(), vec!["shell".into()]));
    assert!(matches!(received.try_recv().unwrap(), Job::CloseIdle(..)));
    assert!(received.try_recv().is_err());
    app.insert("a", Tab::Terminal("new-shell".into()), Some("right"));
    app.idle_closed(
        target,
        vec!["shell".into()],
        Ok(vec![terminator_core::idle_close::Outcome {
            session: "shell".into(),
            status: terminator_core::idle_close::Status::Closed,
            reason: "Exited".into(),
        }]),
    );
    assert!(app.layouts["a"].tabs.iter().any(|t| t.id == tab));
    assert!(app.layouts["a"].contains(&Tab::Terminal("new-shell".into())));
}

#[test]
fn idle_close_ignores_state_pruning_of_the_closing_shell() {
    let (mut app, _, _dir) = fixture();
    let (jobs, received) = mpsc::channel();
    app.jobs = jobs.into();
    app.state
        .sessions
        .push(session_fixture("shell", SessionKind::Shell));
    app.state
        .capabilities
        .push(terminator_core::idle_close::CAPABILITY.into());
    app.insert("a", Tab::Terminal("shell".into()), None);
    let tab = app.layouts["a"].active.clone();
    let target = editor_close::Target::Workspace("a".into(), tab.clone());
    assert!(app.check_idle_close(target.clone(), vec!["shell".into()]));
    assert!(matches!(received.try_recv().unwrap(), Job::CloseIdle(..)));
    // The daemon's state broadcast prunes the dead shell's pane before
    // the close completion arrives (the bulk-close race).
    app.layouts.get_mut("a").unwrap().remove_session("shell");
    app.idle_closed(
        target,
        vec!["shell".into()],
        Ok(vec![terminator_core::idle_close::Outcome {
            session: "shell".into(),
            status: terminator_core::idle_close::Status::Closed,
            reason: "Exited".into(),
        }]),
    );
    assert!(app.error.is_none(), "no spurious changed-while-closing");
    assert!(app.idle_close_fallback.is_none());
    // The closed tab is gone; closing the last one leaves the usual
    // fresh empty Workspace tab (rebuilt with the same id).
    assert!(!app.layouts["a"].contains(&Tab::Terminal("shell".into())));
    assert!(
        app.layouts["a"]
            .tabs
            .iter()
            .all(|t| t.layout.iter_all_tabs().next().is_none())
    );
}

#[test]
fn older_daemons_and_mixed_editor_tabs_keep_confirmation() {
    let (mut app, _, _dir) = fixture();
    let (jobs, received) = mpsc::channel();
    app.jobs = jobs.into();
    app.state
        .sessions
        .push(session_fixture("shell", SessionKind::Shell));
    app.state
        .sessions
        .push(session_fixture("editor", SessionKind::Editor));
    let target = editor_close::Target::Pane("shell".into());
    assert!(!app.check_idle_close(target.clone(), vec!["shell".into()]));
    app.state
        .capabilities
        .push(terminator_core::idle_close::CAPABILITY.into());
    assert!(!app.check_idle_close(target, vec!["shell".into(), "editor".into()]));
    assert!(received.try_recv().is_err());
}

#[test]
fn idle_close_busy_shell_asks_without_error() {
    let (mut app, _, _dir) = fixture();
    let (jobs, received) = mpsc::channel();
    app.jobs = jobs.into();
    app.state
        .sessions
        .push(session_fixture("shell", SessionKind::Shell));
    app.state
        .capabilities
        .push(terminator_core::idle_close::CAPABILITY.into());
    app.insert("a", Tab::Terminal("shell".into()), None);
    let tab = app.layouts["a"].active.clone();
    let target = editor_close::Target::Workspace("a".into(), tab.clone());
    app.close_workspace = Some(("a".into(), tab.clone()));
    assert!(app.check_idle_close(target.clone(), vec!["shell".into()]));
    assert!(matches!(received.try_recv().unwrap(), Job::CloseIdle(..)));
    app.idle_closed(
        target.clone(),
        vec!["shell".into()],
        Ok(vec![terminator_core::idle_close::Outcome {
            session: "shell".into(),
            status: terminator_core::idle_close::Status::Busy,
            reason: "A foreground command owns the terminal".into(),
        }]),
    );
    assert_eq!(
        app.close_workspace.as_ref(),
        Some(&(String::from("a"), tab))
    );
    assert_eq!(app.idle_close_fallback.as_ref(), Some(&target));
    assert!(app.error.is_none());
    assert!(app.layouts["a"].contains(&Tab::Terminal("shell".into())));
}

#[test]
fn idle_close_failed_signal_reports_error() {
    let (mut app, _, _dir) = fixture();
    let (jobs, received) = mpsc::channel();
    app.jobs = jobs.into();
    app.state
        .sessions
        .push(session_fixture("shell", SessionKind::Shell));
    app.state
        .capabilities
        .push(terminator_core::idle_close::CAPABILITY.into());
    app.insert("a", Tab::Terminal("shell".into()), None);
    let target = editor_close::Target::Pane("shell".into());
    assert!(app.check_idle_close(target.clone(), vec!["shell".into()]));
    assert!(matches!(received.try_recv().unwrap(), Job::CloseIdle(..)));
    app.idle_closed(
        target.clone(),
        vec!["shell".into()],
        Ok(vec![terminator_core::idle_close::Outcome {
            session: "shell".into(),
            status: terminator_core::idle_close::Status::Failed,
            reason: "Signal delivered, but exit was not confirmed; view preserved".into(),
        }]),
    );
    assert_eq!(app.idle_close_fallback.as_ref(), Some(&target));
    assert!(app.error.as_deref().is_some_and(|e| e.contains("Failed")));
    assert!(app.layouts["a"].contains(&Tab::Terminal("shell".into())));
}

#[test]
fn close_tabs_to_the_left_queues_then_cancel_keeps_the_rest() {
    let (mut app, _, _dir) = fixture();
    let workspace = app.layouts.get_mut("a").unwrap();
    workspace.add("t0".into(), Tab::Terminal("s0".into()));
    workspace.add("t1".into(), Tab::Terminal("s1".into()));
    workspace.add("t2".into(), Tab::Terminal("s2".into()));
    app.begin_workspace_close_tabs("a", vec!["t0".into(), "t1".into()]);
    assert_eq!(app.close_workspace, Some(("a".into(), "t0".into())));
    assert_eq!(app.close_workspace_queue, ["t1"]);
    app.close_workspace_tab_now("a", "t0");
    assert_eq!(app.close_workspace, Some(("a".into(), "t1".into())));
    assert!(!app.layouts["a"].tabs.iter().any(|tab| tab.id == "t0"));
    assert!(app.layouts["a"].tabs.iter().any(|tab| tab.id == "t2"));
    app.abort_workspace_close();
    assert!(app.close_workspace.is_none());
    assert!(app.close_workspace_queue.is_empty());
    assert!(app.layouts["a"].tabs.iter().any(|tab| tab.id == "t1"));
}

#[test]
fn close_all_tabs_queues_every_tab() {
    let (mut app, _, _dir) = fixture();
    let workspace = app.layouts.get_mut("a").unwrap();
    workspace.add("t0".into(), Tab::Terminal("s0".into()));
    workspace.add("t1".into(), Tab::Terminal("s1".into()));
    workspace.add("t2".into(), Tab::Terminal("s2".into()));
    let ids = app.layouts["a"].ids();
    app.begin_workspace_close_tabs("a", ids);
    assert_eq!(app.close_workspace, Some(("a".into(), "t0".into())));
    assert_eq!(app.close_workspace_queue, ["t1", "t2"]);
}

#[test]
fn close_all_empty_tabs_leaves_an_empty_workspace() {
    let (mut app, ctx, _dir) = fixture();
    let workspace = app.layouts.get_mut("a").unwrap();
    workspace.add(
        "t0".into(),
        Tab::Image {
            path: "/a.png".into(),
        },
    );
    workspace.add(
        "t1".into(),
        Tab::Image {
            path: "/b.png".into(),
        },
    );
    workspace.add(
        "t2".into(),
        Tab::Image {
            path: "/c.png".into(),
        },
    );
    let ids = app.layouts["a"].ids();
    app.begin_workspace_close_tabs("a", ids);
    app.poll_workspace_close(&ctx);
    assert!(app.close_workspace.is_none());
    assert!(app.close_workspace_queue.is_empty());
    assert_eq!(app.layouts["a"].tabs.len(), 1);
    assert_eq!(app.layouts["a"].iter_all_tabs().count(), 0);
}

#[test]
fn close_all_live_tabs_wait_then_keep_running_advances() {
    let (mut app, ctx, _dir) = fixture();
    let workspace = app.layouts.get_mut("a").unwrap();
    workspace.add("t0".into(), Tab::Terminal("s0".into()));
    workspace.add("t1".into(), Tab::Terminal("s1".into()));
    workspace.add("t2".into(), Tab::Terminal("s2".into()));
    app.state.sessions.extend([
        session_fixture("s0", SessionKind::Shell),
        session_fixture("s1", SessionKind::Shell),
        session_fixture("s2", SessionKind::Shell),
    ]);
    let ids = app.layouts["a"].ids();
    app.begin_workspace_close_tabs("a", ids);
    poll_workspace_close_in_frame(&mut app, &ctx);
    assert_eq!(app.close_workspace, Some(("a".into(), "t0".into())));
    assert_eq!(app.close_workspace_queue, ["t1", "t2"]);
    assert_eq!(app.layouts["a"].ids(), ["t0", "t1", "t2"]);
    app.close_workspace_tab_now("a", "t0");
    assert_eq!(app.close_workspace, Some(("a".into(), "t1".into())));
    assert_eq!(app.close_workspace_queue, ["t2"]);
    assert_eq!(app.layouts["a"].ids(), ["t1", "t2"]);
}

#[test]
fn close_all_drains_empty_tabs_until_a_live_session() {
    let (mut app, ctx, _dir) = fixture();
    let workspace = app.layouts.get_mut("a").unwrap();
    workspace.add(
        "t0".into(),
        Tab::Image {
            path: "/a.png".into(),
        },
    );
    workspace.add("t1".into(), Tab::Terminal("s1".into()));
    workspace.add(
        "t2".into(),
        Tab::Image {
            path: "/c.png".into(),
        },
    );
    app.state
        .sessions
        .push(session_fixture("s1", SessionKind::Shell));
    let ids = app.layouts["a"].ids();
    app.begin_workspace_close_tabs("a", ids);
    poll_workspace_close_in_frame(&mut app, &ctx);
    assert_eq!(app.close_workspace, Some(("a".into(), "t1".into())));
    assert_eq!(app.close_workspace_queue, ["t2"]);
    assert_eq!(app.layouts["a"].ids(), ["t1", "t2"]);
}

#[test]
fn close_all_ended_sessions_leave_an_empty_workspace() {
    let (mut app, ctx, _dir) = fixture();
    let workspace = app.layouts.get_mut("a").unwrap();
    workspace.add("t0".into(), Tab::Terminal("s0".into()));
    workspace.add("t1".into(), Tab::Terminal("s1".into()));
    app.state.sessions.extend(["s0", "s1"].map(|id| {
        let mut session = session_fixture(id, SessionKind::Shell);
        session.lifecycle = Lifecycle::Ended;
        session
    }));
    let ids = app.layouts["a"].ids();
    app.begin_workspace_close_tabs("a", ids);
    app.poll_workspace_close(&ctx);
    assert!(app.close_workspace.is_none());
    assert!(app.close_workspace_queue.is_empty());
    assert_eq!(app.layouts["a"].tabs.len(), 1);
    assert_eq!(app.layouts["a"].iter_all_tabs().count(), 0);
}

#[test]
fn empty_workspace_tabs_drain_without_prompt() {
    let (mut app, ctx, _dir) = fixture();
    let workspace = app.layouts.get_mut("a").unwrap();
    workspace.add(
        "t0".into(),
        Tab::Image {
            path: "/a.png".into(),
        },
    );
    workspace.add(
        "t1".into(),
        Tab::Image {
            path: "/b.png".into(),
        },
    );
    workspace.add(
        "t2".into(),
        Tab::Image {
            path: "/c.png".into(),
        },
    );
    app.begin_workspace_close_tabs("a", vec!["t0".into(), "t1".into()]);
    app.poll_workspace_close(&ctx);
    assert!(app.close_workspace.is_none());
    assert!(app.close_workspace_queue.is_empty());
    assert_eq!(
        app.layouts["a"]
            .tabs
            .iter()
            .map(|tab| tab.id.as_str())
            .collect::<Vec<_>>(),
        ["t2"]
    );
}

#[test]
fn unsaved_close_cancel_aborts_remaining_workspace_tabs() {
    let (mut app, _, _dir) = fixture();
    app.close_workspace_queue = vec!["t1".into()];
    app.apply_unsaved_close_choice(
        appearance::UnsavedCloseChoice::Cancel,
        editor_close::Target::Workspace("a".into(), "t0".into()),
        vec!["editor".into()],
    );
    assert!(app.close_workspace_queue.is_empty());
}
