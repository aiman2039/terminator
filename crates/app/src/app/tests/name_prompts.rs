use super::*;

#[test]
fn explorer_prompts_do_not_suspend_terminal_input() {
    let (mut app, _, _dir) = fixture();
    app.open_name_prompt(workspace_ops::NamePrompt::File {
        dir: PathBuf::from("/a"),
        name: "a.txt".into(),
    });
    app.pending_delete = Some(PathBuf::from("/a/old"));
    assert!(app.terminal_input_enabled("shell"));
    assert!(app.strip_terminal_input_enabled("shell"));
    app.settings_open = true;
    assert!(!app.terminal_input_enabled("shell"));
    assert!(app.strip_terminal_input_enabled("shell"));
    app.settings_open = false;
    app.state
        .sessions
        .push(session_fixture("shell", SessionKind::Shell));
    app.rename_session = Some(("shell".into(), "shell".into()));
    assert!(!app.terminal_input_enabled("shell"));
    assert!(!app.strip_terminal_input_enabled("shell"));
}

#[test]
fn hidden_rename_field_releases_terminal_input() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.state.sessions = vec![session_fixture("named", SessionKind::Shell)];
    app.preferences.expanded.insert("a".into(), true);
    app.begin_rename("named", RenameSurface::Sidebar);
    app.rename_session.as_mut().unwrap().1 = "Draft".into();
    assert_rename_blocks(&app);

    app.preferences.left_visible = false;
    app.suspend_hidden_rename(&ctx);
    assert_rename_draft_stays_live(&app);
    app.preferences.left_visible = true;
    assert_rename_blocks(&app);

    app.preferences.left_agents = true;
    assert_rename_draft_stays_live(&app);
    app.preferences.left_agents = false;
    assert_rename_blocks(&app);

    app.preferences.expanded.insert("a".into(), false);
    assert_rename_draft_stays_live(&app);
    app.preferences.expanded.insert("a".into(), true);
    assert_rename_blocks(&app);

    app.preferences.hidden_projects.insert("a".into());
    assert_rename_draft_stays_live(&app);
    app.preferences.hidden_projects.remove("a");
    assert_rename_blocks(&app);

    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.projects(ui));
    output.textures_delta.clear();
    assert!(app.rename_painted);
    assert!(ctx.memory(|memory| memory.focused().is_some()));
    app.preferences.left_visible = false;
    app.suspend_hidden_rename(&ctx);
    assert!(ctx.memory(|memory| memory.focused().is_none()));
    assert_rename_draft_stays_live(&app);
    app.preferences.left_visible = true;

    app.begin_rename("named", RenameSurface::Pane);
    app.rename_session.as_mut().unwrap().1 = "Draft".into();
    assert_rename_blocks(&app);
    let covers: [fn(&mut App); 5] = [
        |app| app.settings_open = true,
        |app| app.player_open = true,
        |app| app.palette.open = true,
        |app| app.worktree_open = true,
        |app| app.search_open = true,
    ];
    for cover in covers {
        cover(&mut app);
        app.suspend_hidden_rename(&ctx);
        assert_eq!(
            app.rename_session.as_ref().map(|(_, title)| title.as_str()),
            Some("Draft")
        );
        assert!(app.strip_terminal_input_enabled("named"));
        app.settings_open = false;
        app.player_open = false;
        app.palette.open = false;
        app.worktree_open = false;
        app.search_open = false;
        assert_rename_blocks(&app);
    }
    app.worktree_draft = Some(worktree_ui::WorktreeDraft {
        source: "a".into(),
        start: "HEAD".into(),
        branch: "task".into(),
        dest: "/tmp/task".into(),
        open_terminal: true,
    });
    assert!(app.strip_terminal_input_enabled("named"));
    app.worktree_draft = None;
    assert_rename_blocks(&app);

    app.preferences.ide_mode = true;
    app.preferences.ide_strip_docks.0.insert(
        "a".into(),
        egui_dock::DockState::new(vec![Tab::Terminal("named".into())]),
    );
    app.settings_open = true;
    assert!(!app.strip_terminal_input_enabled("named"));
    app.preferences.ide_terminal_collapsed = true;
    assert!(app.strip_terminal_input_enabled("named"));
    app.settings_open = false;
    app.preferences.ide_mode = false;
    app.preferences.ide_terminal_collapsed = false;
    app.preferences.ide_strip_docks.0.clear();

    let mut ended = session_fixture("named", SessionKind::Shell);
    ended.lifecycle = Lifecycle::Ended;
    app.state.sessions = vec![ended];
    app.state.agents = vec![Agent {
        invocation_id: "agent-1".into(),
        session_id: "named".into(),
        kind: "codex".into(),
        provider_session_id: Some("provider-1".into()),
        state: AgentState::Stopped,
        sequence: None,
        updated: 0,
        resume: Some(Resume {
            program: "codex".into(),
            args: vec!["resume".into(), "provider-1".into()],
        }),
        process: None,
    }];
    app.preferences.tool = SidebarTool::History;
    app.preferences.visible = true;
    app.begin_rename("named", RenameSurface::Sidebar);
    app.rename_session.as_mut().unwrap().1 = "Draft".into();
    assert_rename_blocks(&app);
    app.preferences.visible = false;
    assert_rename_draft_stays_live(&app);
    app.preferences.visible = true;
    assert_rename_blocks(&app);
    app.preferences.tool = SidebarTool::Agents;
    assert_rename_draft_stays_live(&app);
    app.preferences.tool = SidebarTool::History;
    assert_rename_blocks(&app);
    app.preferences.history_expanded.insert("a".into(), false);
    assert_rename_draft_stays_live(&app);
    app.preferences.history_expanded.insert("a".into(), true);
    assert_rename_blocks(&app);
    app.preferences.history_filter = "zzz".into();
    assert_rename_draft_stays_live(&app);
    app.preferences.history_filter.clear();
    assert_rename_blocks(&app);

    app.state.sessions = vec![session_fixture("named", SessionKind::Shell)];
    app.begin_rename("named", RenameSurface::Workspace);
    app.rename_session.as_mut().unwrap().1 = "Draft".into();
    assert_rename_blocks(&app);
    app.note_rename_frame(&ctx);
    assert_rename_blocks(&app);
    app.note_rename_frame(&ctx);
    assert_rename_draft_stays_live(&app);
    while requests.try_recv().is_ok() {}
    let rect = egui::Rect::from_min_size(egui::pos2(8.0, 8.0), egui::vec2(220.0, 24.0));
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(260.0, 80.0),
            )),
            ..Default::default()
        },
        |ui| {
            app.inline_rename(crate::workspace_ui::InlineRenameInput {
                ui,
                sid: "named",
                surface: RenameSurface::Workspace,
                rect,
            })
        },
    );
    output.textures_delta.clear();
    app.note_rename_frame(&ctx);
    assert_rename_blocks(&app);
    assert!(requests.try_recv().is_err());
}

