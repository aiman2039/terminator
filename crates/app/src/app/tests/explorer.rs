use super::*;

#[test]
#[cfg(feature = "test-support")]
fn explorer_toolbar_controls_share_one_row() {
    let (mut app, ctx, _dir) = fixture();
    app.preferences.tool = SidebarTool::Explorer;
    app.preferences.explorer_search_mode = crate::preferences::ExplorerSearchMode::Contents;
    let cwd = std::path::PathBuf::from("/a");
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(360.0, 220.0),
            )),
            ..Default::default()
        },
        |ui| app.explorer_toolbar(ui, &cwd),
    );
    output.textures_delta.clear();
    for names in [
        [
            "explorer-new-file",
            "explorer-new-folder",
            "explorer-collapse",
            "explorer-refresh",
            "explorer-show-ignored",
            "explorer-more",
        ]
        .as_slice(),
        [
            "explorer-search",
            "explorer-match-case",
            "explorer-whole-word",
            "explorer-regex",
        ]
        .as_slice(),
    ] {
        let mut centers = Vec::new();
        for name in names {
            let rect = app
                .fixture_rect(&ctx, name)
                .unwrap_or_else(|| panic!("missing {name}"));
            centers.push((name, rect[1] + rect[3] / 2.0, rect[3]));
        }
        let (_, base, base_h) = centers[0];
        for (name, center, height) in &centers {
            assert!(
                (center - base).abs() < 1.0,
                "{name} center {center} should equal {base}"
            );
            assert!(
                (height - base_h).abs() < 1.0,
                "{name} height {height} should equal {base_h}"
            );
        }
    }
    let icons = app
        .fixture_rect(&ctx, "explorer-new-file")
        .expect("new file");
    let search = app.fixture_rect(&ctx, "explorer-search").expect("search");
    assert!(
        search[1] < icons[1] + icons[3] + 12.0,
        "search row stays under the toolbar, icons={} search={}",
        icons[1],
        search[1]
    );
}

#[test]
#[cfg(feature = "test-support")]
fn contents_search_queues_a_query() {
    let (mut app, ctx, _dir) = fixture();
    app.preferences.tool = SidebarTool::Explorer;
    app.preferences.explorer_search_mode = crate::preferences::ExplorerSearchMode::Contents;
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    let cwd = std::path::PathBuf::from("/a");
    let draw = |app: &mut App| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(360.0, 220.0),
                )),
                ..Default::default()
            },
            |ui| app.explorer_toolbar(ui, &cwd),
        );
        output.textures_delta.clear();
    };
    draw(&mut app);
    assert!(requests.try_recv().is_err(), "empty query queued a search");
    app.explorer_query = "needle".into();
    draw(&mut app);
    let job = requests
        .try_recv()
        .expect("non-empty query queues a search");
    assert!(matches!(job, Job::Search { .. }));
}

#[test]
fn explorer_prepared_rows_reuse_listings_and_refresh_expansion_and_content() {
    let (mut app, _, _dir) = fixture();
    let root = PathBuf::from("/a");
    let folder = root.join("folder");
    app.dirs.insert(
        root.clone(),
        vec![
            terminator_git::Entry {
                path: folder.clone(),
                directory: true,
                ignored: false,
            },
            terminator_git::Entry {
                path: root.join("ignored.rs"),
                directory: false,
                ignored: true,
            },
        ],
    );
    app.dirs.insert(
        folder.clone(),
        vec![terminator_git::Entry {
            path: folder.join("file.rs"),
            directory: false,
            ignored: false,
        }],
    );
    let rows = app.explorer_rows(&root);
    assert_eq!(rows.len(), 1);
    assert!(std::sync::Arc::ptr_eq(&rows, &app.explorer_rows(&root)));
    app.expanded_dirs.insert(folder.clone());
    let expanded = app.explorer_rows(&root);
    assert_eq!(expanded.len(), 2);
    assert_eq!(expanded[1].depth, 1);
    app.visible_dirs.clear();
    assert!(std::sync::Arc::ptr_eq(&expanded, &app.explorer_rows(&root)));
    assert_eq!(app.visible_dirs, [root.clone(), folder.clone()]);
    app.preferences.show_ignored = true;
    assert_eq!(app.explorer_rows(&root).len(), 3);
    app.explorer_query = "missing".into();
    assert_eq!(app.explorer_rows(&root).len(), 1); // folders remain navigable
    app.explorer_query.clear();
    app.dirs.get_mut(&folder).unwrap()[0].path = folder.join("changed.rs");
    app.sidebar_cache.get_mut().clear_files();
    assert_eq!(app.explorer_rows(&root)[1].label, "changed.rs");
}

