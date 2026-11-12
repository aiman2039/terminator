use super::*;
use crate::app::FileActionInput;

pub(super) fn run_git_outcome(
    app: &mut App,
    ctx: &egui::Context,
    outcome: sidebar_ui::GitPanelOutcome,
) {
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
    app.error.set("failed to fill whole buffer");
    for staged in [false, true] {
        let action = sidebar_ui::git_click_action(sidebar_ui::GitClickActionInput {
            deleted: false,
            staged: Some(staged),
            clicked: true,
            review_mode: ReviewMode::Neovim,
            neovim_review: false,
        })
        .unwrap();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            app.file_action(FileActionInput {
                ui,
                action,
                path: Path::new("/a/file.rs"),
                line: None,
            });
        });
        output.textures_delta.clear();
        let Job::Diff(Tab::Diff { staged: actual, .. }) = requests.recv().unwrap() else {
            panic!("Legacy daemon must use the local diff renderer");
        };
        assert_eq!(actual, staged);
    }
    assert!(requests.try_recv().is_err());
    assert!(app.error.text().is_none());
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
    let action = sidebar_ui::git_click_action(sidebar_ui::GitClickActionInput {
        deleted: false,
        staged: Some(false),
        clicked: true,
        review_mode: app.state.settings.review_mode,
        neovim_review: true,
    })
    .unwrap();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        app.file_action(FileActionInput {
            ui,
            action,
            path: Path::new("/a/file.rs"),
            line: None,
        });
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
    let action = sidebar_ui::git_click_action(sidebar_ui::GitClickActionInput {
        deleted: false,
        staged: Some(false),
        clicked: true,
        review_mode: app.state.settings.review_mode,
        neovim_review: false,
    })
    .unwrap();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        app.file_action(FileActionInput {
            ui,
            action,
            path: Path::new("/a/dirty.rs"),
            line: None,
        });
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
                        tab: crate::sidebar_ui::GitTab::Commit,
                        view_list: false,
                        commits: &[],
                        shelves: &[],
                        branches: &[],
                        commit_draft: &mut draft,
                        recent_messages: &[],
                        amend: &mut false,
                        signoff: &mut false,
                        collapse_generation: 0,
                        tree_open: true,
                        open_shortcut: "",
                        compare: None,
                        base_ref: None,
                        focus_message: false,
                        commit_run: None,
                        commit_lock: Default::default(),
                        commit_failure: None,
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
        let action = sidebar_ui::git_click_action(sidebar_ui::GitClickActionInput {
            deleted: false,
            staged: Some(staged),
            clicked: true,
            review_mode: ReviewMode::Neovim,
            neovim_review: true,
        })
        .unwrap();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            app.file_action(FileActionInput {
                ui,
                action,
                path: Path::new("/a/file.rs"),
                line: None,
            });
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
            PathBuf::from("/repo"),
            workspace_ops::Op::Stage(root.join("a.txt")),
            Ok(workspace_ops::Report::Message("Staged".into())),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(app.git.list_root, Some(root.clone()));
    app.update_tx
        .send(Update::Workspace(
            PathBuf::from("/repo"),
            commit_op("nope", false),
            Err("commit failed".into()),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(app.git.list_root, Some(root.clone()));
    app.update_tx
        .send(Update::Workspace(
            PathBuf::from("/repo"),
            commit_op("yes", false),
            Ok(workspace_ops::Report::Message("Committed".into())),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(app.git.list_root.is_none());
    app.git.list_root = Some(root.clone());
    app.update_tx
        .send(Update::Workspace(
            PathBuf::from("/repo"),
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
            PathBuf::from("/repo"),
            commit_op("m", false),
            Ok(workspace_ops::Report::Message("Committed".into())),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(app.git_views.is_empty());
}

/// A commit made in a terminal changes History and Blame; their Reload asks
/// Git again (a replaceable GUI job, no daemon request).
#[cfg(feature = "test-support")]
#[test]
fn reload_asks_git_again_for_an_open_blame_or_file_history() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, requests) = std::sync::mpsc::channel();
    app.jobs = jobs.into();
    let (cwd, path) = (PathBuf::from("/a"), PathBuf::from("/a/f.rs"));
    let blame = Tab::Blame {
        cwd: cwd.clone(),
        path: path.clone(),
    };
    let history = Tab::FileHistory { cwd, path };
    app.git_views.insert(
        blame.key(),
        crate::GitView::Blame(Ok(crate::git_log::BlameData::default())),
    );
    app.git_views
        .insert(history.key(), crate::GitView::History(Ok(Vec::new())));
    for tab in [blame, history] {
        let frame = |app: &mut App, events: Vec<egui::Event>| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 400.0),
                )),
                events,
                ..Default::default()
            };
            let mut output = ctx.run_ui(raw, |ui| match &tab {
                Tab::Blame { .. } => app.blame_view(ui, &tab),
                _ => app.file_history_view(ui, &tab),
            });
            output.textures_delta.clear();
        };
        frame(&mut app, vec![]);
        assert!(requests.try_iter().next().is_none(), "the view is loaded");
        let pos = header_target(&ctx)("git-view-reload").unwrap().center();
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut app, vec![egui::Event::PointerMoved(pos)]);
        frame(&mut app, vec![press(true)]);
        frame(&mut app, vec![press(false)]);
        assert!(
            matches!(app.git_views.get(&tab.key()), Some(crate::GitView::Loading)),
            "{tab:?}"
        );
        let sent: Vec<Job> = requests.try_iter().collect();
        assert!(
            matches!(sent.as_slice(), [Job::GitView(t)] if *t == tab),
            "{tab:?}: {} jobs",
            sent.len()
        );
    }
}