#[test]
fn name_prompt_takes_keys_until_a_terminal_press() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.name_prompt = Some(workspace_ops::NamePrompt::File {
        dir: PathBuf::from("/a"),
        name: String::new(),
    });
    paint_explorer(&mut app, &ctx, vec![egui::Event::Text("nope".into())]);
    assert_eq!(prompt_name(&app), Some(""));
    assert_no_workspace_job(&requests);

    app.open_name_prompt(workspace_ops::NamePrompt::File {
        dir: PathBuf::from("/a"),
        name: String::new(),
    });
    paint_explorer(&mut app, &ctx, vec![]);
    assert_eq!(
        ctx.memory(eframe::egui::Memory::focused),
        app.name_prompt_widget
    );
    paint_explorer(&mut app, &ctx, vec![egui::Event::Text("a.txt".into())]);
    assert_eq!(prompt_name(&app), Some("a.txt"));
    assert_no_workspace_job(&requests);

    app.terminal_pressed(&ctx, "shell");
    assert_eq!(app.active_session.as_deref(), Some("shell"));
    assert_eq!(prompt_name(&app), Some("a.txt"));
    assert!(!app.name_prompt_focus);
    assert_ne!(
        ctx.memory(eframe::egui::Memory::focused),
        app.name_prompt_widget
    );
    paint_explorer(&mut app, &ctx, vec![egui::Event::Text("more".into())]);
    assert_eq!(prompt_name(&app), Some("a.txt"));
    assert_no_workspace_job(&requests);
}