#[test]
fn reveal_current_file_opens_ancestors_and_shows_ignored_target() {
    let (mut app, ctx, _dir) = fixture();
    let root = PathBuf::from("/a");
    let src = root.join("src");
    let deep = src.join("deep");
    let file = deep.join("agents.rs");
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(egui_dock::DockState::new(vec![Tab::NativeEditor {
            path: file.clone(),
        }])),
    );
    app.selected = Some("a".into());
    app.active_session = None;
    app.non_terminal_selected = true;
    app.preferences.explorer_search_mode = ExplorerSearchMode::Contents;
    app.explorer_query = "different".into();
    app.dirs.insert(
        root.clone(),
        vec![terminator_git::Entry {
            path: src.clone(),
            directory: true,
            ignored: false,
        }],
    );
    app.dirs.insert(
        src.clone(),
        vec![terminator_git::Entry {
            path: deep.clone(),
            directory: true,
            ignored: true,
        }],
    );
    app.dirs.insert(
        deep.clone(),
        vec![terminator_git::Entry {
            path: file.clone(),
            directory: false,
            ignored: true,
        }],
    );
    let host = egui::Id::unique("left-explorer");
    assert!(app.reveal_current_file_in_explorer(host));
    assert!(app.expanded_dirs.contains(&src));
    assert!(app.expanded_dirs.contains(&deep));
    assert_eq!(
        app.preferences.explorer_search_mode,
        ExplorerSearchMode::Names
    );
    assert!(app.explorer_query.is_empty());
    assert!(app.explorer_rows(&root).iter().any(|row| row.path == file));
    assert!(
        app.explorer_reveal
            .as_ref()
            .is_some_and(|reveal| reveal.pending_scroll)
    );
    let mut long_root: Vec<terminator_git::Entry> = (0..40)
        .map(|index| terminator_git::Entry {
            path: root.join(format!("before-{index:02}.rs")),
            directory: false,
            ignored: false,
        })
        .collect();
    long_root.push(terminator_git::Entry {
        path: src.clone(),
        directory: true,
        ignored: false,
    });
    app.dirs.insert(root, long_root);
    app.sidebar_cache.get_mut().clear_files();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(500.0, 280.0),
            )),
            ..Default::default()
        },
        |ui| {
            app.explorer_reveal.as_mut().unwrap().host = ui.scope_id();
            app.sidebar_tool(SidebarTool::Explorer, ui);
        },
    );
    output.textures_delta.clear();
    assert!(!app.explorer_reveal.as_ref().unwrap().pending_scroll);
}

#[test]
fn reveal_is_unavailable_for_shell_or_file_outside_project() {
    let (mut app, _, _dir) = fixture();
    let root = PathBuf::from("/a");
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(egui_dock::DockState::new(vec![Tab::Terminal(
            "shell".into(),
        )])),
    );
    app.state
        .sessions
        .push(session_fixture("shell", SessionKind::Shell));
    app.active_session = Some("shell".into());
    assert!(app.explorer_reveal_candidate().is_none());
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(egui_dock::DockState::new(vec![Tab::NativeEditor {
            path: PathBuf::from("/outside/file.rs"),
        }])),
    );
    app.active_session = None;
    app.non_terminal_selected = true;
    assert!(app.explorer_reveal_candidate().is_none());
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(egui_dock::DockState::new(vec![Tab::NativeEditor {
            path: root.join("src/file.rs"),
        }])),
    );
    assert!(app.explorer_reveal_candidate().is_some());
}

#[test]
fn search_results_preserve_headers_and_row_heights_when_clipped() {
    let (mut app, ctx, _dir) = fixture();
    app.explorer_search = (0..50)
        .map(|i| search::Hit {
            path: PathBuf::from(format!("/a/{}.rs", i / 5)),
            line: i + 1,
            text: format!("hit {i}"),
        })
        .collect();
    let mut heights = Vec::new();
    for clipped in [false, true] {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(300.0);
            ui.set_clip_rect(if clipped {
                egui::Rect::NOTHING
            } else {
                egui::Rect::EVERYTHING
            });
            let start = ui.next_widget_position().y;
            app.explorer_results(ui, Path::new("/a"));
            heights.push(ui.next_widget_position().y - start);
        });
        output.textures_delta.clear();
    }
    assert_eq!(heights[0], heights[1]);
    assert!(heights[0] > 50.0 * 20.0); // includes all ten file headers
}

