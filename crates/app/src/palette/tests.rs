use super::*;

#[test]
fn file_search_reaches_entries_after_the_empty_result_limit() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::with_context(&egui::Context::default(), Paths::at(dir.path().into()));
    app.state.projects.push(Project {
        id: "p".into(),
        name: "p".into(),
        path: dir.path().into(),
        layout: serde_json::Value::Null,
    });
    app.selected = Some("p".into());
    app.dirs.insert(
        dir.path().into(),
        (0..50)
            .map(|index| terminator_git::Entry {
                path: dir.path().join(format!("file-{index:02}.rs")),
                directory: false,
                ignored: false,
            })
            .collect(),
    );
    assert_eq!(
        app.filtered_palette()
            .iter()
            .filter(|item| matches!(item, PaletteItem::File(_)))
            .count(),
        40
    );
    app.palette.query = "file-49.rs".into();
    let matches = app.filtered_palette();
    assert!(
        matches
            .iter()
            .any(|item| matches!(item, PaletteItem::File(path) if path.ends_with("file-49.rs")))
    );
}

#[test]
fn fuzzy_index_ranks_go_to_file() {
    use terminator_native_edit::finder::FileIndex;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/main.rs"), "").unwrap();
    std::fs::write(root.join("src/lib.rs"), "").unwrap();
    let mut app = App::with_context(&egui::Context::default(), Paths::at(dir.path().into()));
    app.state.projects.push(Project {
        id: "p".into(),
        name: "p".into(),
        path: root.clone(),
        layout: serde_json::Value::Null,
    });
    app.selected = Some("p".into());
    app.file_index = FileIndex::scan(&[root], MAX_INDEX_FILES);
    app.palette.query = "main".into();
    let files: Vec<PathBuf> = app
        .filtered_palette()
        .into_iter()
        .filter_map(|item| match item {
            PaletteItem::File(path) => Some(path),
            _ => None,
        })
        .collect();
    assert_eq!(files.len(), 1);
    assert!(files[0].ends_with("src/main.rs"));
}

#[test]
fn background_refresh_delivers_an_index() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "").unwrap();
    let mut app = App::with_context(&egui::Context::default(), Paths::at(dir.path().into()));
    app.dirs.insert(
        dir.path().into(),
        vec![terminator_git::Entry {
            path: dir.path().join("a.rs"),
            directory: false,
            ignored: false,
        }],
    );
    app.refresh_file_index();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while app.file_index_rx.is_some() && std::time::Instant::now() < deadline {
        app.poll_file_index();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!app.file_index.is_empty());
    // A second refresh with unchanged roots and a fresh index is a no-op.
    app.refresh_file_index();
    assert!(app.file_index_rx.is_none());
}

#[test]
fn palette_labels_are_stable() {
    assert_eq!(PaletteItem::AddProject.label(), "Add project");
    assert_eq!(
        PaletteItem::Action(Action::ToggleIdeMode).label(),
        "Toggle IDE mode"
    );
    assert_eq!(
        PaletteItem::Action(Action::MoveToMain).label(),
        "Move terminal to main pane"
    );
    assert_eq!(
        PaletteItem::Action(Action::MoveToStrip).label(),
        "Move terminal to lower pane"
    );
    assert_eq!(
        PaletteItem::ToggleIdeSidebarHeight.label(),
        "Toggle IDE sidebar height"
    );
    assert_eq!(
        PaletteItem::Action(Action::NextAttention).label(),
        "Next agent needing attention"
    );
    assert_eq!(
        PaletteItem::Settings(SettingsSection::Terminal).label(),
        "Settings  Terminal & Editor"
    );
}

#[test]
fn palette_toggles_ide_sidebar_height() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    assert!(app.preferences.ide_sidebars_full_height);
    app.run_palette(&ctx, PaletteItem::ToggleIdeSidebarHeight);
    assert!(!app.preferences.ide_sidebars_full_height);
    app.run_palette(&ctx, PaletteItem::ToggleIdeSidebarHeight);
    assert!(app.preferences.ide_sidebars_full_height);
}

