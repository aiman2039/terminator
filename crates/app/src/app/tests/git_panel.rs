use super::*;

fn run_git_outcome(app: &mut App, ctx: &egui::Context, outcome: sidebar_ui::GitPanelOutcome) {
    let mut outcome = Some(outcome);
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        if let Some(outcome) = outcome.take() {
            app.perform_git_outcome(ui, outcome);
        }
    });
    output.textures_delta.clear();
}

#[test]
fn legacy_daemon_diff_never_sends_an_unsupported_creation_request() {
    let (mut app, ctx, _dir) = fixture();
    app.state.settings.review_mode = ReviewMode::Neovim;
    app.context = Some(services::ContextData {
        cwd: "/a".into(),
        root: Some("/a".into()),
        perforce: None,
        git_dirs: vec![],
        branch: "main".into(),
        changes: vec![],
        decorations: HashMap::default(),
        stats: HashMap::default(),
        error: None,
    });
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.selected = Some("a".into());
    app.error = Some("failed to fill whole buffer".into());
    for staged in [false, true] {
        let action =
            sidebar_ui::git_click_action(false, Some(staged), true, ReviewMode::Neovim, false)
                .unwrap();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            app.file_action(ui, action, Path::new("/a/file.rs"), None);
        });
        output.textures_delta.clear();
        let Job::Diff(Tab::Diff { staged: actual, .. }) = requests.recv().unwrap() else {
            panic!("Legacy daemon must use the local diff renderer");
        };
        assert_eq!(actual, staged);
    }
    assert!(requests.try_recv().is_err());
    assert!(app.error.is_none());
    assert!(app.info.as_ref().unwrap().contains("updated daemon"));
    assert!(app.state.sessions.is_empty());
    assert_eq!(app.layouts["a"].tabs.len(), 2);
}

#[test]
fn native_menu_diff_opens_the_built_in_viewer_when_neovim_is_selected() {
    let (mut app, _, _dir) = fixture();
    app.state.settings.review_mode = ReviewMode::Neovim;
    app.state.capabilities.push(NVIM_REVIEW_CAPABILITY.into());
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.selected = Some("a".into());
    app.spawn_diff(SpawnDiff {
        cwd: "/a".into(),
        path: "/a/file.rs".into(),
        staged: false,
        native: true,
    });
    let Job::Diff(Tab::Diff { staged, .. }) = requests.recv().unwrap() else {
        panic!("Native menu diff must stay in the GUI");
    };
    assert!(!staged);
    assert!(requests.try_recv().is_err());
    assert!(app.info.is_none());
    assert!(app.state.sessions.is_empty());
}

#[test]
fn native_review_skips_create_review_when_the_daemon_can_review() {
    let (mut app, ctx, _dir) = fixture();
    app.state.capabilities.push(NVIM_REVIEW_CAPABILITY.into());
    app.context = Some(services::ContextData {
        cwd: "/a".into(),
        root: Some("/a".into()),
        perforce: None,
        git_dirs: vec![],
        branch: "main".into(),
        changes: vec![],
        decorations: HashMap::default(),
        stats: HashMap::default(),
        error: None,
    });
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.selected = Some("a".into());
    let action = sidebar_ui::git_click_action(
        false,
        Some(false),
        true,
        app.state.settings.review_mode,
        true,
    )
    .unwrap();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        app.file_action(ui, action, Path::new("/a/file.rs"), None);
    });
    output.textures_delta.clear();
    let Job::Diff(Tab::Diff { staged, .. }) = requests.recv().unwrap() else {
        panic!("Native review must stay in the GUI");
    };
    assert!(!staged);
    assert!(requests.try_recv().is_err());
    assert!(app.info.is_none());
    assert!(app.state.sessions.is_empty());
}

