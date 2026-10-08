use super::super::appearance;
use eframe::egui::{self};
#[cfg(test)]
use egui_dock::DockState;
use std::{
    path::PathBuf,
    sync::mpsc::{self},
};
use terminator_core::*;

use super::super::*;
use super::nav_common::*;
#[cfg(test)]
mod tests {
    use super::*;
    /// The tab strip must sit below the 40px drag band (or press-and-move
    /// gestures on tabs move the whole window instead of reordering tabs)
    /// and beside the sidebars, which run full height underneath the band.
    #[test]
    #[cfg(feature = "test-support")]
    fn tab_strip_sits_below_the_drag_band_and_beside_full_height_sidebars() {
        let (mut app, ctx, _dir) = fixture();
        let workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        app.layouts.insert("a".into(), workspace);
        app.selected = Some("a".into());
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                // Same order as the real panels: drag band, sidebars,
                // then the center-only tab strip.
                egui::Panel::top("window-header")
                    .exact_size(40.0)
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| app.window_header(ui));
                assert!(
                    header_target(&ctx)("workspace-strip").is_none(),
                    "the drag band must not paint the tab strip"
                );
                egui::Panel::left("projects")
                    .exact_size(225.0)
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| {
                        ui.label("sidebar");
                    });
                egui::Panel::top("workspace-tabs")
                    .exact_size(36.0)
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| app.window_header_tabs(ui));
            },
        );
        output.textures_delta.clear();
        let strip = header_target(&ctx)("workspace-strip").expect("strip geometry");
        assert!(
            strip.top() >= 40.0,
            "tab strip must clear the native drag band, got {strip:?}"
        );
        assert!(
            strip.left() >= 225.0,
            "tab strip must sit beside the full-height sidebar, got {strip:?}"
        );
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn narrow_header_puts_search_behind_the_overflow_menu() {
        let (mut app, ctx, _dir) = fixture();
        app.preferences.width = UiPreferences::default().width;
        paint_header(&mut app, &ctx, &[]);
        let rect = header_target(&ctx);
        assert!(rect("tool-Info").is_some());
        assert!(rect("palette").is_none(), "search must leave the bar");
        assert!(rect("settings").is_some());
        let overflow = rect("header-overflow").expect("overflow menu");
        let pos = overflow.center();
        paint_header(&mut app, &ctx, &[egui::Event::PointerMoved(pos)]);
        paint_header(
            &mut app,
            &ctx,
            &[egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            }],
        );
        paint_header(
            &mut app,
            &ctx,
            &[egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            }],
        );
        // The opening frame is a sizing pass. Read the item after the menu settles.
        paint_header(&mut app, &ctx, &[]);
        let search = rect("palette").expect("search in the menu");
        assert!(!app.palette_open);
        let pos = search.center();
        paint_header(&mut app, &ctx, &[egui::Event::PointerMoved(pos)]);
        paint_header(
            &mut app,
            &ctx,
            &[egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::default(),
            }],
        );
        paint_header(
            &mut app,
            &ctx,
            &[egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::default(),
            }],
        );
        assert!(app.palette_open);
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn wide_header_shows_search_without_an_overflow_menu() {
        let (mut app, ctx, _dir) = fixture();
        app.preferences.width = 480.0;
        paint_header(&mut app, &ctx, &[]);
        let rect = header_target(&ctx);
        assert!(rect("palette").is_some());
        assert!(rect("header-overflow").is_none());
    }

    #[test]
    fn worktree_completion_opens_only_the_requested_projects_terminal() {
        for open_terminal in [false, true] {
            let (mut app, ctx, _dir) = fixture();
            let (jobs, requests) = mpsc::channel();
            app.jobs = jobs.into();
            let (updates, rx) = mpsc::channel();
            app.updates = rx;
            updates
                .send(Update::WorktreeCreated(
                    Box::new(app.state.clone()),
                    "b".into(),
                    open_terminal,
                ))
                .unwrap();
            app.process_updates(&ctx);
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
    fn command_dialogs_suspend_terminal_input_until_dismissed() {
        let (mut app, _, _dir) = fixture();
        assert!(app.terminal_input_enabled("shell"));
        app.palette_open = true;
        assert!(!app.terminal_input_enabled("shell"));
        app.palette_open = false;
        assert!(app.terminal_input_enabled("shell"));
        app.player_open = true;
        assert!(!app.terminal_input_enabled("shell"));
        app.player_open = false;
        assert!(app.terminal_input_enabled("shell"));
        app.worktree_draft = Some(worktree_ui::WorktreeDraft {
            source: "a".into(),
            start: "HEAD".into(),
            branch: "task".into(),
            dest: "/tmp/task".into(),
            open_terminal: true,
        });
        assert!(!app.terminal_input_enabled("shell"));
        app.worktree_draft = None;
        app.worktree_remove = Some("a".into());
        assert!(!app.terminal_input_enabled("shell"));
        app.worktree_remove = None;
        assert!(app.terminal_input_enabled("shell"));
        app.search_open = true;
        assert!(!app.terminal_input_enabled("shell"));
        app.search_open = false;
        assert!(app.terminal_input_enabled("shell"));
        app.worktree_open = true;
        assert!(!app.terminal_input_enabled("shell"));
        app.worktree_open = false;
        assert!(app.terminal_input_enabled("shell"));
    }

    #[test]
    fn closing_scrollback_search_restores_terminal_input() {
        let (mut app, _, _dir) = fixture();
        app.active_session = Some("live".into());
        app.search_active_scrollback(&egui::Context::default());
        assert!(app.search_open);
        assert_eq!(app.search_session.as_deref(), Some("live"));
        assert!(!app.terminal_input_enabled("live"));
        assert!(app.strip_terminal_input_enabled("live"));

        app.hide_center_overlay();
        assert!(!app.search_open);
        assert!(app.search_session.is_none());
        assert!(app.terminal_input_enabled("live"));
        assert!(app.strip_terminal_input_enabled("live"));

        // The session id is not itself a lock. A close that only cleared
        // `search_open` used to leave every main terminal disabled.
        app.search_session = Some("live".into());
        assert!(app.terminal_input_enabled("live"));
        app.search_open = true;
        assert!(!app.terminal_input_enabled("live"));
        app.close_scrollback_search();
        assert!(app.search_session.is_none());
        assert!(app.terminal_input_enabled("live"));
    }

    #[test]
    fn select_all_yields_to_focused_text_field() {
        let (app, ctx, _dir) = fixture();
        assert!(!app.text_input_focused(&ctx));
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let mut text = String::new();
            let response =
                ui.add(appearance::singleline(&mut text).id(egui::Id::new("select-all-probe")));
            response.request_focus();
        });
        output.textures_delta.clear();
        assert!(app.text_input_focused(&ctx));
    }

    #[test]
    fn select_all_yields_to_focused_native_editor() {
        let (mut app, ctx, _dir) = fixture();
        assert!(!app.text_input_focused(&ctx));
        let path = PathBuf::from("/a/file.rs");
        app.native_docs.insert(
            path.clone(),
            crate::native_editor::NativeDoc::new(path.clone(), false),
        );
        let id = terminator_native_edit::view::source_focus_id(&path.to_string_lossy());
        ctx.memory_mut(|memory| memory.request_focus(id));
        assert!(app.text_input_focused(&ctx));
    }

    #[test]
    fn opening_scrollback_search_focuses_query_field() {
        let (mut app, ctx, _dir) = fixture();
        app.active_session = Some("live".into());
        app.open_scrollback_search(&ctx, "live");
        assert!(app.search_open);
        assert_eq!(
            ctx.memory(eframe::egui::Memory::focused),
            Some(egui::Id::new("scrollback-search"))
        );
    }

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
            |app| app.palette_open = true,
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
            app.palette_open = false;
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
            |ui| app.inline_rename(ui, "named", RenameSurface::Workspace, rect),
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
            Some(App::explorer_name_prompt_id())
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
            Some(App::explorer_name_prompt_id())
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
        let glyph = placeholder_center(&output.shapes, "Find in folder")
            .expect("Find in folder placeholder");
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
    fn hide_center_overlay_when_settings_dirty_sets_pending_close() {
        let (mut app, _, _dir) = fixture();
        app.open_settings();
        app.settings_draft.shell = "/tmp/custom".into();
        assert!(app.settings_dirty());
        app.hide_center_overlay();
        assert!(app.settings_pending.is_some());
        assert!(!app.settings_open);
    }

    #[test]
    fn open_settings_clears_search_and_worktree() {
        let (mut app, _, _dir) = fixture();
        app.search_open = true;
        app.worktree_open = true;
        app.open_settings();
        assert!(!app.search_open);
        assert!(!app.worktree_open);
        assert!(app.settings_open);
    }

    #[test]
    fn open_player_clears_search_and_worktree() {
        let (mut app, _, _dir) = fixture();
        app.search_open = true;
        app.worktree_open = true;
        app.open_player();
        assert!(!app.search_open);
        assert!(!app.worktree_open);
        assert!(app.player_open);
    }

    #[test]
    fn raw_input_hook_preserves_renderer_texture_limit() {
        let (mut app, ctx, _dir) = fixture();
        // The hook must not rewrite the renderer limit: immediate
        // (floating) viewports bypass the hook, so a parent-only value
        // would alternate the shared font atlas every frame.
        let mut input = egui::RawInput {
            max_texture_side: Some(16_384),
            ..Default::default()
        };
        eframe::App::raw_input_hook(&mut app, &ctx, &mut input);
        assert_eq!(input.max_texture_side, Some(16_384));
        let mut missing = egui::RawInput::default();
        eframe::App::raw_input_hook(&mut app, &ctx, &mut missing);
        assert_eq!(missing.max_texture_side, None);
    }

    #[test]
    fn idle_legacy_or_broken_daemon_is_retired_but_live_sessions_are_preserved() {
        let mut state = State {
            daemon_version: Some(env!("CARGO_PKG_VERSION").into()),
            capabilities: vec![SHUTDOWN_IF_IDLE_CAPABILITY.into()],
            ..State::default()
        };
        for health in [None, Some(false)] {
            state.attachment_helper_available = health;
            assert!(can_retire_daemon(&state));
            assert!(!can_restart_service(&state));
            state
                .sessions
                .push(session_fixture("live", SessionKind::Shell));
            assert!(!can_retire_daemon(&state));
            assert!(can_restart_service(&state));
            state.sessions.clear();
        }
        state.attachment_helper_available = Some(true);
        // Even a healthy sibling helper must migrate to a pinned copy when idle.
        assert!(can_retire_daemon(&state));
        state.capabilities.push(STABLE_HELPER_CAPABILITY.into());
        assert!(
            can_retire_daemon(&state),
            "An idle legacy service must migrate to generation ownership"
        );
        state.capabilities.push(generations::CAPABILITY.into());
        assert!(!can_retire_daemon(&state));
        state.attachment_helper_available = Some(false);
        state.capabilities.clear();
        assert!(!can_retire_daemon(&state));
        state.capabilities.push(SHUTDOWN_IF_IDLE_CAPABILITY.into());
        state.daemon_version = Some("999.0.0".into());
        assert!(!can_retire_daemon(&state));
    }
}