#[test]
#[cfg(feature = "test-support")]
fn name_prompt_wraps_when_the_sidebar_narrows() {
    let (mut app, ctx, _dir) = fixture();
    let cwd = PathBuf::from("/a");
    let paint = |app: &mut App, width: f32, folder: bool| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 240.0),
                )),
                ..Default::default()
            },
            |ui| {
                let prompt = if folder {
                    workspace_ops::NamePrompt::Folder {
                        dir: cwd.clone(),
                        name: String::new(),
                    }
                } else {
                    workspace_ops::NamePrompt::File {
                        dir: cwd.clone(),
                        name: String::new(),
                    }
                };
                app.open_name_prompt(prompt);
                app.explorer_toolbar(ui, &cwd);
            },
        );
        output.textures_delta.clear();
    };
    let top = |app: &App, name: &str| {
        app.fixture_rect(&ctx, name)
            .unwrap_or_else(|| panic!("missing {name}"))[1]
    };
    paint(&mut app, 480.0, false);
    let field = top(&app, "explorer-name-field");
    let save = top(&app, "explorer-name-save");
    let cancel = top(&app, "explorer-name-cancel");
    assert!(
        (field - save).abs() < 2.0 && (field - cancel).abs() < 2.0,
        "wide sidebar keeps the name prompt on one row, field={field} save={save} cancel={cancel}"
    );
    paint(&mut app, 200.0, false);
    let field = top(&app, "explorer-name-field");
    let save = top(&app, "explorer-name-save");
    assert!(
        save > field + 8.0,
        "narrow sidebar wraps Save below the name field, field={field} save={save}"
    );

    paint(&mut app, 200.0, true);
    let field = top(&app, "explorer-name-field");
    let save = top(&app, "explorer-name-save");
    assert!(
        save > field + 8.0,
        "narrow sidebar wraps a new-folder prompt, field={field} save={save}"
    );
}

#[test]
#[cfg(feature = "test-support")]
fn singleline_placeholder_is_vertically_centered() {
    let (mut app, ctx, _dir) = fixture();
    let cwd = PathBuf::from("/a");
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
    let field = app
        .fixture_rect(&ctx, "explorer-search")
        .expect("search field");
    let field_center = field[1] + field[3] / 2.0;
    let glyph =
        placeholder_center(&output.shapes, "Find in folder").expect("Find in folder placeholder");
    output.textures_delta.clear();
    assert!(
        (glyph - field_center).abs() < 2.0,
        "placeholder center {glyph} should match the field center {field_center}"
    );
}

