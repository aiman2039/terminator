use super::*;

fn shell(id: &str) -> Session {
    Session {
        remote: None,
        review: false,
        id: id.into(),
        project_id: "a".into(),
        label: "new".into(),
        cwd: "/a".into(),
        kind: SessionKind::Shell,
        file: None,
        lifecycle: Lifecycle::Running,
        created: 0,
        exit_code: None,
        rows: 24,
        cols: 80,
        generation: "fixture".into(),
        pid: None,
        truncated: false,
        cwd_confirmed: true,
    }
}

#[test]
fn post_create_input_waits_for_backend_attach() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, received) = mpsc::channel();
    app.jobs = jobs.into();
    app.selected = Some("a".into());
    let post = PostCreate {
        input: "claude".into(),
        enter: false,
    };
    app.create_workspace_tab_with(None, None, Some(post.clone()));
    let Job::Control(_, After::Workspace(id, _)) = received.try_recv().unwrap() else {
        panic!("Expected workspace create")
    };
    assert_eq!(app.post_create.by_tab.get(&id), Some(&post));
    app.update_tx
        .send(Update::WorkspaceCreated(
            shell("s1"),
            id.clone(),
            Vec::new(),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(app.post_create.by_tab.is_empty());
    // No backend is attached yet: the input stays queued for the session.
    app.flush_pending_input("s1");
    assert_eq!(app.post_create.by_session.get("s1"), Some(&post));
}

#[test]
fn empty_post_create_input_is_not_queued() {
    let (mut app, _, _dir) = fixture();
    let (jobs, _received) = mpsc::channel();
    app.jobs = jobs.into();
    app.selected = Some("a".into());
    app.create_workspace_tab_with(None, None, Some(PostCreate::default()));
    assert!(app.post_create.by_tab.is_empty());
}

/// Resume conversation opens a terminal in the ended session's folder and
/// runs the stored resume command there (an explicit click only).
#[test]
fn resume_button_runs_resume_in_session_cwd() {
    let (mut app, _, _dir) = fixture();
    let (jobs, received) = mpsc::channel();
    app.jobs = jobs.into();
    let mut ended = shell("old");
    ended.cwd = "/a/sub".into();
    ended.lifecycle = Lifecycle::Ended;
    app.state.agents.push(Agent {
        invocation_id: "agent-old".into(),
        session_id: "old".into(),
        kind: "codex".into(),
        provider_session_id: Some("p1".into()),
        state: AgentState::Stopped,
        sequence: None,
        updated: 0,
        resume: Some(Resume {
            program: "codex".into(),
            args: vec!["resume".into(), "p1".into()],
        }),
        process: None,
    });
    let command = app.resume_command("old").unwrap();
    assert_eq!(command, "'codex' 'resume' 'p1'");
    app.resume_conversation(&ended, command.clone());
    let Job::Control(request, After::Workspace(id, _)) = received.try_recv().unwrap() else {
        panic!("Expected workspace create")
    };
    let Request::Create { project, cwd, .. } = *request else {
        panic!("Expected create")
    };
    assert_eq!((project.as_str(), cwd), ("a", Some("/a/sub".into())));
    assert_eq!(
        app.post_create.by_tab.get(&id),
        Some(&PostCreate {
            input: command,
            enter: true,
        })
    );
}

/// Ask agent opens only when a live agent terminal can take the text; it
/// never starts an agent.
#[test]
fn ask_agent_without_a_live_agent_only_says_why() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, received) = mpsc::channel();
    app.jobs = jobs.into();
    app.run_shortcut(&ctx, Action::AskAgent);
    assert!(app.palette.ask.is_none());
    assert_eq!(app.info.as_deref(), Some("No open agent terminal to ask."));
    assert!(received.try_recv().is_err());
}

