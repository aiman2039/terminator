use super::*;

#[test]
fn floatable_excludes_browser_and_player() {
    assert!(App::floatable(&Tab::Terminal("one".into())));
    assert!(App::floatable(&Tab::NativeEditor {
        path: "/a/note.md".into()
    }));
    assert!(App::floatable(&Tab::Diff {
        remote: None,
        cwd: "/a".into(),
        path: "/a/note.md".into(),
        staged: false,
    }));
    assert!(!App::floatable(&Tab::browser_file("/a/page.html".into())));
    assert!(!App::floatable(&Tab::Player));
}

#[test]
fn float_split_layout_docks_back_whole() {
    let (mut app, _, _dir) = fixture();
    let first = Tab::Terminal("float-one".into());
    let second = Tab::Terminal("float-two".into());
    let mut dock = DockState::new(vec![first.clone()]);
    dock.main_surface_mut()
        .split_right(egui_dock::NodeIndex::root(), 0.5, vec![second.clone()]);
    app.layouts.insert("a".into(), Workspace::empty());
    let home = app.layouts["a"].tabs[0].id.clone();
    // Closing the window adopts the whole split layout instead of
    // scattering its panes: the empty tab is reused, both panes
    // arrive with their split.
    app.dock_back_window("a".into(), home, dock);
    let workspace = app.layouts.get("a").expect("workspace");
    assert_eq!(workspace.tabs.len(), 1);
    let panes = workspace.tabs[0]
        .layout
        .iter_all_tabs()
        .map(|(_, pane)| pane.clone())
        .collect::<Vec<_>>();
    assert!(panes.contains(&first));
    assert!(panes.contains(&second));
    assert_eq!(workspace.tabs[0].layout.iter_leaves().count(), 2);
}

#[test]
fn float_created_lands_in_originating_window() {
    let (mut app, ctx, _dir) = fixture();
    let old = Tab::Terminal("float-old".into());
    app.floating
        .push(float_window("float-origin", "a", "home", vec![old.clone()]));
    let viewport = app.floating[0].viewport;
    let mut session = session_fixture("float-new", SessionKind::Shell);
    session.project_id = "a".into();
    // The user selected another project while the shell started.
    app.selected = Some("b".into());
    app.update_tx
        .send(Update::FloatCreated(
            session,
            viewport,
            Some("right".into()),
            vec![old.clone()],
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    // The new shell splits the originating window, not the newly
    // selected project; nothing falls back to the main dock.
    let dock = app.floating[0].dock.as_ref().expect("window kept");
    assert_eq!(dock.iter_all_tabs().count(), 2);
    assert!(dock.find_tab(&Tab::Terminal("float-new".into())).is_some());
    assert!(app.layouts.get("a").is_none_or(|workspace| {
        workspace
            .find_tab(&Tab::Terminal("float-new".into()))
            .is_none()
    }));
    assert_eq!(app.active_session.as_deref(), Some("float-new"));
}

#[test]
fn float_created_falls_back_to_project_dock() {
    let (mut app, ctx, _dir) = fixture();
    let mut session = session_fixture("float-gone", SessionKind::Shell);
    session.project_id = "a".into();
    app.selected = Some("a".into());
    let viewport = egui::ViewportId::from_hash_of("float-closed");
    app.update_tx
        .send(Update::FloatCreated(session, viewport, None, Vec::new()))
        .unwrap();
    drain_updates(&mut app, &ctx);
    // The window closed mid-flight: the tab still lands in its
    // project instead of nowhere.
    let workspace = app.layouts.get("a").expect("project dock");
    assert!(
        workspace
            .find_tab(&Tab::Terminal("float-gone".into()))
            .is_some()
    );
}

#[test]
fn float_native_split_stays_in_window() {
    let (mut app, _, _dir) = fixture();
    let path = PathBuf::from("/a/float-split.md");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::dirty_for_test(path.clone()),
    );
    let editor = Tab::NativeEditor { path: path.clone() };
    app.floating.push(float_window(
        "float-split",
        "a",
        "home",
        vec![editor.clone()],
    ));
    let viewport = app.floating[0].viewport;
    // `:vsplit` typed in the floating window defers through the same
    // drain as main-dock splits but lands in the issuing window.
    app.pending_native_splits.push((
        viewport,
        path.clone(),
        crate::native_editor::NativeSplit::Beside,
    ));
    app.drain_pending_native_close();
    let dock = app.floating[0].dock.as_ref().expect("window kept");
    assert_eq!(dock.iter_all_tabs().count(), 2);
    assert!(app.layouts.get("a").is_none_or(|workspace| {
        workspace
            .find_tab(&Tab::NativeEditor { path: path.clone() })
            .is_none()
    }));
}

#[test]
fn float_plain_close_takes_issuing_copy_only() {
    let (mut app, _, _dir) = fixture();
    let path = PathBuf::from("/a/float-copy.md");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::dirty_for_test(path.clone()),
    );
    let editor = || Tab::NativeEditor { path: path.clone() };
    let mut dock = DockState::new(vec![editor()]);
    let [_, right] =
        dock.main_surface_mut()
            .split_right(egui_dock::NodeIndex::root(), 0.5, vec![editor()]);
    app.floating
        .push(float_window("float-copies", "a", "home", vec![]));
    app.floating[0].dock = Some(dock);
    let viewport = app.floating[0].viewport;
    // `:q` in the right split removes that copy: the left keeps the
    // file and its buffer.
    let issuer = crate::native_editor::CloseIssuer::Floating {
        viewport,
        node: Some(egui_dock::NodePath {
            surface: egui_dock::SurfaceIndex::main(),
            node: right,
        }),
        delayed: false,
    };
    app.close_native_tab(&path, false, Some(issuer));
    let dock = app.floating[0].dock.as_ref().expect("window kept");
    assert_eq!(dock.iter_all_tabs().count(), 1);
    assert!(app.native_docs.contains_key(&path));
}