#[test]
fn name_prompt_enter_submits_and_cancel_clears() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.open_name_prompt(workspace_ops::NamePrompt::File {
        dir: PathBuf::from("/a"),
        name: String::new(),
    });
    paint_explorer(&mut app, &ctx, vec![]);
    paint_explorer(
        &mut app,
        &ctx,
        vec![
            egui::Event::Text("a.txt".into()),
            key_press(egui::Key::Enter),
        ],
    );
    assert!(app.name_prompt.is_none());
    let job = requests.try_recv().expect("enter submits the name");
    match job {
        Job::Workspace(root, workspace_ops::Op::CreateFile(path)) => {
            assert_eq!(root, PathBuf::from("/a"));
            assert_eq!(path, PathBuf::from("/a/a.txt"));
        }
        _ => panic!("enter did not queue a file create"),
    }

    app.open_name_prompt(workspace_ops::NamePrompt::Folder {
        dir: PathBuf::from("/a"),
        name: "dir".into(),
    });
    paint_explorer(&mut app, &ctx, vec![]);
    paint_explorer(&mut app, &ctx, vec![key_press(egui::Key::Tab)]);
    paint_explorer(&mut app, &ctx, vec![key_press(egui::Key::Tab)]);
    paint_explorer(&mut app, &ctx, vec![key_press(egui::Key::Space)]);
    assert!(app.name_prompt.is_none());
    assert_no_workspace_job(&requests);
}

#[test]
fn hidden_sidebar_keeps_explorer_prompts() {
    let (mut app, ctx, _dir) = fixture();
    app.name_prompt = Some(workspace_ops::NamePrompt::Rename {
        from: PathBuf::from("/a/old"),
        name: "kept".into(),
    });
    app.pending_delete = Some(PathBuf::from("/a/old"));
    for tool in [
        SidebarTool::Agents,
        SidebarTool::History,
        SidebarTool::Info,
        SidebarTool::Git,
    ] {
        app.preferences.tool = tool;
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
        assert_eq!(prompt_name(&app), Some("kept"));
        assert_eq!(app.pending_delete, Some(PathBuf::from("/a/old")));
        assert!(app.terminal_input_enabled("shell"));
    }
    app.preferences.visible = false;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        },
        |ui| app.workspace_sidebars(ui),
    );
    output.textures_delta.clear();
    assert_eq!(prompt_name(&app), Some("kept"));
    assert_eq!(app.pending_delete, Some(PathBuf::from("/a/old")));
}

#[test]
fn inline_rename_saves_with_enter_and_cancels_with_escape() {
    for surface in [
        RenameSurface::Workspace,
        RenameSurface::Pane,
        RenameSurface::Sidebar,
    ] {
        for save in [false, true] {
            let (mut app, ctx, _dir) = fixture();
            app.state
                .sessions
                .push(session_fixture("named", SessionKind::Shell));
            let (jobs, requests) = mpsc::channel();
            app.jobs = jobs.into();
            app.begin_rename("named", surface);
            let rect = egui::Rect::from_min_size(egui::pos2(8.0, 8.0), egui::vec2(220.0, 24.0));
            let mut frame = |events| {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(260.0, 80.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        app.inline_rename(crate::workspace_ui::InlineRenameInput {
                            ui,
                            sid: "named",
                            surface,
                            rect,
                        })
                    },
                );
                output.textures_delta.clear();
            };
            frame(vec![]);
            frame(vec![
                egui::Event::Text("New title".into()),
                egui::Event::Key {
                    key: if save {
                        egui::Key::Enter
                    } else {
                        egui::Key::Escape
                    },
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::default(),
                },
            ]);
            assert!(app.rename_session.is_none());
            assert!(ctx.input(|input| {
                !input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Text(_) | egui::Event::Key { .. }))
            }));
            if save {
                let Ok(Job::Control(request, _)) = requests.try_recv() else {
                    panic!("Inline rename should save")
                };
                assert!(
                    matches!(*request,Request::Rename {ref session,ref label} if session=="named"&&label=="New title")
                );
            } else {
                assert!(requests.try_recv().is_err());
            }
        }
    }
}

#[test]
fn rename_targets_the_requested_session() {
    let (mut app, _, _dir) = fixture();
    app.state
        .sessions
        .push(session_fixture("named", SessionKind::Shell));
    app.active_session = Some("different".into());
    app.begin_rename("named", RenameSurface::Sidebar);
    assert_eq!(app.rename_session, Some(("named".into(), "named".into())));
    assert!(app.rename_focus);
}