/// #268: input for a create that was refused or failed is not kept.
#[test]
fn post_create_input_is_dropped_when_the_create_fails() {
    let (mut app, ctx, _dir) = fixture();
    let post = PostCreate {
        input: "claude".into(),
        enter: false,
    };
    let (jobs, received) = mpsc::channel();
    drop(received);
    app.jobs = jobs.into();
    app.selected = Some("a".into());
    app.create_workspace_tab_with(Some(0), None, Some(post.clone()));
    assert!(app.post_create.by_tab.is_empty() && app.workspace_insert.is_empty());
    let (jobs, received) = mpsc::channel();
    app.jobs = jobs.into();
    app.create_workspace_tab_with(Some(0), None, Some(post));
    let Job::Control(_, After::Workspace(id, _)) = received.try_recv().unwrap() else {
        panic!("Expected workspace create")
    };
    app.update_tx
        .send(Update::CreateFailed(id, "daemon refused".into()))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(app.post_create.by_tab.is_empty() && app.workspace_insert.is_empty());
    assert!(
        app.error
            .text()
            .is_some_and(|e| e.contains("daemon refused"))
    );
}

fn installed(kind: &str) -> crate::agent_launch::InstalledAgent {
    crate::agent_launch::InstalledAgent {
        kind: kind.into(),
        program: kind.into(),
    }
}

/// A launcher opens a tab in the current project and types the agent
/// command there; it never presses Enter.
#[test]
fn agent_launcher_types_command_without_enter() {
    let (mut app, _, _dir) = fixture();
    let (jobs, received) = mpsc::channel();
    app.jobs = jobs.into();
    app.selected = Some("a".into());
    app.installed_agents = vec![installed("codex")];
    let launcher = app.agent_launchers().remove(0);
    app.launch_agent(&launcher);
    let Job::Control(request, After::Workspace(id, _)) = received.try_recv().unwrap() else {
        panic!("Expected workspace create")
    };
    let Request::Create { project, .. } = *request else {
        panic!("Expected create")
    };
    assert_eq!(project, "a");
    assert_eq!(
        app.post_create.by_tab.get(&id),
        Some(&PostCreate {
            input: "codex".into(),
            enter: false,
        })
    );
    assert_eq!(app.preferences.agent_launch_recent, ["agent:codex"]);
}

/// New agent session lists only the launchers, the last used first.
#[test]
fn new_agent_session_lists_launchers_last_used_first() {
    let (mut app, ctx, _dir) = fixture();
    app.selected = Some("a".into());
    app.installed_agents = vec![installed("codex"), installed("pi")];
    app.preferences.agent_launch_recent = vec!["agent:pi".into()];
    app.run_shortcut(&ctx, Action::NewAgentSession);
    assert!(app.palette.open);
    let titles: Vec<_> = app
        .filtered_palette()
        .iter()
        .map(|item| match item {
            crate::palette::PaletteItem::AgentLaunch(launcher) => launcher.title(),
            other => panic!("Unexpected row {other:?}"),
        })
        .collect();
    assert_eq!(titles, ["New terminal with Pi", "New terminal with Codex"]);
}

/// Without a preset or an installed agent the picker only says why.
#[test]
fn new_agent_session_without_agents_only_says_why() {
    let (mut app, ctx, _dir) = fixture();
    app.installed_agents.clear();
    app.run_shortcut(&ctx, Action::NewAgentSession);
    assert!(!app.palette.open);
    assert!(app.info.is_some());
}

/// #268: a refused editor close does not leave the editors busy, and a
/// refused Explorer search does not stay "Searching…".
#[test]
fn refused_close_and_search_leave_no_pending_flag() {
    let (mut app, _ctx, dir) = fixture();
    let (jobs, received) = mpsc::channel();
    drop(received);
    app.jobs = jobs.into();
    app.close_editors(
        editor_close::Target::Pane("e1".into()),
        vec!["e1".into()],
        editor_close::Mode::Save,
    );
    assert!(app.editor_close_sessions.is_empty());
    app.preferences.explorer_search_mode = crate::preferences::ExplorerSearchMode::Contents;
    app.explorer_query = "needle".into();
    app.refresh_explorer_search(dir.path());
    assert!(!app.explorer_search_pending);
}