#[test]
fn float_stale_delayed_wq_closes_nothing() {
    let (mut app, _, _dir) = fixture();
    let path = PathBuf::from("/a/float-stale.md");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::dirty_for_test(path.clone()),
    );
    let editor = || Tab::NativeEditor { path: path.clone() };
    let mut dock = DockState::new(vec![editor()]);
    dock.main_surface_mut()
        .split_right(egui_dock::NodeIndex::root(), 0.5, vec![editor()]);
    app.floating
        .push(float_window("float-stale", "a", "home", vec![]));
    app.floating[0].dock = Some(dock);
    let viewport = app.floating[0].viewport;
    // The issuing view closed while the `:wq` save settled: no
    // fallback may take the surviving copy.
    let issuer = crate::native_editor::CloseIssuer::Floating {
        viewport,
        node: Some(egui_dock::NodePath {
            surface: egui_dock::SurfaceIndex::main(),
            node: egui_dock::NodeIndex(999),
        }),
        delayed: true,
    };
    app.close_native_tab(&path, false, Some(issuer));
    let dock = app.floating[0].dock.as_ref().expect("window kept");
    assert_eq!(dock.iter_all_tabs().count(), 2);
    assert!(app.native_docs.contains_key(&path));
}

#[test]
fn remove_tab_drops_emptied_float_window() {
    let (mut app, _, _dir) = fixture();
    app.state
        .sessions
        .push(session_fixture("float-solo", SessionKind::Shell));
    app.floating.push(float_window(
        "float-solo",
        "a",
        "home",
        vec![Tab::Terminal("float-solo".into())],
    ));
    app.remove_tab("float-solo");
    assert!(app.floating.is_empty());
}

#[test]
fn modal_choice_carries_issuing_float_pane() {
    use crate::native_editor::{CloseIssuer, NativeCloseChoice};
    let (mut app, _, _dir) = fixture();
    let path = PathBuf::from("/a/modal-float.md");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::dirty_for_test(path.clone()),
    );
    let editor = || Tab::NativeEditor { path: path.clone() };
    let mut dock = DockState::new(vec![editor()]);
    let [_, right] =
        dock.main_surface_mut()
            .split_right(egui_dock::NodeIndex::root(), 0.5, vec![editor()]);
    app.floating
        .push(float_window("modal-float", "a", "home", vec![]));
    app.floating[0].dock = Some(dock);
    let viewport = app.floating[0].viewport;
    let issuer = CloseIssuer::Floating {
        viewport,
        node: Some(egui_dock::NodePath {
            surface: egui_dock::SurfaceIndex::main(),
            node: right,
        }),
        delayed: false,
    };
    // Discard closes only the issuing copy: the left split keeps
    // the file and its buffer.
    app.apply_native_close_choice(&path, NativeCloseChoice::Discard, Some(issuer));
    let dock = app.floating[0].dock.as_ref().expect("window kept");
    assert_eq!(dock.iter_all_tabs().count(), 1);
    assert!(app.native_docs.contains_key(&path));
}