#[test]
fn a_git_tab_opened_from_a_pane_lands_in_the_painted_workspace() {
    let (mut app, _ctx, _dir) = fixture();
    app.selected = Some("a".into());
    app.insert("a", Tab::Terminal("shell".into()), None);
    let tab = Tab::FileHistory {
        cwd: PathBuf::from("/a"),
        path: PathBuf::from("/a/f.rs"),
    };
    // As in `workspace_project`: the dock is out of `layouts` while panes
    // paint, and a Commit Log or File History click opens a tab then.
    let dock = app.layouts.remove("a").unwrap();
    app.pending_git_tabs = Some(Vec::new());
    app.open_git_tab(tab.clone());
    app.layouts.insert("a".into(), dock);
    app.drain_pending_git_tabs();
    assert!(app.layouts["a"].contains(&tab));
    assert!(app.layouts["a"].contains(&Tab::Terminal("shell".into())));
    assert!(app.pending_git_tabs.is_none());
}

#[test]
fn a_closed_blame_tab_loads_fresh_data_when_opened_again() {
    let (mut app, _ctx, _dir) = fixture();
    let tab = Tab::Blame {
        cwd: PathBuf::from("/a"),
        path: PathBuf::from("/a/f.rs"),
    };
    let key = tab.key();
    app.insert("a", Tab::Terminal("shell".into()), None);
    app.insert("a", tab.clone(), None);
    app.git_views.insert(
        key.clone(),
        crate::GitView::Blame(Ok(crate::git_log::BlameData::default())),
    );
    let group = app.layouts["a"]
        .tabs
        .iter()
        .find(|group| group.layout.find_tab(&tab).is_some())
        .map(|group| group.id.clone())
        .unwrap();
    app.close_workspace_tab_now("a", &group);
    assert!(!app.layouts["a"].contains(&tab));
    assert!(!app.git_views.contains_key(&key));
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
        app.git.confirm.clone().map(|confirm| confirm.action),
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
        app.git.confirm.clone().map(|confirm| confirm.action),
        Some(sidebar_ui::GitConfirm::DropShelf("stash@{0}".into()))
    );
}

fn commit_op(message: &str, push: bool) -> workspace_ops::Op {
    workspace_ops::Op::Commit {
        message: message.into(),
        amend: false,
        signoff: false,
        push,
        cancel: Default::default(),
    }
}

pub(super) fn click_commit(app: &mut App, ctx: &egui::Context, push: bool) {
    run_git_outcome(
        app,
        ctx,
        sidebar_ui::GitPanelOutcome {
            commit: !push,
            commit_push: push,
            ..Default::default()
        },
    );
}

/// The commit job the panel queued: its root and op.
pub(super) fn sent_commit(requests: &mpsc::Receiver<Job>) -> (PathBuf, workspace_ops::Op) {
    requests
        .try_iter()
        .find_map(|job| match job {
            Job::Workspace(root, op @ workspace_ops::Op::Commit { .. }) => Some((root, op)),
            _ => None,
        })
        .expect("a commit job was queued")
}

pub(super) fn reply(
    app: &mut App,
    ctx: &egui::Context,
    job: (PathBuf, workspace_ops::Op),
    result: Result<workspace_ops::Report, String>,
) {
    let (root, op) = job;
    app.update_tx
        .send(Update::Workspace(root, op, result))
        .unwrap();
    drain_updates(app, ctx);
}