#[test]
fn accepted_filesystem_refresh_replaces_prepared_git_and_explorer_data() {
    let (mut app, ctx, _dir) = fixture();
    let root = PathBuf::from("/a");
    let context = services::ContextData {
        cwd: root.clone(),
        root: Some(root.clone()),
        perforce: None,
        git_dirs: vec![],
        branch: "main".into(),
        changes: vec![],
        decorations: HashMap::default(),
        stats: HashMap::default(),
        error: None,
    };
    app.dirs.insert(root.clone(), vec![]);
    let rows = app.explorer_rows(&root);
    app.sidebar_cache.get_mut().git =
        Some(std::sync::Arc::new(sidebar_ui::PreparedGit::new(&context)));
    app.refresh_generation = 7;
    app.refresh_request = Some(refresh::Request {
        cwd: root.clone(),
        generation: 7,
        directories: vec![root.clone()],
        include_stats: false,
    });
    app.update_tx
        .send(Update::Refresh(6, context.clone(), vec![], false))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(std::sync::Arc::ptr_eq(&rows, &app.explorer_rows(&root)));
    assert!(app.sidebar_cache.borrow().git.is_some());
    app.update_tx
        .send(Update::Refresh(
            7,
            context,
            vec![(
                root.clone(),
                Ok(vec![terminator_git::Entry {
                    path: root.join("new.rs"),
                    directory: false,
                    ignored: false,
                }]),
            )],
            false,
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(app.sidebar_cache.borrow().git.is_none());
    let updated = app.explorer_rows(&root);
    assert!(!std::sync::Arc::ptr_eq(&rows, &updated));
    assert_eq!(updated[0].label, "new.rs");
}

#[test]
fn explorer_click_on_a_dirty_html_file_opens_the_browser() {
    let (mut app, ctx, dir) = fixture();
    let path = dir.path().join("page.html");
    fs::write(&path, "<html></html>").unwrap();
    app.dirs.insert(
        dir.path().into(),
        vec![terminator_git::Entry {
            path: path.clone(),
            directory: false,
            ignored: false,
        }],
    );
    app.context = Some(services::ContextData {
        cwd: dir.path().into(),
        root: Some(dir.path().into()),
        perforce: None,
        git_dirs: vec![],
        branch: "main".into(),
        changes: vec![terminator_git::Change {
            path: path.clone(),
            status: " M".into(),
        }],
        decorations: std::collections::HashMap::from([(path.clone(), 'M')]),
        stats: HashMap::default(),
        error: None,
    });
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    let mut draw = |events| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(260.0, 80.0),
                )),
                events,
                ..Default::default()
            },
            |ui| app.tree(ui, dir.path(), 0),
        );
        output.textures_delta.clear();
    };
    draw(vec![]);
    let pos = egui::pos2(55.0, 12.0);
    draw(vec![egui::Event::PointerMoved(pos)]);
    for pressed in [true, false] {
        draw(vec![egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        }]);
    }
    let expected = Tab::browser_file(std::path::absolute(&path).unwrap_or(path));
    // `process_updates` is budget-limited (2ms/64 items per call, like one
    // production frame) and drains background service updates before the
    // click's update. On a loaded runner the click can be deferred past a
    // single call, so pump until it lands instead of asserting after one.
    for _ in 0..100 {
        drain_updates(&mut app, &ctx);
        if app.layouts["a"].contains(&expected) {
            break;
        }
    }
    assert!(app.layouts["a"].contains(&expected));
    assert!(!requests.try_iter().any(|job| matches!(job, Job::Diff(_))));
}