#[test]
fn modal_save_close_stores_float_issuer_deferred() {
    use crate::native_editor::{CloseIssuer, NativeCloseChoice};
    let (mut app, _, _dir) = fixture();
    let path = PathBuf::from("/a/modal-save-float.md");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::dirty_for_test(path.clone()),
    );
    // Save-and-close records the deferred issuer so the settled save
    // closes the issuing float copy instead of every docked copy.
    let issuer = CloseIssuer::Floating {
        viewport: egui::ViewportId::from_hash_of("modal-save-float"),
        node: None,
        delayed: false,
    };
    app.apply_native_close_choice(&path, NativeCloseChoice::SaveClose, Some(issuer));
    assert_eq!(app.native_close_after_save.as_deref(), Some(path.as_path()));
    assert!(matches!(
        app.native_close_after_save_issuer,
        Some(CloseIssuer::Floating { delayed: true, .. })
    ));
}

#[test]
fn float_delayed_close_with_live_pane_closes_only_it() {
    let (mut app, _, _dir) = fixture();
    let path = PathBuf::from("/a/float-live-delayed.md");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::dirty_for_test(path.clone()),
    );
    let editor = || Tab::NativeEditor { path: path.clone() };
    let mut dock = DockState::new(vec![editor()]);
    let [_, right] =
        dock.main_surface_mut()
            .split_right(egui_dock::NodeIndex::root(), 0.5, vec![editor()]);
    app.floating
        .push(float_window("float-live-delayed", "a", "home", vec![]));
    app.floating[0].dock = Some(dock);
    let viewport = app.floating[0].viewport;
    // A settled `:wq`-style close names a live pane: exactly that
    // copy goes, never a fallback, never both.
    let issuer = crate::native_editor::CloseIssuer::Floating {
        viewport,
        node: Some(egui_dock::NodePath {
            surface: egui_dock::SurfaceIndex::main(),
            node: right,
        }),
        delayed: true,
    };
    app.close_native_tab(&path, false, Some(issuer));
    let dock = app.floating[0].dock.as_ref().expect("window kept");
    assert_eq!(dock.iter_all_tabs().count(), 1);
    assert!(app.native_docs.contains_key(&path));
}

#[test]
fn float_terminal_ignores_center_overlays() {
    let (mut app, _, _dir) = fixture();
    app.state
        .sessions
        .push(session_fixture("float-gated", SessionKind::Shell));
    // Settings and Player cover the main window, never a float.
    app.settings_open = true;
    app.player_open = true;
    let viewport = egui::ViewportId::from_hash_of("float-gated");
    let float_input = {
        let viewer = crate::workspace_ui::Viewer {
            app: &mut app,
            strip: false,
            project: Some("a".into()),
            tab: None,
            window: Some(viewport),
            render_path: None,
            render_tab: None,
        };
        viewer.terminal_input_for("float-gated")
    };
    let main_input = {
        let viewer = crate::workspace_ui::Viewer {
            app: &mut app,
            strip: false,
            project: Some("a".into()),
            tab: Some("tab".into()),
            window: None,
            render_path: None,
            render_tab: None,
        };
        viewer.terminal_input_for("float-gated")
    };
    assert!(float_input);
    assert!(!main_input);
    // A true modal suspends every terminal.
    app.close_session = Some("other".into());
    assert!(!app.float_terminal_input_enabled("float-gated"));
}

#[test]
fn go_session_keeps_floated_terminal() {
    let (mut app, _, _dir) = fixture();
    let mut session = session_fixture("float-agent", SessionKind::Shell);
    session.project_id = "a".into();
    app.state.sessions.push(session);
    app.state.projects.push(terminator_core::Project {
        id: "a".into(),
        name: "a".into(),
        path: PathBuf::from("/a"),
        layout: serde_json::Value::Null,
    });
    app.floating.push(float_window(
        "float-agent",
        "a",
        "home",
        vec![Tab::Terminal("float-agent".into())],
    ));
    app.go_session("float-agent");
    // No duplicate tab in the main dock; the window keeps the view.
    assert!(app.layouts.get("a").is_none_or(|workspace| {
        workspace
            .find_tab(&Tab::Terminal("float-agent".into()))
            .is_none()
    }));
    assert_eq!(app.active_session.as_deref(), Some("float-agent"));
}