#[test]
fn fuzzy_list_finds_settings_keywords_and_shortcut_actions() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    app.palette.query = "font".into();
    assert!(app.filtered_palette().iter().any(|item| matches!(
        item,
        PaletteItem::Settings(SettingsSection::Appearance | SettingsSection::Terminal)
    )));
    app.palette.query = "open file".into();
    assert!(
        app.filtered_palette()
            .iter()
            .any(|item| matches!(item, PaletteItem::Action(Action::OpenFile)))
    );
}

#[test]
fn waiting_agent_is_a_direct_result_and_resolved_notice_disappears() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::with_context(&egui::Context::default(), Paths::at(dir.path().into()));
    app.state.projects.push(Project {
        id: "a".into(),
        name: "Alpha".into(),
        path: "/a".into(),
        layout: serde_json::Value::Null,
    });
    app.selected = Some("a".into());
    app.state.sessions.push(Session {
        remote: None,
        id: "shell".into(),
        project_id: "a".into(),
        label: "Agent terminal".into(),
        cwd: "/a".into(),
        kind: SessionKind::Shell,
        file: None,
        lifecycle: Lifecycle::Running,
        review: false,
        created: 1,
        exit_code: None,
        rows: 24,
        cols: 80,
        generation: "test".into(),
        pid: Some(1),
        truncated: false,
        cwd_confirmed: true,
    });
    app.state.notifications.push(Notification {
        id: "notice".into(),
        session_id: "shell".into(),
        invocation_id: "inv".into(),
        request_id: Some("request".into()),
        state: AgentState::WaitingPermission,
        summary: "Permission needed".into(),
        details: String::new(),
        created: 2,
        read: false,
        dismissed: false,
        resolved: false,
        snoozed_until: 0,
    });
    app.palette.query = "permission".into();
    assert!(
        app.filtered_palette()
            .iter()
            .any(|item| matches!(item, PaletteItem::Agent(id, _, _) if id == "shell"))
    );
    app.state.notifications[0].resolved = true;
    assert!(
        !app.filtered_palette()
            .iter()
            .any(|item| matches!(item, PaletteItem::Agent(..)))
    );
}

#[test]
fn file_and_symbol_results_open_in_their_own_project() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    for id in ["a", "b"] {
        app.state.projects.push(Project {
            id: id.into(),
            name: id.into(),
            path: dir.path().join(id),
            layout: serde_json::Value::Null,
        });
    }
    app.selected = Some("a".into());
    let file = dir.path().join("b/src/main.rs");
    app.run_palette(&ctx, PaletteItem::File(file.clone()));
    assert_eq!(app.selected.as_deref(), Some("b"));
    app.selected = Some("a".into());
    app.state.settings.editor_mode = EditorMode::Native;
    app.run_palette(
        &ctx,
        PaletteItem::Symbol {
            path: file.clone(),
            name: "helper".into(),
            kind: "Function".into(),
            line: 8,
            column: 4,
            container: None,
        },
    );
    assert_eq!(app.selected.as_deref(), Some("b"));
    assert_eq!(app.native_pending_line.get(&file), Some(&8));
    // The jump lands on the symbol name, not just its line.
    assert_eq!(app.native_pending_col.get(&file), Some(&4));
}

fn project_at(app: &mut App, id: &str, path: std::path::PathBuf) {
    app.state.projects.push(Project {
        id: id.into(),
        name: id.into(),
        path,
        layout: serde_json::Value::Null,
    });
}

