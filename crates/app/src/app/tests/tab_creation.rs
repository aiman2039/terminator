use super::*;
use crate::app::TerminalActionInput;

#[test]
fn worktree_completion_opens_only_the_requested_projects_terminal() {
    for open_terminal in [false, true] {
        let (mut app, ctx, _dir) = fixture();
        let (jobs, requests) = mpsc::channel();
        app.jobs = jobs.into();
        let (updates, rx) = mpsc::channel();
        app.updates = rx;
        app.update_tx = updates.clone();
        updates
            .send(Update::WorktreeCreated(
                Box::new(app.state.clone()),
                "b".into(),
                open_terminal,
            ))
            .unwrap();
        drain_updates(&mut app, &ctx);
        assert_eq!(app.selected.as_deref(), Some("b"));
        let creates: Vec<_> = requests
            .try_iter()
            .filter_map(|job| match job {
                Job::Control(request, _) => match *request {
                    Request::Create { project, .. } => Some(project),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert_eq!(
            creates,
            if open_terminal {
                vec!["b".to_string()]
            } else {
                vec![]
            }
        );
    }
}

#[test]
fn new_workspace_tabs_use_the_project_root_after_cd() {
    let (mut app, _, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    let mut shell = session_fixture("shell", SessionKind::Shell);
    shell.cwd = "/a/worktree/subdir".into();
    app.state.sessions.push(shell);
    app.active_session = Some("shell".into());
    app.terminal_context.insert("a".into(), "shell".into());
    for active in [Some("shell".to_owned()), None] {
        app.active_session = active;
        app.create_workspace_tab(None);
        let Job::Control(request, _) = requests.try_recv().unwrap() else {
            panic!("Expected session creation");
        };
        assert!(matches!(*request, Request::Create { cwd: None, .. }));
    }
}

#[test]
fn ended_shells_do_not_supply_a_remembered_working_directory() {
    let (mut app, _, _dir) = fixture();
    let mut shell = session_fixture("shell", SessionKind::Shell);
    shell.cwd = "/deleted-worktree".into();
    shell.lifecycle = Lifecycle::Ended;
    app.state.sessions.push(shell);
    app.active_session = Some("shell".into());
    app.terminal_context.insert("a".into(), "shell".into());
    assert_eq!(app.cwd(), Some("/a".into()));
}

#[test]
fn removed_worktrees_cannot_remain_selected_or_supply_folder_memory() {
    let (mut app, _, _dir) = fixture();
    app.terminal_context.insert("a".into(), "old".into());
    let mut state = app.state.clone();
    state.worktrees.push(worktrees::Registration {
        project_id: "a".into(),
        common_dir: "/repo/.git".into(),
        path: "/a".into(),
        created: 0,
        removed: true,
    });
    app.apply_state(state);
    assert_eq!(app.selected.as_deref(), Some("b"));
    assert!(!app.visible_projects().iter().any(|p| p.id == "a"));
    assert!(!app.terminal_context.contains_key("a"));
    app.select_project("a".into());
    assert_eq!(app.selected.as_deref(), Some("b"));
}

#[test]
fn externally_deleted_folders_stop_new_tabs_and_restored_folders_reappear() {
    let (mut app, ctx, dir) = fixture();
    let root = dir.path().join("worktree");
    std::fs::create_dir(&root).unwrap();
    app.state
        .projects
        .iter_mut()
        .find(|p| p.id == "a")
        .unwrap()
        .path = root.clone();
    let directories = || crate::gui_services::project_directories(vec![("a".into(), root.clone())]);
    assert!(app.visible_projects().iter().any(|p| p.id == "a"));
    std::fs::remove_dir(&root).unwrap();
    app.update_tx
        .send(Update::ProjectDirectories(directories()))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(app.selected.as_deref(), Some("b"));
    assert!(!app.visible_projects().iter().any(|p| p.id == "a"));
    app.apply_state(app.state.clone());
    assert_eq!(app.selected.as_deref(), Some("b"));
    std::fs::create_dir(&root).unwrap();
    app.update_tx
        .send(Update::OpenedProject(
            Box::new(app.state.clone()),
            "a".into(),
            app.selection_generation,
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(app.selected.as_deref(), Some("a"));
    app.apply_project_directories(directories());
    assert!(app.visible_projects().iter().any(|p| p.id == "a"));
}

#[test]
fn missing_project_folders_keep_live_sessions_accessible_but_block_new_tabs() {
    let (mut app, _, _dir) = fixture();
    app.state
        .sessions
        .push(session_fixture("shell", SessionKind::Shell));
    app.active_session = Some("shell".into());
    app.apply_project_directories(vec![("a".into(), "/a".into(), false)]);
    assert_eq!(app.selected.as_deref(), Some("a"));
    assert_eq!(app.active_session.as_deref(), Some("shell"));
    assert!(app.visible_projects().iter().any(|p| p.id == "a"));
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.create_workspace_tab(None);
    app.create_strip();
    app.create(Some("right"));
    assert!(requests.try_recv().is_err());
}

#[test]
fn new_strip_tabs_use_the_project_root_but_splits_keep_the_shell_directory() {
    let (mut app, _, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    let mut shell = session_fixture("shell", SessionKind::Shell);
    shell.cwd = "/a/subdir".into();
    app.state.sessions.push(shell);
    app.active_session = Some("shell".into());
    app.create_strip();
    let Job::Control(request, after) = requests.try_recv().unwrap() else {
        panic!("Expected new strip tab");
    };
    assert!(matches!(*request, Request::Create { ref project, cwd: None, .. } if project == "a"));
    assert!(matches!(after, After::Strip));
    app.create_strip_split(Some("right"));
    let Job::Control(request, _) = requests.try_recv().unwrap() else {
        panic!("Expected strip split");
    };
    assert!(
        matches!(*request, Request::Create { cwd: Some(ref cwd), .. } if cwd == Path::new("/a/subdir"))
    );
    app.create(Some("right"));
    let Job::Control(request, _) = requests.try_recv().unwrap() else {
        panic!("Expected workspace split");
    };
    assert!(
        matches!(*request, Request::Create { cwd: Some(ref cwd), .. } if cwd == Path::new("/a/subdir"))
    );
}

#[test]
fn remembered_editors_and_other_projects_cannot_supply_a_shell_directory() {
    for (kind, project) in [(SessionKind::Editor, "a"), (SessionKind::Shell, "b")] {
        let (mut app, _, _dir) = fixture();
        let mut session = session_fixture("other", kind);
        session.project_id = project.into();
        session.cwd = "/other".into();
        app.state.sessions.push(session);
        app.active_session = Some("other".into());
        app.terminal_context.insert("a".into(), "other".into());
        assert_eq!(app.cwd(), Some("/a".into()));
    }
}

#[test]
fn stale_directory_results_do_not_hide_a_project_opened_at_another_path() {
    let (mut app, _, _dir) = fixture();
    app.state
        .projects
        .iter_mut()
        .find(|p| p.id == "a")
        .unwrap()
        .path = "/new-location".into();
    app.apply_project_directories(vec![
        ("a".into(), "/a".into(), false),
        ("unknown".into(), "/gone".into(), false),
    ]);
    assert_eq!(app.selected.as_deref(), Some("a"));
    assert!(app.missing_projects.is_empty());
    assert!(app.visible_projects().iter().any(|p| p.id == "a"));
}

#[test]
fn delayed_split_stays_in_origin_tab_without_stealing_tab_selection() {
    let (mut app, ctx, _dir) = fixture();
    app.insert("a", Tab::Terminal("one".into()), None);
    let first = app.layouts["a"].active.clone();
    app.layouts
        .get_mut("a")
        .unwrap()
        .add("second".into(), Tab::Terminal("two".into()));
    app.active_session = Some("two".into());
    app.update_tx
        .send(Update::Created(
            session_fixture("split", SessionKind::Shell),
            Some("right".into()),
            Some(vec![Tab::Terminal("one".into())]),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(app.layouts["a"].active, "second");
    assert_eq!(app.active_session.as_deref(), Some("two"));
    assert_eq!(
        app.layouts["a"]
            .tabs
            .iter()
            .find(|tab| tab.id == first)
            .unwrap()
            .layout
            .iter_all_tabs()
            .count(),
        2
    );
}

#[test]
fn workspace_created_inserts_at_requested_index() {
    let (mut app, ctx, _dir) = fixture();
    let workspace = app.layouts.get_mut("a").unwrap();
    workspace.add("t0".into(), Tab::Terminal("s0".into()));
    workspace.add("t1".into(), Tab::Terminal("s1".into()));
    app.workspace_insert.insert("mid".into(), 1);
    app.update_tx
        .send(Update::WorkspaceCreated(
            session_fixture("mid-session", SessionKind::Shell),
            "mid".into(),
            vec![],
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(
        app.layouts["a"]
            .tabs
            .iter()
            .map(|tab| tab.id.as_str())
            .collect::<Vec<_>>(),
        ["t0", "mid", "t1"]
    );
    assert_eq!(app.layouts["a"].active, "mid");
    assert!(app.workspace_insert.is_empty());
}

#[test]
fn leaf_plus_opens_stacked_in_that_split() {
    let (mut app, _, _dir) = fixture();
    let (jobs, received) = mpsc::channel();
    app.jobs = jobs.into();
    app.selected = Some("a".into());
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
    workspace.main_surface_mut().split_right(
        egui_dock::NodeIndex::root(),
        0.5,
        vec![Tab::Terminal("two".into())],
    );
    app.layouts.insert("a".into(), workspace);
    let path = app.layouts["a"]
        .find_tab(&Tab::Terminal("two".into()))
        .expect("right leaf present")
        .node_path();
    app.add_tab = Some((path, None));
    let mut dock = app.layouts.remove("a").unwrap();
    app.apply_add_tab("a", &mut dock);
    app.layouts.insert("a".into(), dock);
    let Job::Control(_, After::CreateAt(_, split)) = received.try_recv().unwrap() else {
        panic!("leaf plus stacks into the split");
    };
    assert_eq!(split, None);
}

#[test]
fn insert_here_stacks_into_focused_leaf() {
    let (mut app, _, _dir) = fixture();
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
    workspace.main_surface_mut().split_right(
        egui_dock::NodeIndex::root(),
        0.5,
        vec![Tab::Terminal("two".into())],
    );
    app.layouts.insert("a".into(), workspace);
    let editor = Tab::NativeEditor {
        path: "/a/note.md".into(),
    };
    app.insert("a", editor.clone(), Some(App::SPLIT_HERE));
    // No new top-level tab and no side split: the file stacks into
    // the focused leaf alongside its existing pane.
    assert_eq!(app.layouts["a"].tabs.len(), 1);
    let path = app.layouts["a"]
        .find_tab(&editor)
        .expect("editor placed")
        .node_path();
    let leaf = app.layouts["a"].leaf(path).expect("leaf present");
    assert_eq!(leaf.tabs.len(), 2);
}

#[test]
fn file_open_here_routes_stacked_create() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, received) = mpsc::channel();
    app.jobs = jobs.into();
    let session = session_fixture("one", SessionKind::Shell);
    app.state.sessions.push(session.clone());
    let target = services::Target::File("/a/note.md".into(), None, None);
    app.terminal_action(TerminalActionInput {
        ctx: &ctx,
        session: &session,
        target: &target,
        action: FileAction::Here,
    });
    let Job::Control(_, After::CreateAt(_, split)) = received.try_recv().unwrap() else {
        panic!("Here opens stacked in the current split");
    };
    assert_eq!(split.as_deref(), Some("here"));
}

#[test]
fn add_tab_to_the_left_records_insert_index() {
    let (mut app, _, _dir) = fixture();
    let (jobs, received) = mpsc::channel();
    app.jobs = jobs.into();
    app.selected = Some("a".into());
    app.create_workspace_tab(Some(1));
    let Job::Control(_, After::Workspace(id, anchors)) = received.try_recv().unwrap() else {
        panic!("Expected workspace create")
    };
    assert!(anchors.is_empty());
    assert_eq!(app.workspace_insert.get(&id), Some(&1));
}

#[test]
fn delayed_creation_uses_original_project_after_navigation_and_pane_removal() {
    let (mut app, ctx, _dir) = fixture();
    app.insert("a", Tab::Terminal("anchor".into()), None);
    app.select_project("b".into());
    app.remove_tab("anchor");
    let session = Session {
        remote: None,
        review: false,
        id: "created".into(),
        project_id: "a".into(),
        label: "new".into(),
        cwd: "/a/subdir".into(),
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
    };
    app.update_tx
        .send(Update::Created(
            session,
            None,
            Some(vec![Tab::Terminal("anchor".into())]),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(app.selected.as_deref(), Some("b"));
    assert!(
        app.layouts["a"]
            .find_tab(&Tab::Terminal("created".into()))
            .is_some()
    );
}

#[test]
fn cancelled_picker_and_error_keep_workspace_and_layout() {
    let (mut app, ctx, _dir) = fixture();
    app.insert("a", Tab::Terminal("original".into()), None);
    app.update_tx.send(Update::PickedProject(None, 0)).unwrap();
    app.update_tx
        .send(Update::Error("folder unavailable".into()))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(app.selected.as_deref(), Some("a"));
    assert!(
        app.layouts["a"]
            .find_tab(&Tab::Terminal("original".into()))
            .is_some()
    );
}

#[test]
fn delayed_open_refreshes_inventory_without_overriding_new_selection() {
    let (mut app, ctx, _dir) = fixture();
    app.select_project("b".into());
    app.select_project("a".into());
    app.update_tx
        .send(Update::OpenedProject(
            Box::new(app.state.clone()),
            "b".into(),
            0,
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(app.selected.as_deref(), Some("a"));
    app.update_tx
        .send(Update::OpenedProject(
            Box::new(app.state.clone()),
            "b".into(),
            app.selection_generation,
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(app.selected.as_deref(), Some("b"));
}
