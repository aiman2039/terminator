use super::*;

#[test]
fn invalid_saved_focus_reports_error_and_preserves_layout() {
    let (mut app, _, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    let workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
    let mut saved = sanitize_layout(serde_json::to_value(workspace).unwrap());
    saved["tabs"][0]["layout"]["surfaces"][0]["Main"]["focused_node"] = serde_json::json!(999);
    let mut state = app.state.clone();
    state.projects[0].layout = saved.clone();
    app.layouts.clear();
    app.apply_state(state);
    // Loading persisted input must not allow an invalid index to reach the GUI.
    app.layouts["a"].active_pane();
    assert!(app.error.as_ref().is_some_and(|e| e.contains("focus")));
    assert!(app.layout_readonly.contains("a"));
    app.save_layouts();
    assert_eq!(app.state.projects[0].layout, saved);
    assert!(
        !requests
            .try_iter()
            .any(|job| matches!(job, Job::SaveLayout(ref project, _, _) if project == "a"))
    );
}

#[test]
fn unknown_workspace_format_is_not_overwritten() {
    let (mut app, _, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.layouts.clear();
    let mut state = app.state.clone();
    state.projects[0].layout = serde_json::json!({"version":99});
    app.apply_state(state);
    app.save_layouts();
    assert!(app.layout_readonly.contains("a"));
    assert!(
        !requests
            .try_iter()
            .any(|job| matches!(job,Job::SaveLayout(ref project,_,_) if project=="a"))
    );
}

#[test]
fn save_layouts_skips_unknown_projects() {
    let (mut app, _, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.layouts.insert("ghost".into(), Workspace::empty());
    app.save_layouts();
    let Job::PrepareLayouts(_, layouts) = requests.try_recv().unwrap() else {
        panic!("expected prepared layouts")
    };
    assert!(layouts.iter().all(|(project, _)| project != "ghost"));
    assert!(layouts.iter().any(|(project, _)| project == "a"));
}

#[test]
fn idle_app_does_not_resave_unchanged_layouts() {
    let (mut app, _, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.save_layouts();
    assert!(
        matches!(requests.try_recv(), Ok(Job::PrepareLayouts(..))),
        "the first pass must prepare layouts"
    );
    app.save_layouts();
    assert!(
        !requests
            .try_iter()
            .any(|job| matches!(job, Job::PrepareLayouts(..))),
        "an unchanged workspace set must not be prepared again"
    );
}

#[test]
fn mutating_a_workspace_resaves_its_layout() {
    let (mut app, _, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.save_layouts();
    let _ = requests.try_recv();
    app.layouts
        .get_mut("a")
        .unwrap()
        .add("shell".into(), Tab::Terminal("s".into()));
    app.save_layouts();
    assert!(
        requests
            .try_iter()
            .any(|job| matches!(job, Job::PrepareLayouts(..))),
        "a changed workspace must be prepared again"
    );
}

#[test]
fn reconciling_empty_workspaces_does_not_change_them() {
    let (mut app, _, _dir) = fixture();
    let snapshot = |app: &App| -> HashMap<String, serde_json::Value> {
        app.layouts
            .iter()
            .map(|(id, workspace)| (id.clone(), serde_json::to_value(workspace).unwrap()))
            .collect()
    };
    let before = snapshot(&app);
    app.reconcile_gui_resources();
    app.reconcile_gui_resources();
    assert_eq!(snapshot(&app), before);
}

#[test]
fn snapshot_drops_layouts_for_removed_projects() {
    let (mut app, _, _dir) = fixture();
    app.layouts.insert("ghost".into(), Workspace::empty());
    app.layout_saved.insert("ghost".into(), "{}".into());
    app.layout_pending.insert("ghost".into(), "{}".into());
    app.layout_readonly.insert("ghost".into());
    app.selected = Some("ghost".into());
    let mut state = app.state.clone();
    state.revision += 1;
    app.apply_state(state);
    assert!(!app.layouts.contains_key("ghost"));
    assert!(!app.layout_saved.contains_key("ghost"));
    assert!(!app.layout_pending.contains_key("ghost"));
    assert!(!app.layout_readonly.contains("ghost"));
    assert_ne!(app.selected.as_deref(), Some("ghost"));
}

#[test]
fn project_round_trip_restores_split_tabs_and_focus() {
    let (mut app, _, _dir) = fixture();
    app.insert("a", Tab::Terminal("first".into()), None);
    app.insert("a", Tab::Terminal("focused".into()), Some("right"));
    app.select_project("b".into());
    app.insert("b", Tab::Terminal("other".into()), None);
    app.select_project("a".into());
    assert_eq!(app.active_session.as_deref(), Some("focused"));
    assert_eq!(app.layouts["a"].iter_all_tabs().count(), 2);
    assert_eq!(app.layouts["b"].iter_all_tabs().count(), 1);
}

#[test]
fn image_split_survives_layout_temporarily_owned_by_renderer() {
    let (mut app, ctx, _dir) = fixture();
    app.insert("a", Tab::Terminal("shell".into()), None);
    app.active_session = Some("shell".into());
    let mut dock = app.layouts.remove("a").unwrap();
    let path = dock
        .find_tab(&Tab::Terminal("shell".into()))
        .unwrap()
        .node_path();
    app.pane_by_tab
        .insert(Tab::Terminal("shell".into()).key(), path);
    app.pane_tabs
        .insert(path, vec![Tab::Terminal("shell".into())]);
    app.open_image("a", crate::test_path("/a/picture.png"), Some("right"));
    let original = dock.active.clone();
    dock.add("other".into(), Tab::Terminal("other".into()));
    app.layouts.insert("a".into(), dock);
    pump_until(&mut app, &ctx, |app| {
        app.layouts.get("a").is_some_and(|workspace| {
            workspace.tabs.iter().any(|tab| {
                tab.id == original
                    && tab
                        .layout
                        .find_tab(&Tab::Image {
                            path: crate::test_path("/a/picture.png"),
                        })
                        .is_some()
            })
        })
    });
    assert_eq!(app.layouts["a"].active, "other");
    let source = app.layouts["a"]
        .tabs
        .iter()
        .find(|t| t.id == original)
        .unwrap();
    assert!(
        source
            .layout
            .find_tab(&Tab::Image {
                path: crate::test_path("/a/picture.png")
            })
            .is_some()
    );
}

#[test]
fn pane_maps_reuse_unchanged_dock_and_refresh_on_change() {
    let (mut app, _, _dir) = fixture();
    app.insert("a", Tab::Terminal("shell".into()), None);
    let dock = app.layouts.get("a").cloned().unwrap();
    app.refresh_pane_maps("a", &dock);
    assert_eq!(app.pane_by_tab.len(), 1);
    // Same dock next frame: cleared maps staying empty proves no rebuild.
    app.pane_by_tab.clear();
    app.pane_tabs.clear();
    app.refresh_pane_maps("a", &dock);
    assert!(app.pane_by_tab.is_empty());
    assert!(app.pane_tabs.is_empty());
    // Structural change rebuilds the maps.
    let mut changed = dock.clone();
    changed.add("other".into(), Tab::Terminal("other".into()));
    app.refresh_pane_maps("a", &changed);
    assert!(
        app.pane_by_tab
            .contains_key(&Tab::Terminal("other".into()).key())
    );
}