#[test]
fn failed_commit_restores_the_draft_but_failed_push_does_not() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    let pushed = format!("{} (exit status: 1)\nrejected", workspace_ops::PUSH_FAILED);
    for (push, error, restored) in [
        (false, "Commit failed (exit status: 1)\nhook rejected", true),
        (true, pushed.as_str(), false),
    ] {
        app.git.commit = "msg".into();
        click_commit(&mut app, &ctx, push);
        assert!(app.git.commit.is_empty());
        assert!(app.git.run.is_some());
        reply(&mut app, &ctx, sent_commit(&requests), Err(error.into()));
        assert_eq!(app.git.commit == "msg", restored, "{error}");
        assert!(app.git.run.is_none());
        // The panel keeps the output; the status bar names the failure.
        assert_eq!(app.git.commit_failure.as_deref(), Some(error));
        assert_eq!(app.error.text(), error.lines().next());
    }
    app.git.commit = "msg".into();
    click_commit(&mut app, &ctx, false);
    assert!(app.git.commit_failure.is_none(), "a new commit clears it");
    let ok = Ok(workspace_ops::Report::Message("Committed".into()));
    reply(&mut app, &ctx, sent_commit(&requests), ok);
    assert!(app.git.commit.is_empty());
    assert!(app.git.run.is_none());
    assert_eq!(app.info.as_deref(), Some("Committed"));
}

#[test]
fn commit_progress_and_cancel_follow_the_job() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.git.list_root = Some(PathBuf::from("/a"));
    app.git.commit = "msg".into();
    click_commit(&mut app, &ctx, true);
    let job = sent_commit(&requests);
    let workspace_ops::Op::Commit {
        cancel, push: true, ..
    } = &job.1
    else {
        panic!("Commit and Push queues a pushing commit");
    };
    let token = cancel.clone();
    let progress = |app: &App| {
        app.git.run.as_ref().map(|run| {
            let progress = run.progress();
            (progress.phase, progress.cancelling)
        })
    };
    assert_eq!(
        progress(&app),
        Some((sidebar_ui::CommitPhase::Committing, false))
    );
    // The commit landed; the push runs, and the lists show the commit.
    reply(
        &mut app,
        &ctx,
        job.clone(),
        Ok(workspace_ops::Report::Pushing),
    );
    assert_eq!(
        progress(&app),
        Some((sidebar_ui::CommitPhase::Pushing, false))
    );
    assert!(app.git.list_root.is_none());
    app.git.list_root = Some(PathBuf::from("/a"));
    // Cancel stops the job; the panel waits for its reply.
    run_git_outcome(
        &mut app,
        &ctx,
        sidebar_ui::GitPanelOutcome {
            cancel_commit: true,
            ..Default::default()
        },
    );
    assert!(token.is_cancelled());
    assert_eq!(
        progress(&app),
        Some((sidebar_ui::CommitPhase::Pushing, true))
    );
    reply(
        &mut app,
        &ctx,
        job,
        Err(workspace_ops::PUSH_CANCELLED.into()),
    );
    assert!(app.git.run.is_none());
    assert!(app.git.commit.is_empty(), "the commit landed");
    assert_eq!(app.info.as_deref(), Some(workspace_ops::PUSH_CANCELLED));
    assert!(app.git.commit_failure.is_none(), "a cancel is a notice");
    assert!(app.git.list_root.is_none(), "the log changed");
}

#[test]
fn a_second_commit_click_waits_for_the_running_job() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.git.commit = "first".into();
    click_commit(&mut app, &ctx, false);
    let job = sent_commit(&requests);
    app.git.commit = "second".into();
    click_commit(&mut app, &ctx, true);
    assert!(
        requests
            .try_iter()
            .all(|job| !matches!(job, Job::Workspace(_, workspace_ops::Op::Commit { .. }))),
        "no second commit job"
    );
    assert_eq!(app.git.commit, "second", "the new draft stays");
    // A cancelled commit gives its message back only to an empty draft.
    run_git_outcome(
        &mut app,
        &ctx,
        sidebar_ui::GitPanelOutcome {
            cancel_commit: true,
            ..Default::default()
        },
    );
    reply(
        &mut app,
        &ctx,
        job,
        Err(workspace_ops::COMMIT_CANCELLED.into()),
    );
    assert_eq!(app.git.commit, "second");
    assert_eq!(app.info.as_deref(), Some(workspace_ops::COMMIT_CANCELLED));
    app.git.commit = "third".into();
    click_commit(&mut app, &ctx, false);
    sent_commit(&requests);
}

#[test]
fn a_dismissed_commit_failure_leaves_the_panel() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.git.commit = "msg".into();
    click_commit(&mut app, &ctx, false);
    reply(
        &mut app,
        &ctx,
        sent_commit(&requests),
        Err("Commit failed (exit status: 1)\nhook said no".into()),
    );
    assert!(app.git.commit_failure.is_some());
    run_git_outcome(
        &mut app,
        &ctx,
        sidebar_ui::GitPanelOutcome {
            dismiss_commit_failure: true,
            ..Default::default()
        },
    );
    assert!(app.git.commit_failure.is_none());
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