#[test]
fn explorer_single_click_opens_an_editor_tab_across_the_entire_row() {
    for x in [12.0, 55.0, 230.0] {
        let (mut app, ctx, dir) = fixture();
        let path = dir.path().join(".gitkeep");
        fs::write(&path, "").unwrap();
        app.dirs.insert(
            dir.path().into(),
            vec![terminator_git::Entry {
                path: path.clone(),
                directory: false,
                ignored: false,
            }],
        );
        let (jobs, received) = mpsc::channel();
        app.jobs = jobs.into();
        let mut draw = |events| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(260.0, 80.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| app.tree(ui, dir.path(), 0),
            );
            output.textures_delta.clear();
        };
        draw(vec![]);
        let pos = egui::pos2(x, 12.0);
        draw(vec![egui::Event::PointerMoved(pos)]);
        for pressed in [true, false] {
            draw(vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            }]);
        }
        let Ok(Job::Control(request, After::Workspace(_, _))) = received.try_recv() else {
            panic!("A single click at x={x} should open an editor tab");
        };
        assert!(
            matches!(*request, Request::Create { file: Some(ref file), editor: true, ref project, .. } if file == &path && project == "a")
        );
        assert!(
            received.try_recv().is_err(),
            "One click should create only one tab"
        );
    }
}

#[test]
fn failed_directory_refresh_preserves_cached_entries_until_retry_succeeds() {
    let (mut app, ctx, _dir) = fixture();
    let path = PathBuf::from("/a");
    let cached = terminator_git::Entry {
        path: path.join("retained.rs"),
        directory: false,
        ignored: false,
    };
    app.dirs.insert(path.clone(), vec![cached]);
    app.refresh_generation = 7;
    app.refresh_request = Some(refresh::Request {
        cwd: path.clone(),
        generation: 7,
        directories: vec![path.clone()],
        include_stats: false,
    });
    let context = services::ContextData {
        cwd: path.clone(),
        root: None,
        perforce: None,
        git_dirs: vec![],
        branch: String::new(),
        changes: vec![],
        decorations: HashMap::default(),
        stats: HashMap::default(),
        error: None,
    };
    app.update_tx
        .send(Update::Refresh(
            7,
            context.clone(),
            vec![(
                path.clone(),
                Err(services::DirectoryError {
                    path: path.clone(),
                    kind: std::io::ErrorKind::PermissionDenied,
                    message: "Fixture access revoked".into(),
                }),
            )],
            false,
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(app.dirs[&path].len(), 1);
    assert!(app.directory_errors.contains_key(&path));
    app.update_tx
        .send(Update::Refresh(
            7,
            context,
            vec![(path.clone(), Ok(vec![]))],
            false,
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(app.dirs[&path].is_empty());
    assert!(!app.directory_errors.contains_key(&path));
}

#[test]
fn early_directory_preview_shows_entries_and_preserves_cached_ignores() {
    let (mut app, ctx, _dir) = fixture();
    let path = PathBuf::from("/a");
    app.refresh_generation = 7;
    app.refresh_request = Some(refresh::Request {
        cwd: path.clone(),
        generation: 7,
        directories: vec![path.clone()],
        include_stats: false,
    });
    let entry = terminator_git::Entry {
        path: path.join("ready.md"),
        directory: false,
        ignored: false,
    };
    app.update_tx
        .send(Update::DirectoryPreview(
            7,
            path.clone(),
            path.clone(),
            Ok(vec![entry.clone()]),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(app.dirs[&path][0].path, entry.path);
    assert!(app.context.is_none());

    app.dirs.get_mut(&path).unwrap()[0].ignored = true;
    app.update_tx
        .send(Update::DirectoryPreview(
            7,
            path.clone(),
            path.clone(),
            Ok(vec![entry]),
        ))
        .unwrap();
    app.update_tx
        .send(Update::DirectoryPreview(
            6,
            path.clone(),
            path.join("stale"),
            Ok(vec![]),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(app.dirs[&path][0].ignored);
    assert!(!app.dirs.contains_key(&path.join("stale")));
}

#[test]
fn stale_refresh_is_ignored_even_for_same_directory() {
    let (mut app, ctx, _dir) = fixture();
    app.refresh_generation = 3;
    app.refresh_request = Some(refresh::Request {
        cwd: "/a".into(),
        generation: 3,
        directories: vec![],
        include_stats: false,
    });
    app.update_tx
        .send(Update::Refresh(
            2,
            services::ContextData {
                cwd: "/a".into(),
                root: None,
                perforce: None,
                git_dirs: vec![],
                branch: "stale".into(),
                changes: vec![],
                decorations: HashMap::default(),
                stats: HashMap::default(),
                error: None,
            },
            vec![],
            false,
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(app.context.is_none());
}
