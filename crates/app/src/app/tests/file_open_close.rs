use super::*;
use crate::app::OpenFileInput;

#[test]
fn closing_a_file_from_the_leftmost_tab_restores_its_origin() {
    let (mut app, ctx, _dir) = fixture();
    app.insert("a", Tab::Terminal("shell".into()), None);
    let origin = app.layouts["a"].active.clone();
    app.layouts
        .get_mut("a")
        .unwrap()
        .add("right".into(), Tab::Terminal("right".into()));
    app.layouts.get_mut("a").unwrap().active = origin.clone();
    app.update_tx
        .send(Update::WorkspaceCreated(
            session_fixture("editor", SessionKind::Editor),
            "file".into(),
            vec![Tab::Terminal("shell".into())],
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    app.editors_closed(
        editor_close::Target::Workspace("a".into(), "file".into()),
        vec!["editor".into()],
        Ok(()),
    );
    assert_eq!(app.layouts["a"].active, origin);
    assert_eq!(app.active_session.as_deref(), Some("shell"));
}

#[test]
fn gui_file_views_restore_the_issuing_tab_after_navigation() {
    for file in [
        Tab::Image {
            path: "/a/image.png".into(),
        },
        Tab::browser_file("/a/page.html".into()),
        Tab::NativeEditor {
            path: "/a/file.rs".into(),
        },
    ] {
        let (mut app, _, _dir) = fixture();
        app.insert("a", Tab::Terminal("shell".into()), None);
        let origin = app.layouts["a"].active.clone();
        let after = app.editor_target("a", None, None);
        app.layouts
            .get_mut("a")
            .unwrap()
            .add("right".into(), Tab::Terminal("right".into()));
        app.place_gui_tab("a".into(), file.clone(), after);
        let file_id = app.layouts["a"].active.clone();
        if let Tab::NativeEditor { path } = file {
            app.close_native_tab(&path, false, None);
        } else {
            app.close_workspace_tab_now("a", &file_id);
        }
        assert_eq!(app.layouts["a"].active, origin);
    }
}

#[test]
fn failed_or_cancelled_file_close_keeps_the_file_and_focus() {
    let (mut app, _, _dir) = fixture();
    app.insert("a", Tab::Terminal("shell".into()), None);
    app.layouts
        .get_mut("a")
        .unwrap()
        .add("file".into(), Tab::Terminal("editor".into()));
    app.active_session = Some("editor".into());
    let target = editor_close::Target::Workspace("a".into(), "file".into());
    app.editors_closed(
        target.clone(),
        vec!["editor".into()],
        Err("Unsaved changes".into()),
    );
    assert_eq!(app.layouts["a"].active, "file");
    assert_eq!(app.active_session.as_deref(), Some("editor"));
    assert!(app.unsaved_close_prompt("editor").is_some());
    app.apply_unsaved_close_choice(
        appearance::UnsavedCloseChoice::Cancel,
        target,
        vec!["editor".into()],
    );
    assert_eq!(app.layouts["a"].active, "file");
    assert_eq!(app.active_session.as_deref(), Some("editor"));
    assert!(app.layouts["a"].contains(&Tab::Terminal("editor".into())));
    assert!(app.unsaved_close_prompt("editor").is_none());
}

#[test]
fn closing_editor_tab_restores_the_other_tabs_latest_pane_focus() {
    let (mut app, ctx, _dir) = fixture();
    app.state.sessions.extend([
        session_fixture("shell", SessionKind::Shell),
        session_fixture("other", SessionKind::Shell),
    ]);
    app.insert("a", Tab::Terminal("shell".into()), None);
    let original = app.layouts["a"].active.clone();
    app.update_tx
        .send(Update::WorkspaceCreated(
            session_fixture("editor", SessionKind::Editor),
            "file".into(),
            vec![Tab::Terminal("shell".into())],
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    app.layouts.get_mut("a").unwrap().active = original.clone();
    app.insert("a", Tab::Terminal("other".into()), Some("right"));
    app.layouts.get_mut("a").unwrap().active = "file".into();
    app.active_session = Some("editor".into());
    let mut state = app.state.clone();
    state
        .sessions
        .iter_mut()
        .find(|s| s.id == "editor")
        .unwrap()
        .lifecycle = Lifecycle::Ended;
    app.apply_state(state);
    assert_eq!(app.layouts["a"].active, original);
    assert_eq!(app.active_session.as_deref(), Some("other"));
}

#[test]
fn rapid_file_activation_opens_one_editor_and_a_later_click_opens_another() {
    let (mut app, ctx, directory) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    // Wider than one click on a slow runner. A later click is aged past
    // this window instead of sleeping through it.
    ctx.options_mut(|options| options.input_options.max_double_click_delay = 60.0);
    let path = directory.path().join("file.rs");
    let click = |app: &mut App, ctx: &egui::Context| {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            app.activate_file_action(ui, &path, FileAction::Open);
        });
        output.textures_delta.clear();
    };
    let creates = || {
        requests
            .try_iter()
            .filter(|job| {
                matches!(
                    job,
                    Job::Control(request, _)
                        if matches!(request.as_ref(), Request::Create { .. })
                )
            })
            .count()
    };
    click(&mut app, &ctx);
    click(&mut app, &ctx);
    assert_eq!(creates(), 1);
    app.file_activation.as_mut().unwrap().at = Instant::now()
        .checked_sub(Duration::from_mins(2))
        .unwrap_or_else(Instant::now);
    click(&mut app, &ctx);
    assert_eq!(creates(), 1);
}

#[test]
fn editor_open_preserves_shell_tabs_and_quit_restores_original_focus() {
    for lower in [false, true] {
        let (mut app, ctx, _dir) = fixture();
        let shell = session_fixture("shell", SessionKind::Shell);
        app.state.sessions.push(shell.clone());
        app.insert("a", Tab::Terminal(shell.id.clone()), None);
        if lower {
            let lower = session_fixture("lower", SessionKind::Shell);
            app.state.sessions.push(lower);
            app.insert("a", Tab::Terminal("lower".into()), Some("down"));
        }
        let original = if lower { "lower" } else { "shell" };
        app.active_session = Some(original.into());
        let After::Workspace(workspace_id, anchors) = app.editor_target("a", None, None) else {
            panic!("Expected anchored editor creation")
        };

        let editor = session_fixture("editor", SessionKind::Editor);
        app.update_tx
            .send(Update::WorkspaceCreated(
                editor.clone(),
                workspace_id,
                anchors,
            ))
            .unwrap();
        drain_updates(&mut app, &ctx);
        assert!(app.layouts["a"].contains(&Tab::Terminal(original.into())));
        assert_eq!(app.layouts["a"].tabs.len(), 2);
        assert_eq!(app.layouts["a"].iter_all_tabs().count(), 1);
        let mut state = app.state.clone();
        state
            .sessions
            .iter_mut()
            .find(|s| s.id == editor.id)
            .unwrap()
            .lifecycle = Lifecycle::Ended;
        app.apply_state(state);
        assert!(
            app.layouts["a"]
                .find_tab(&Tab::Terminal(editor.id))
                .is_none()
        );
        assert!(
            app.layouts["a"]
                .find_tab(&Tab::Terminal(original.into()))
                .is_some()
        );
        assert_eq!(app.active_session.as_deref(), Some(original));
        assert!(
            app.state
                .sessions
                .iter()
                .find(|s| s.id == original)
                .unwrap()
                .lifecycle
                .live()
        );
    }
}

#[test]
fn editor_exit_does_not_change_another_projects_focus() {
    let (mut app, ctx, _dir) = fixture();
    app.state
        .sessions
        .push(session_fixture("shell", SessionKind::Shell));
    app.insert("a", Tab::Terminal("shell".into()), None);
    app.update_tx
        .send(Update::Created(
            session_fixture("editor", SessionKind::Editor),
            Some("right".into()),
            Some(vec![Tab::Terminal("shell".into())]),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    app.select_project("b".into());
    app.insert("b", Tab::Terminal("other".into()), None);
    app.active_session = Some("other".into());
    let mut state = app.state.clone();
    state
        .sessions
        .iter_mut()
        .find(|s| s.id == "editor")
        .unwrap()
        .lifecycle = Lifecycle::Ended;
    app.apply_state(state);
    assert_eq!(app.selected.as_deref(), Some("b"));
    assert_eq!(app.active_session.as_deref(), Some("other"));
}

#[test]
fn repeated_gui_show_focuses_the_existing_session_without_duplicate_tabs() {
    let (mut app, ctx, _dir) = fixture();
    app.state
        .sessions
        .push(session_fixture("shell", SessionKind::Shell));
    app.insert("a", Tab::Terminal("shell".into()), None);
    for _ in 0..2 {
        app.ui_request(
            &ctx,
            terminator_core::ui_control::Request::ShowSession {
                session: "shell".into(),
                anchor: None,
                split: None,
            },
        )
        .unwrap();
    }
    assert_eq!(app.layouts["a"].tabs.len(), 1);
    assert_eq!(app.layouts["a"].iter_all_tabs().count(), 1);
}

#[test]
fn image_open_creates_no_editor_and_keeps_original_project() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.open_file(OpenFileInput {
        path: crate::test_path("/a/image.PNG"),
        line: None,
        split: None,
        external: false,
    });
    app.select_project("b".into());
    pump_until(&mut app, &ctx, |app| {
        app.layouts["a"].contains(&Tab::Image {
            path: crate::test_path("/a/image.PNG"),
        })
    });
    assert_eq!(app.selected.as_deref(), Some("b"));
    assert!(app.layouts["a"].contains(&Tab::Image {
        path: crate::test_path("/a/image.PNG")
    }));
    assert_eq!(app.layouts["a"].version, 3);
    assert!(app.state.sessions.is_empty());
    assert!(!requests.try_iter().any(|j|matches!(j,Job::Control(request,_) if matches!(*request,Request::Create { editor:true,.. }))));
}

#[test]
fn html_open_creates_no_editor_and_keeps_original_project() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.open_file(OpenFileInput {
        path: crate::test_path("/a/index.HTML"),
        line: None,
        split: None,
        external: false,
    });
    app.select_project("b".into());
    pump_until(&mut app, &ctx, |app| {
        app.layouts["a"].contains(&Tab::browser_file(crate::test_path("/a/index.HTML")))
    });
    assert_eq!(app.selected.as_deref(), Some("b"));
    assert!(app.layouts["a"].contains(&Tab::browser_file(crate::test_path("/a/index.HTML"))));
    assert_eq!(app.layouts["a"].version, 6);
    assert!(app.state.sessions.is_empty());
    assert!(!requests.try_iter().any(|j|matches!(j,Job::Control(request,_) if matches!(*request,Request::Create { editor:true,.. }))));
}

#[test]
fn http_url_opens_browser_tab_and_rejects_other_schemes() {
    let (mut app, ctx, _dir) = fixture();
    app.open_browser_url("a", "https://example.com/app", None)
        .unwrap();
    app.select_project("b".into());
    pump_until(&mut app, &ctx, |app| {
        app.layouts["a"].contains(&Tab::Browser {
            id: String::new(),
            target: BrowserTarget::Url("https://example.com/app".into()),
        })
    });
    assert_eq!(app.selected.as_deref(), Some("b"));
    assert!(app.layouts["a"].contains(&Tab::Browser {
        id: String::new(),
        target: BrowserTarget::Url("https://example.com/app".into())
    }));
    assert_eq!(app.layouts["a"].version, 6);
    assert!(app.state.sessions.is_empty());
    assert!(
        app.open_browser_url("a", "javascript:alert(1)", None)
            .is_err()
    );
    assert!(
        app.open_browser_url("a", "file:///tmp/x.html", None)
            .is_err()
    );
}

#[test]
fn browser_url_submit_replaces_tab_target() {
    let (mut app, ctx, _dir) = fixture();
    app.open_browser_url("a", "https://example.com/app", None)
        .unwrap();
    pump_until(&mut app, &ctx, |app| {
        matches!(app.layouts["a"].active_pane(), Some(Tab::Browser { .. }))
    });
    app.selected = Some("a".into());
    let old = app.layouts["a"].active_pane().unwrap().clone();
    app.browser_submit = Some((
        old.key(),
        BrowserTarget::from_http_url("https://example.com/other").unwrap(),
    ));
    app.apply_browser_submit();
    assert!(app.layouts["a"].contains(&Tab::Browser {
        id: String::new(),
        target: BrowserTarget::from_http_url("https://example.com/other").unwrap()
    }));
    assert!(!app.layouts["a"].contains(&old));
}