#[test]
fn search_stays_inside_the_selected_project() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::with_context(&egui::Context::default(), Paths::at(dir.path().into()));
    for id in ["a", "b"] {
        let path = dir.path().join(id);
        std::fs::create_dir_all(&path).unwrap();
        project_at(&mut app, id, path.clone());
        app.dirs.insert(
            path.clone(),
            vec![terminator_git::Entry {
                path: path.join("main.rs"),
                directory: false,
                ignored: false,
            }],
        );
        app.state.sessions.push(Session {
            remote: None,
            id: id.into(),
            project_id: id.into(),
            label: format!("{id} shell"),
            cwd: path,
            kind: SessionKind::Shell,
            file: None,
            lifecycle: Lifecycle::Running,
            review: false,
            created: 1,
            exit_code: None,
            rows: 24,
            cols: 80,
            generation: "test".into(),
            pid: Some(1),
            truncated: false,
            cwd_confirmed: true,
        });
    }
    app.selected = Some("a".into());
    app.palette.query = "main".into();
    let files = app.filtered_palette();
    assert!(
        files
            .iter()
            .any(|item| matches!(item, PaletteItem::File(path) if path.ends_with("a/main.rs")))
    );
    assert!(
        !files
            .iter()
            .any(|item| matches!(item, PaletteItem::File(path) if path.ends_with("b/main.rs")))
    );
    app.palette.query = "shell".into();
    let sessions = app.filtered_palette();
    assert!(
        sessions
            .iter()
            .any(|item| matches!(item, PaletteItem::Session(id, _) if id == "a"))
    );
    assert!(
        !sessions
            .iter()
            .any(|item| matches!(item, PaletteItem::Session(id, _) if id == "b"))
    );
    app.palette.query.clear();
    assert!(
        !app.filtered_palette()
            .iter()
            .any(|item| matches!(item, PaletteItem::Project(..)))
    );
    app.palette.filters = super::FILTER_PROJECTS;
    let projects = app.filtered_palette();
    assert!(
        projects
            .iter()
            .any(|item| matches!(item, PaletteItem::Project(id, _) if id == "b"))
    );
    assert!(
        projects
            .iter()
            .all(|item| matches!(item, PaletteItem::Project(..)))
    );
}

#[test]
fn file_filter_drops_settings() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::with_context(&egui::Context::default(), Paths::at(dir.path().into()));
    project_at(&mut app, "p", dir.path().into());
    app.selected = Some("p".into());
    app.dirs.insert(
        dir.path().into(),
        vec![terminator_git::Entry {
            path: dir.path().join("font.rs"),
            directory: false,
            ignored: false,
        }],
    );
    app.palette.filters = super::FILTER_FILES;
    app.palette.query = "font".into();
    let items = app.filtered_palette();
    assert!(
        items
            .iter()
            .any(|item| matches!(item, PaletteItem::File(path) if path.ends_with("font.rs")))
    );
    assert!(
        items
            .iter()
            .all(|item| matches!(item, PaletteItem::File(_)))
    );
}

fn live_shell(id: &str, project: &str, cwd: std::path::PathBuf) -> Session {
    Session {
        remote: None,
        id: id.into(),
        project_id: project.into(),
        label: format!("{id} shell"),
        cwd,
        kind: SessionKind::Shell,
        file: None,
        lifecycle: Lifecycle::Running,
        review: false,
        created: 1,
        exit_code: None,
        rows: 24,
        cols: 80,
        generation: "test".into(),
        pid: Some(1),
        truncated: false,
        cwd_confirmed: true,
    }
}

#[test]
fn empty_search_leads_with_recent_files_then_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("proj");
    std::fs::create_dir_all(&root).unwrap();
    let mut app = App::with_context(&egui::Context::default(), Paths::at(dir.path().into()));
    project_at(&mut app, "p", root.clone());
    app.selected = Some("p".into());
    let recent = root.join("recent.rs");
    let other = root.join("other.rs");
    app.dirs.insert(
        root.clone(),
        vec![
            terminator_git::Entry {
                path: other.clone(),
                directory: false,
                ignored: false,
            },
            terminator_git::Entry {
                path: recent.clone(),
                directory: false,
                ignored: false,
            },
        ],
    );
    app.preferences.recent_native_files = vec![recent.clone()];
    app.state.sessions.push(live_shell("shell", "p", root));
    let items = app.filtered_palette();
    let recent_at = items
        .iter()
        .position(|item| matches!(item, PaletteItem::File(path) if path == &recent));
    let session_at = items
        .iter()
        .position(|item| matches!(item, PaletteItem::Session(id, _) if id == "shell"));
    let other_at = items
        .iter()
        .position(|item| matches!(item, PaletteItem::File(path) if path == &other));
    assert_eq!(
        (recent_at, session_at, other_at),
        (Some(0), Some(1), Some(2))
    );
    assert!(items.iter().all(|item| {
        matches!(
            item,
            PaletteItem::File(_) | PaletteItem::Session(..) | PaletteItem::Agent(..)
        )
    }));
    app.palette.filters = super::FILTER_ACTIONS;
    assert!(
        app.filtered_palette()
            .iter()
            .any(|item| item.filter_bit() == super::FILTER_ACTIONS)
    );
}