#[test]
fn float_pane_applies_to_window_and_docks_back() {
    let (mut app, _, _dir) = fixture();
    app.selected = Some("a".into());
    let workspace = Workspace::from_layout(DockState::new(vec![
        Tab::Terminal("one".into()),
        Tab::Terminal("two".into()),
    ]));
    app.layouts.insert("a".into(), workspace);
    app.float_pane = Some(Tab::Terminal("one".into()));
    let mut dock = app.layouts.remove("a").unwrap();
    app.apply_float_pane("a", &mut dock);
    app.layouts.insert("a".into(), dock);
    // The dock loses the pane; the window gains it with its home.
    assert!(
        app.layouts["a"]
            .find_tab(&Tab::Terminal("one".into()))
            .is_none()
    );
    assert!(
        app.layouts["a"]
            .find_tab(&Tab::Terminal("two".into()))
            .is_some()
    );
    assert_eq!(app.floating.len(), 1);
    assert_eq!(app.floating[0].home.0, "a");
    // Closing the window (or quitting) docks the pane back.
    app.dock_back_all_floating();
    assert!(app.floating.is_empty());
    assert!(
        app.layouts["a"]
            .find_tab(&Tab::Terminal("one".into()))
            .is_some()
    );
}

#[test]
fn prune_and_close_keep_floated_native_buffers() {
    let (mut app, _, _dir) = fixture();
    let path = PathBuf::from("/a/floated.md");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::dirty_for_test(path.clone()),
    );
    assert!(app.native_dirty(&path));
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(DockState::new(vec![Tab::NativeEditor {
            path: path.clone(),
        }])),
    );
    app.floating.push(float_window(
        "float-native",
        "a",
        "home",
        vec![Tab::NativeEditor { path: path.clone() }],
    ));
    // A plain close issued from the dock removes that copy and keeps
    // the floated buffer: the floating window stays open.
    let tab = app.layouts.get("a").expect("workspace").active.clone();
    app.close_native_tab(&path, false, docked_issuer("a", &tab, None, false));
    assert_eq!(app.floating.len(), 1);
    assert!(
        app.native_docs.contains_key(&path),
        "floated buffer survives docked close"
    );
    // Pruning with only the float left still keeps it: dropping it
    // here would reload from disk and lose unsaved changes.
    app.prune_native_docs();
    assert!(
        app.native_docs.contains_key(&path),
        "floated buffer survives prune"
    );
}

#[test]
fn force_close_takes_floating_views_and_buffers() {
    let (mut app, _, _dir) = fixture();
    let path = PathBuf::from("/a/forced.md");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::dirty_for_test(path.clone()),
    );
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(DockState::new(vec![Tab::NativeEditor {
            path: path.clone(),
        }])),
    );
    app.floating.push(float_window(
        "float-forced",
        "a",
        "home",
        vec![Tab::NativeEditor { path: path.clone() }],
    ));
    // `:q!` closes every view of the file, floating included.
    app.close_native_tab(&path, true, None);
    assert!(app.floating.is_empty());
    assert!(!app.native_docs.contains_key(&path));
}

#[test]
fn plain_close_takes_issuing_float_and_keeps_other_views() {
    let (mut app, _, _dir) = fixture();
    let path = PathBuf::from("/a/issuing.md");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::dirty_for_test(path.clone()),
    );
    let first = egui::ViewportId::from_hash_of("float-first");
    let second = egui::ViewportId::from_hash_of("float-second");
    for (name, viewport) in [("float-first", first), ("float-second", second)] {
        let mut window = float_window(
            name,
            "a",
            "home",
            vec![Tab::NativeEditor { path: path.clone() }],
        );
        window.viewport = viewport;
        app.floating.push(window);
    }
    // `:q` from the first floating view closes that window only:
    // the second view — and its unsaved buffer — survive.
    let floating = |viewport| crate::native_editor::CloseIssuer::Floating {
        viewport,
        node: None,
        delayed: false,
    };
    app.close_native_tab(&path, false, Some(floating(first)));
    assert_eq!(app.floating.len(), 1);
    assert_eq!(app.floating[0].viewport, second);
    assert!(app.native_docs.contains_key(&path));
    // Closing the last view drops the buffer.
    app.close_native_tab(&path, false, Some(floating(second)));
    assert!(app.floating.is_empty());
    assert!(!app.native_docs.contains_key(&path));
}