#[test]
fn git_click_opens_a_native_diff() {
    let (mut app, ctx, _dir) = fixture();
    app.context = Some(services::ContextData {
        cwd: "/a".into(),
        root: Some("/a".into()),
        perforce: None,
        git_dirs: vec![],
        branch: "main".into(),
        changes: vec![],
        decorations: HashMap::default(),
        stats: HashMap::default(),
        error: None,
    });
    app.selected = Some("a".into());
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    let action = sidebar_ui::git_click_action(
        false,
        Some(false),
        true,
        app.state.settings.review_mode,
        false,
    )
    .unwrap();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        app.file_action(ui, action, Path::new("/a/dirty.rs"), None);
    });
    output.textures_delta.clear();
    let Job::Diff(Tab::Diff { path, staged, .. }) = requests.recv().unwrap() else {
        panic!("Git click must open a native diff");
    };
    assert_eq!(path, PathBuf::from("/a/dirty.rs"));
    assert!(!staged);
    assert!(requests.try_recv().is_err());
}

#[test]
#[cfg(feature = "test-support")]
fn git_panel_clicks_reuse_an_open_diff_instead_of_duplicating() {
    let (mut app, ctx, _dir) = fixture();
    app.context = Some(services::ContextData {
        cwd: "/a".into(),
        root: Some("/a".into()),
        perforce: None,
        git_dirs: vec![],
        branch: "main".into(),
        changes: vec![terminator_git::Change {
            path: "/a/dirty.rs".into(),
            status: " M".into(),
        }],
        decorations: HashMap::default(),
        stats: HashMap::default(),
        error: None,
    });
    app.selected = Some("a".into());
    ctx.options_mut(|options| options.input_options.max_double_click_delay = 5.0);
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    let mut clicks = 0;
    let mut draw = |events, time: f64| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(260.0, 240.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                let context = app.context.clone().unwrap();
                let outcome = {
                    let mut draft = String::new();
                    let input = sidebar_ui::GitPanelInput {
                        context: &context,
                        prepared: None,
                        review_mode: app.state.settings.review_mode,
                        neovim_review: false,
                        theme: &app.theme,
                        history: false,
                        view_list: false,
                        commits: &[],
                        shelves: &[],
                        branches: &[],
                        commit_draft: &mut draft,
                        recent_messages: &[],
                        amend: &mut false,
                        signoff: &mut false,
                        collapse_generation: 0,
                        open_shortcut: "",
                        compare: None,
                        base_ref: None,
                        committing: false,
                    };
                    sidebar_ui::git_panel(ui, &mut { input })
                };
                clicks += outcome.clicked.len();
                app.perform_git_outcome(ui, outcome);
            },
        );
        output.textures_delta.clear();
    };
    draw(vec![], 1.0);
    let rect = agent_target(&ctx, "git-file-dirty.rs").expect("git row is recorded");
    let pos = rect.center();
    draw(vec![egui::Event::PointerMoved(pos)], 1.01);
    for (time, pressed) in [(1.02, true), (1.03, false), (1.04, true), (1.05, false)] {
        draw(
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }],
            time,
        );
    }
    assert_eq!(clicks, 2);
    assert!(matches!(requests.try_recv().unwrap(), Job::Diff(_)));
    assert!(requests.try_recv().is_err());
    assert_eq!(
        app.layouts["a"]
            .iter_all_tabs()
            .filter(|(_, tab)| matches!(tab, Tab::Diff { .. }))
            .count(),
        1
    );
}

#[test]
fn git_reviews_open_distinct_top_level_tabs_in_the_origin_project() {
    let (mut app, ctx, _dir) = fixture();
    app.state.settings.review_mode = ReviewMode::Neovim;
    app.state.capabilities.push(NVIM_REVIEW_CAPABILITY.into());
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.selected = Some("a".into());
    app.context = Some(services::ContextData {
        cwd: "/a".into(),
        root: Some("/a".into()),
        perforce: None,
        git_dirs: vec![],
        branch: "main".into(),
        changes: vec![],
        decorations: HashMap::default(),
        stats: HashMap::default(),
        error: None,
    });
    for staged in [false, true] {
        let action =
            sidebar_ui::git_click_action(false, Some(staged), true, ReviewMode::Neovim, true)
                .unwrap();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            app.file_action(ui, action, Path::new("/a/file.rs"), None);
        });
        output.textures_delta.clear();
    }
    let mut ids = Vec::new();
    for staged in [false, true] {
        let Job::Control(request, After::Workspace(id, anchors)) = requests.recv().unwrap() else {
            panic!("Expected review workspace")
        };
        assert!(
            matches!(*request, Request::CreateReview { ref project, staged: actual, .. } if project == "a" && actual == staged)
        );
        ids.push(id.clone());
        app.selected = Some("b".into());
        let mut session = session_fixture(
            if staged { "staged" } else { "working" },
            SessionKind::Editor,
        );
        session.review = true;
        app.update_tx
            .send(Update::WorkspaceCreated(session, id, anchors))
            .unwrap();
        drain_updates(&mut app, &ctx);
        assert_eq!(app.selected.as_deref(), Some("b"));
    }
    assert_ne!(ids[0], ids[1]);
    assert!(app.layouts["a"].contains(&Tab::Terminal("working".into())));
    assert!(app.layouts["a"].contains(&Tab::Terminal("staged".into())));
}