#[test]
fn nested_project_files_stay_in_that_project() {
    use terminator_native_edit::finder::FileIndex;
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().join("parent");
    let child = parent.join("child");
    std::fs::create_dir_all(&child).unwrap();
    std::fs::write(parent.join("own.rs"), "").unwrap();
    std::fs::write(child.join("own.rs"), "").unwrap();
    let mut app = App::with_context(&egui::Context::default(), Paths::at(dir.path().into()));
    project_at(&mut app, "parent", parent.clone());
    project_at(&mut app, "child", child.clone());
    app.file_index = FileIndex::scan(&[parent], MAX_INDEX_FILES);
    app.selected = Some("parent".into());
    app.palette.query = "own".into();
    let parent_hits = app.filtered_palette();
    assert!(
        parent_hits
            .iter()
            .any(|item| matches!(item, PaletteItem::File(path) if path.ends_with("parent/own.rs")))
    );
    assert!(
        !parent_hits
            .iter()
            .any(|item| matches!(item, PaletteItem::File(path) if path.ends_with("child/own.rs")))
    );
    app.selected = Some("child".into());
    let child_hits = app.filtered_palette();
    assert!(
        child_hits
            .iter()
            .any(|item| matches!(item, PaletteItem::File(path) if path.ends_with("child/own.rs")))
    );
    assert!(
        !child_hits
            .iter()
            .any(|item| matches!(item, PaletteItem::File(path) if path.ends_with("parent/own.rs")))
    );
}

fn outline_symbol(name: &str, depth: usize, line: u32) -> native_lsp::Symbol {
    let at = native_lsp::Position { line, character: 4 };
    native_lsp::Symbol {
        name: name.into(),
        kind: "Function".into(),
        range: native_lsp::Range { start: at, end: at },
        selection: native_lsp::Range { start: at, end: at },
        depth,
        detail: None,
        deprecated: false,
    }
}

fn symbol_names(items: &[PaletteItem]) -> Vec<String> {
    items
        .iter()
        .filter_map(|item| match item {
            PaletteItem::Symbol { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn symbol_search_ranks_on_names_not_file_paths() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::with_context(&egui::Context::default(), Paths::at(dir.path().into()));
    project_at(&mut app, "p", dir.path().into());
    app.selected = Some("p".into());
    app.lsp.set_symbols_for_test(
        &dir.path().join("main.rs"),
        vec![outline_symbol("helper", 0, 1)],
    );
    app.lsp.set_symbols_for_test(
        &dir.path().join("other.rs"),
        vec![outline_symbol("main_loop", 0, 3)],
    );
    app.palette.filters = FILTER_SYMBOLS;
    app.palette.query = "main".into();
    assert_eq!(symbol_names(&app.filtered_palette()), ["main_loop"]);
}

#[test]
fn file_structure_lists_the_scoped_outline_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::with_context(&egui::Context::default(), Paths::at(dir.path().into()));
    project_at(&mut app, "p", dir.path().into());
    app.selected = Some("p".into());
    let file = dir.path().join("lib.rs");
    app.lsp.set_symbols_for_test(
        &file,
        vec![
            outline_symbol("Point", 0, 0),
            outline_symbol("new", 1, 2),
            outline_symbol("parse", 0, 9),
        ],
    );
    app.lsp.set_symbols_for_test(
        &dir.path().join("other.rs"),
        vec![outline_symbol("elsewhere", 0, 0)],
    );
    app.palette.filters = FILTER_SYMBOLS;
    app.palette.file_scope = Some(file);
    assert_eq!(
        symbol_names(&app.filtered_palette()),
        ["Point", "   new", "parse"]
    );
    app.palette.query = "pa".into();
    assert_eq!(symbol_names(&app.filtered_palette()), ["parse"]);
}