#[test]
fn quit_all_clean_closes_floating_only_editors() {
    let (mut app, _, _dir) = fixture();
    let path = PathBuf::from("/a/qa-float.md");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::new(path.clone(), false),
    );
    assert!(!app.native_dirty(&path));
    app.floating.push(float_window(
        "float-qa",
        "a",
        "home",
        vec![Tab::NativeEditor { path: path.clone() }],
    ));
    // Plain `:qa` on clean buffers leaves no floating window and no
    // buffer behind, even with no workspace tab showing the file.
    app.pending_quit_all = Some(false);
    app.drain_pending_native_close();
    assert!(app.floating.is_empty());
    assert!(!app.native_docs.contains_key(&path));
}

#[test]
fn quit_all_force_closes_floating_only_editors() {
    let (mut app, _, _dir) = fixture();
    let path = PathBuf::from("/a/floated-only.md");
    app.native_docs.insert(
        path.clone(),
        crate::native_editor::NativeDoc::dirty_for_test(path.clone()),
    );
    app.floating.push(float_window(
        "float-only",
        "a",
        "home",
        vec![Tab::NativeEditor { path: path.clone() }],
    ));
    // `:qa!` leaves no floating window and no buffer behind, even
    // when no workspace tab shows the file.
    app.pending_quit_all = Some(true);
    app.drain_pending_native_close();
    assert!(app.floating.is_empty());
    assert!(!app.native_docs.contains_key(&path));
}

#[test]
fn floated_panes_seed_visibility_before_prune() {
    let (mut app, _, _dir) = fixture();
    app.selected = Some("a".into());
    app.floating.push(float_window(
        "float-terminal",
        "a",
        "home",
        vec![Tab::Terminal("one".into())],
    ));
    app.floating.push(float_window(
        "float-image",
        "a",
        "home",
        vec![Tab::Image {
            path: "/tmp/a.png".into(),
        }],
    ));
    let mut doc = session_fixture("doc", SessionKind::Editor);
    doc.file = Some("/a/doc.md".into());
    app.state.sessions.push(doc);
    app.floating.push(float_window(
        "float-doc",
        "a",
        "home",
        vec![Tab::Terminal("doc".into())],
    ));
    app.seed_floating_visibility();
    // The end-of-frame prunes keep exactly what these sets contain, so
    // a floated terminal keeps its backend instead of re-attaching
    // every frame, and floated images and markdown previews survive.
    assert!(app.visible_sessions.contains("one"));
    assert!(app.visible_sessions.contains("doc"));
    assert!(
        app.visible_images
            .contains(std::path::Path::new("/tmp/a.png"))
    );
    assert!(app.markdown.entries.contains_key("doc"));
    // Docked-but-unrendered sessions stay unmarked.
    assert!(!app.visible_sessions.contains("two"));
}

#[cfg(feature = "test-support")]
#[test]
fn caption_menu_floats_pane() {
    let (mut app, ctx, _dir) = fixture();
    app.selected = Some("a".into());
    app.state
        .sessions
        .push(session_fixture("one", SessionKind::Shell));
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())])),
    );
    let secondary = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Secondary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };
    combined_frame(&mut app, &ctx, vec![]);
    let caption = frame_center(&app, &ctx, "pane-caption:one");
    combined_frame(&mut app, &ctx, vec![secondary(caption, true)]);
    combined_frame(&mut app, &ctx, vec![secondary(caption, false)]);
    combined_frame(&mut app, &ctx, vec![]);
    let item = frame_center(&app, &ctx, "Float window");
    combined_frame(&mut app, &ctx, vec![frame_press(item, true)]);
    combined_frame(&mut app, &ctx, vec![frame_press(item, false)]);
    assert_eq!(app.float_pane, Some(Tab::Terminal("one".into())));
}