#[test]
fn diff_navigation_shortcuts_apply_only_to_an_active_diff() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, _requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.selected = Some("a".into());
    assert!(!app.shortcut_applies(Action::DiffNextFile));
    app.spawn_diff(SpawnDiff {
        cwd: "/a".into(),
        path: "/a/file.rs".into(),
        staged: false,
        native: true,
    });
    for action in [
        Action::DiffNextChange,
        Action::DiffPrevChange,
        Action::DiffNextFile,
        Action::DiffPrevFile,
    ] {
        assert!(app.shortcut_applies(action), "{action:?}");
        app.run_shortcut(&ctx, action);
        assert_eq!(app.diff_nav.take().map(|(nav, _)| nav), Some(action));
    }
}

#[test]
fn a_palette_diff_action_survives_into_the_next_frame_only() {
    let (mut app, ctx, _dir) = fixture();
    let pass = |app: &mut App| {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            app.dispatch_shortcuts(ui.ctx(), true);
        });
        output.textures_delta.clear();
    };
    // As the palette does: the action runs after this pass's diff painted.
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        app.run_shortcut(ui.ctx(), Action::DiffNextChange);
    });
    output.textures_delta.clear();
    pass(&mut app);
    assert_eq!(
        app.diff_nav.map(|(nav, _)| nav),
        Some(Action::DiffNextChange),
        "the next frame's diff view still takes it"
    );
    pass(&mut app);
    assert_eq!(app.diff_nav, None, "an untaken diff key does not stay");
}

#[test]
fn stale_git_list_replies_do_not_overwrite_the_current_repo() {
    let (mut app, ctx, _dir) = fixture();
    let current = PathBuf::from("/current");
    app.context = Some(services::ContextData {
        cwd: current.clone(),
        root: Some(current.clone()),
        perforce: None,
        git_dirs: vec![],
        branch: "main".into(),
        changes: vec![],
        decorations: HashMap::default(),
        stats: HashMap::default(),
        error: None,
    });
    app.git.list_root = Some(current.clone());
    for (root, branch) in [("/old", "old-branch"), ("/current", "main")] {
        app.update_tx
            .send(Update::Workspace(
                PathBuf::from(root),
                workspace_ops::Op::Branches,
                Ok(workspace_ops::Report::Branches(vec![branch.into()])),
            ))
            .unwrap();
        app.update_tx
            .send(Update::Workspace(
                PathBuf::from(root),
                workspace_ops::Op::Log,
                Ok(workspace_ops::Report::Log(vec![(
                    branch.into(),
                    "msg".into(),
                )])),
            ))
            .unwrap();
    }
    drain_updates(&mut app, &ctx);
    assert_eq!(app.git.branches, vec!["main".to_string()]);
    assert_eq!(app.git.log, vec![("main".to_string(), "msg".to_string())]);
    app.update_tx
        .send(Update::Workspace(
            PathBuf::from("/old"),
            workspace_ops::Op::Branches,
            Ok(workspace_ops::Report::Branches(vec!["late".into()])),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(app.git.branches, vec!["main".to_string()]);
    assert_eq!(app.git.list_root, Some(current));
}

#[test]
fn git_delete_replaces_an_unanswered_explorer_delete() {
    let (mut app, ctx, _dir) = fixture();
    // An Explorer multi-delete is still waiting for an answer.
    app.pending_delete = vec![PathBuf::from("/a/x.rs"), PathBuf::from("/a/y.rs")];
    let mut outcome = sidebar_ui::GitPanelOutcome::default();
    outcome.delete.push(PathBuf::from("/a/git.rs"));
    run_git_outcome(&mut app, &ctx, outcome);
    // The Git panel's file alone is asked about; no Explorer path rides along.
    assert_eq!(app.pending_delete, [PathBuf::from("/a/git.rs")]);
}

#[test]
fn a_git_question_is_dropped_when_the_repository_changes() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, requests) = std::sync::mpsc::channel();
    app.jobs = jobs.into();
    run_git_outcome(
        &mut app,
        &ctx,
        sidebar_ui::GitPanelOutcome {
            drop_shelf: Some("stash@{1}".into()),
            ..Default::default()
        },
    );
    assert_eq!(
        app.git.confirm.as_ref().map(|confirm| confirm.root.clone()),
        Some(PathBuf::from("/a"))
    );
    // Another project's repository: stash@{1} there is a different shelf.
    app.selected = Some("b".into());
    app.preferences.tool = SidebarTool::Git;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(360.0, 480.0),
            )),
            ..Default::default()
        },
        |ui| app.sidebar(ui),
    );
    output.textures_delta.clear();
    assert!(app.git.confirm.is_none());
    assert_no_workspace_job(&requests);
}