#[test]
fn commit_switch_and_refresh_invalidate_git_lists() {
    let (mut app, ctx, _dir) = fixture();
    let root = PathBuf::from("/repo");
    app.git.list_root = Some(root.clone());
    app.update_tx
        .send(Update::Workspace(
            workspace_ops::Op::Stage(root.join("a.txt")),
            Ok(workspace_ops::Report::Message("Staged".into())),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(app.git.list_root, Some(root.clone()));
    app.update_tx
        .send(Update::Workspace(
            workspace_ops::Op::Commit {
                message: "nope".into(),
                amend: false,
                signoff: false,
                push: false,
            },
            Err("commit failed".into()),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(app.git.list_root, Some(root.clone()));
    app.update_tx
        .send(Update::Workspace(
            workspace_ops::Op::Commit {
                message: "yes".into(),
                amend: false,
                signoff: false,
                push: false,
            },
            Ok(workspace_ops::Report::Message("Committed".into())),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(app.git.list_root.is_none());
    app.git.list_root = Some(root.clone());
    app.update_tx
        .send(Update::Workspace(
            workspace_ops::Op::Switch("feature".into()),
            Ok(workspace_ops::Report::Message("Switched".into())),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(app.git.list_root.is_none());
    app.git.list_root = Some(root.clone());
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        },
        |ui| {
            let outcome = sidebar_ui::GitPanelOutcome {
                refresh: true,
                ..Default::default()
            };
            app.perform_git_outcome(ui, outcome);
        },
    );
    output.textures_delta.clear();
    assert!(app.git.list_root.is_none());
}

#[test]
fn git_views_load_asynchronously_and_reload_after_a_commit() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, requests) = std::sync::mpsc::channel();
    app.jobs = jobs.into();
    let tab = Tab::Blame {
        cwd: PathBuf::from("/a"),
        path: PathBuf::from("/a/f.rs"),
    };
    let key = tab.key();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.blame_view(ui, &tab));
    output.textures_delta.clear();
    assert!(matches!(
        app.git_views.get(&key),
        Some(crate::GitView::Loading)
    ));
    assert!(
        requests
            .try_iter()
            .any(|job| matches!(job, Job::GitView(ref t) if t.key() == key))
    );
    app.update_tx
        .send(Update::GitView(
            key.clone(),
            crate::GitView::Blame(Ok(crate::git_log::BlameData::default())),
        ))
        .unwrap();
    app.update_tx
        .send(Update::GitView(
            "closed".into(),
            crate::GitView::Blame(Err("late".into())),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(matches!(
        app.git_views.get(&key),
        Some(crate::GitView::Blame(Ok(_)))
    ));
    assert!(!app.git_views.contains_key("closed"));
    app.update_tx
        .send(Update::Workspace(
            workspace_ops::Op::Commit {
                message: "m".into(),
                amend: false,
                signoff: false,
                push: false,
            },
            Ok(workspace_ops::Report::Message("Committed".into())),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(app.git_views.is_empty());
}

#[test]
fn discard_and_drop_wait_for_confirmation() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, requests) = std::sync::mpsc::channel();
    app.jobs = jobs.into();
    let path = PathBuf::from("/a/new.txt");
    run_git_outcome(
        &mut app,
        &ctx,
        sidebar_ui::GitPanelOutcome {
            discard: vec![(path.clone(), true)],
            ..Default::default()
        },
    );
    assert_no_workspace_job(&requests);
    assert_eq!(
        app.git.confirm,
        Some(sidebar_ui::GitConfirm::Discard {
            path,
            untracked: true
        })
    );
    run_git_outcome(
        &mut app,
        &ctx,
        sidebar_ui::GitPanelOutcome {
            drop_shelf: Some("stash@{0}".into()),
            ..Default::default()
        },
    );
    assert_no_workspace_job(&requests);
    assert_eq!(
        app.git.confirm,
        Some(sidebar_ui::GitConfirm::DropShelf("stash@{0}".into()))
    );
}

#[test]
fn failed_commit_restores_the_draft_but_failed_push_does_not() {
    let (mut app, ctx, _dir) = fixture();
    let commit = |push| workspace_ops::Op::Commit {
        message: "msg".into(),
        amend: false,
        signoff: false,
        push,
    };
    app.git.commit = "msg".into();
    run_git_outcome(
        &mut app,
        &ctx,
        sidebar_ui::GitPanelOutcome {
            commit: true,
            ..Default::default()
        },
    );
    assert!(app.git.commit.is_empty());
    assert_eq!(app.git.commit_inflight.as_deref(), Some("msg"));
    app.update_tx
        .send(Update::Workspace(
            commit(false),
            Err("hook rejected".into()),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(app.git.commit, "msg");
    assert!(app.git.commit_inflight.is_none());

    app.git.commit.clear();
    app.git.commit_inflight = Some("msg".into());
    app.update_tx
        .send(Update::Workspace(
            commit(true),
            Err(format!("{}: rejected", workspace_ops::PUSH_FAILED)),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(app.git.commit.is_empty());
    assert!(app.git.commit_inflight.is_none());

    app.git.commit_inflight = Some("msg".into());
    app.update_tx
        .send(Update::Workspace(
            commit(false),
            Ok(workspace_ops::Report::Message("Committed".into())),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(app.git.commit.is_empty());
    assert!(app.git.commit_inflight.is_none());
}

#[test]
fn git_refresh_rerequests_only_diffs_whose_file_changed() {
    let (mut app, _ctx, _dir) = fixture();
    let cwd = PathBuf::from("/a");
    let tab = |name: &str| Tab::Diff {
        remote: None,
        cwd: cwd.clone(),
        path: cwd.join(name),
        staged: false,
    };
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(egui_dock::DockState::new(vec![tab("x.rs"), tab("y.rs")])),
    );
    for name in ["x.rs", "y.rs"] {
        app.diffs.insert(
            tab(name).key(),
            Ok(std::sync::Arc::new(crate::diff::DiffDocument::default())),
        );
    }
    let change = |name: &str, status: &str| terminator_git::Change {
        path: cwd.join(name),
        status: status.into(),
    };
    let context = |changes: Vec<terminator_git::Change>, stats: Vec<(&str, (u32, u32))>| {
        crate::services::ContextData {
            cwd: cwd.clone(),
            root: Some(cwd.clone()),
            perforce: None,
            git_dirs: Vec::new(),
            branch: "main".into(),
            changes,
            decorations: HashMap::default(),
            stats: stats
                .into_iter()
                .map(|(name, stat)| (cwd.join(name), stat))
                .collect(),
            error: None,
        }
    };
    app.context = Some(context(
        vec![change("x.rs", " M"), change("y.rs", " M")],
        vec![("x.rs", (1, 0)), ("y.rs", (2, 0))],
    ));
    let before = app.diff_fingerprints();
    // An agent edits x.rs; y.rs is untouched.
    app.context = Some(context(
        vec![change("x.rs", " M"), change("y.rs", " M")],
        vec![("x.rs", (5, 1)), ("y.rs", (2, 0))],
    ));
    let (jobs, received) = mpsc::channel();
    app.jobs = jobs.into();
    app.refresh_stale_diffs(&before);
    let refreshed: Vec<String> = received
        .try_iter()
        .filter_map(|job| match job {
            Job::DiffContext(tab, _) => Some(tab.key()),
            _ => None,
        })
        .collect();
    assert_eq!(refreshed, [tab("x.rs").key()]);
    assert!(app.loading.contains(&tab("x.rs").key()));
}
