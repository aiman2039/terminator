use super::super::appearance;
use eframe::egui::{self};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::mpsc::{self},
    time::{Duration, Instant},
};
use terminator_core::*;

use super::super::*;
use super::nav_common::*;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(feature = "test-support")]
    fn expanded_project_terminals_stay_indented_at_all_sidebar_sizes() {
        for width in [180.0, 280.0, 420.0] {
            for scale in [1.0, 2.0] {
                let (mut app, ctx, _dir) = fixture();
                ctx.set_pixels_per_point(scale);
                let first = session_fixture("first-shell", SessionKind::Shell);
                let mut second = session_fixture("second-shell", SessionKind::Shell);
                second.project_id = "b".into();
                app.state.sessions = vec![first, second];
                for project in ["a", "b"] {
                    app.preferences.expanded.insert(project.into(), true);
                }
                for selected in ["a", "b"] {
                    app.selected = Some(selected.into());
                    // Include the first frame and settled layout frames.
                    for _ in 0..3 {
                        let mut expected_indent = 0.0;
                        let mut output = ctx.run_ui(
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(
                                    egui::Pos2::ZERO,
                                    egui::vec2(width, 800.0),
                                )),
                                ..Default::default()
                            },
                            |ui| {
                                expected_indent = ui.spacing().indent;
                                app.projects(ui);
                            },
                        );
                        output.textures_delta.clear();
                        let target = |name: &str| {
                            ctx.data(|data| {
                                data.get_temp::<egui::Rect>(egui::Id::new(("fixture-target", name)))
                            })
                            .unwrap()
                        };
                        for (project, session) in [("a", "first-shell"), ("b", "second-shell")] {
                            let parent = target(&format!("project-row:{project}"));
                            let child = target(&format!("session-row:{session}"));
                            let indent = child.left() - parent.left();
                            assert!(
                                indent >= expected_indent - 0.1,
                                "width={width}, scale={scale}, project={project}: indent={indent}"
                            );
                            assert!(child.top() >= parent.bottom());
                        }
                    }
                }
            }
        }
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn project_list_excludes_editors_and_global_history_includes_other_projects() {
        let (mut app, ctx, _dir) = fixture();
        let mut ended = session_fixture("ended-other", SessionKind::Shell);
        ended.project_id = "b".into();
        ended.lifecycle = Lifecycle::Ended;
        let mut resumable = session_fixture("ended-resumable", SessionKind::Shell);
        resumable.project_id = "b".into();
        resumable.lifecycle = Lifecycle::Ended;
        app.state.sessions = vec![
            session_fixture("live-shell", SessionKind::Shell),
            session_fixture("open-file", SessionKind::Editor),
            ended,
            resumable,
        ];
        app.state.agents = vec![Agent {
            invocation_id: "agent-1".into(),
            session_id: "ended-resumable".into(),
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
        app.preferences.expanded.insert("a".into(), true);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.projects(ui));
        output.textures_delta.clear();
        let target = |name: &str| {
            ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new(("fixture-target", name))))
        };
        assert!(target("session-row:live-shell").is_some());
        assert!(target("session-row:open-file").is_none());
        assert!(target("session-row:ended-other").is_none());
        app.preferences.tool = SidebarTool::History;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.sidebar(ui));
        output.textures_delta.clear();
        assert!(target("session-row:ended-other").is_none());
        assert!(target("session-row:ended-resumable").is_some());
        assert!(target("history-filter").is_some());
        assert!(target("history-sort").is_some());
        assert!(target("history-toggle-all").is_some());
        assert_eq!(app.state.sessions.len(), 4);
    }

    #[test]
    fn project_sidebar_does_not_repeat_the_live_session_count() {
        let (mut app, ctx, _dir) = fixture();
        app.state.sessions = vec![session_fixture("live-shell", SessionKind::Shell)];
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(280.0, 500.0),
                )),
                ..Default::default()
            },
            |ui| app.projects(ui),
        );
        let mut text = Vec::new();
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        walk(shape, out);
                    }
                }
                egui::Shape::Text(painted) => out.push(painted.galley.text().to_owned()),
                _ => {}
            }
        }
        for clipped in &output.shapes {
            walk(&clipped.shape, &mut text);
        }
        output.textures_delta.clear();
        assert!(
            text.iter().all(|line| !line.contains("live sessions")),
            "{text:?}"
        );
    }

    #[test]
    fn project_sidebar_paints_a_working_agent_icon() {
        let (mut app, ctx, _dir) = fixture();
        appearance::install(&ctx);
        app.selected = Some("a".into());
        app.preferences.expanded.insert("a".into(), true);
        app.state.sessions = vec![session_fixture("s", SessionKind::Shell)];
        app.state.agents = vec![Agent {
            invocation_id: "agent".into(),
            session_id: "s".into(),
            kind: "codex".into(),
            provider_session_id: None,
            state: AgentState::Running,
            sequence: None,
            updated: 1,
            resume: None,
            process: None,
        }];
        let presented = app.present_session("s");
        assert_eq!(presented.brand_icon, Some("AgentCodex"));
        assert_eq!(presented.status_icon, "LoaderCircle");
        let mut marks = 0;
        for _ in 0..4 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(280.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ui| app.projects(ui),
            );
            marks = output
                .shapes
                .iter()
                .filter(|shape| {
                    matches!(
                        &shape.shape,
                        egui::Shape::Rect(rect) if rect.brush.is_some()
                    ) || matches!(&shape.shape, egui::Shape::Mesh(_))
                })
                .count();
            output.textures_delta.clear();
        }
        assert!(
            marks >= 2,
            "a working agent must paint its brand icon and status icon, got {marks}"
        );
    }

    #[test]
    fn strip_tabs_reserve_leading_icons_for_every_terminal() {
        use egui_dock::TabViewer;
        let (mut app, _ctx, _dir) = fixture();
        app.state.sessions = vec![
            session_fixture("agent", SessionKind::Shell),
            session_fixture("plain", SessionKind::Shell),
        ];
        app.state.agents = vec![Agent {
            invocation_id: "agent".into(),
            session_id: "agent".into(),
            kind: "codex".into(),
            provider_session_id: None,
            state: AgentState::Running,
            sequence: None,
            updated: 1,
            resume: None,
            process: None,
        }];
        let viewer = Viewer {
            app: &mut app,
            strip: true,
            project: None,
            tab: None,
            node: None,
        };
        let agent_width = viewer.tab_leading_width(&Tab::Terminal("agent".into()));
        let plain_width = viewer.tab_leading_width(&Tab::Terminal("plain".into()));
        assert!(
            agent_width > plain_width,
            "an agent tab carries brand plus status, a plain shell only its kind glyph"
        );
        assert!(
            plain_width > 0.0,
            "even a plain shell reserves a kind icon on strip tabs"
        );
        assert_eq!(
            viewer.tab_leading_width(&Tab::Terminal("missing".into())),
            0.0,
            "unknown sessions reserve no icon slot"
        );
        assert_eq!(
            viewer.tab_leading_width(&Tab::Player),
            0.0,
            "non-terminal tabs reserve no icon slot"
        );
    }

    #[test]
    fn strip_tabs_show_a_detected_agent_before_any_hook() {
        use egui_dock::TabViewer;
        let (mut app, _ctx, _dir) = fixture();
        app.state
            .capabilities
            .push(AGENT_PRESENCE_CAPABILITY.into());
        app.state.sessions = vec![
            session_fixture("muse", SessionKind::Shell),
            session_fixture("plain", SessionKind::Shell),
        ];
        app.state.presence.push(agents::TerminalPresence {
            session_id: "muse".into(),
            generation: "fixture".into(),
            agents: vec![agents::DetectedAgent {
                kind: "muse".into(),
                process: agents::ProcessIdentity {
                    pid: 9,
                    start_time: 1,
                },
                foreground: true,
            }],
            outcome: agents::PresenceOutcome::Verified,
            observed_at: now(),
        });
        let viewer = Viewer {
            app: &mut app,
            strip: true,
            project: None,
            tab: None,
            node: None,
        };
        let muse = viewer.tab_leading_width(&Tab::Terminal("muse".into()));
        let plain = viewer.tab_leading_width(&Tab::Terminal("plain".into()));
        assert_eq!(
            muse,
            plain + appearance::TERMINAL_LEADING_SLOT,
            "a process-detected agent reserves its brand before any hook"
        );
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn project_header_controls_share_height() {
        let (mut app, ctx, _dir) = fixture();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(320.0, 400.0),
                )),
                ..Default::default()
            },
            |ui| app.projects(ui),
        );
        output.textures_delta.clear();
        let target = |name: &str| {
            ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new(("fixture-target", name))))
                .unwrap()
        };
        let add = target("project-add");
        let sort = target("project-sort");
        assert!(
            (add.height() - sort.height()).abs() < 8.0,
            "add={add:?} sort={sort:?}"
        );
        assert!(
            (add.center().y - sort.center().y).abs() < 4.0,
            "add={add:?} sort={sort:?}"
        );
        let filter = target("project-filter");
        assert!(
            (filter.height() - add.height()).abs() < 8.0,
            "filter={filter:?} add={add:?}"
        );
        assert!(
            (filter.center().y - add.center().y).abs() < 4.0,
            "filter={filter:?} add={add:?}"
        );
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn project_filter_narrows_visible_projects() {
        let (mut app, ctx, _dir) = fixture();
        assert_eq!(app.cached_projects().len(), 2);
        app.preferences.project_filter = "b".into();
        let visible = app.cached_projects();
        assert_eq!(
            visible.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["b"]
        );
        app.preferences.project_filter = "zzz".into();
        assert!(app.cached_projects().is_empty());
        app.preferences.project_filter.clear();
        assert_eq!(app.cached_projects().len(), 2);
        let _ = &ctx;
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn project_filter_field_owns_text_keys() {
        let (mut app, ctx, _dir) = fixture();
        assert!(!app.text_input_focused(&ctx));
        let paint = |app: &mut App| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(320.0, 400.0),
                    )),
                    ..Default::default()
                },
                |ui| app.projects(ui),
            );
            output.textures_delta.clear();
        };
        paint(&mut app);
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new("project-filter")));
        paint(&mut app);
        assert!(app.text_input_focused(&ctx));
    }

    #[test]
    fn file_only_views_are_distinguished_from_shell_and_mixed_tabs() {
        let (mut app, _, _dir) = fixture();
        app.state.sessions.extend([
            session_fixture("file", SessionKind::Editor),
            session_fixture("shell", SessionKind::Shell),
        ]);
        assert!(app.editors_only(&["file".into()]));
        assert!(!app.editors_only(&["shell".into()]));
        assert!(!app.editors_only(&["file".into(), "shell".into()]));
        assert!(!app.editors_only(&[]));
    }

    #[test]
    fn extra_close_while_prompting_does_not_queue_another_check() {
        let (mut app, _, _dir) = fixture();
        let (jobs, requests) = mpsc::channel();
        app.jobs = jobs.into();
        app.upsert_unsaved_close(
            editor_close::Target::Pane("file".into()),
            vec!["file".into()],
            "Unsaved changes".into(),
        );
        assert!(app.skip_editor_close_request(&["file".into()]));
        assert!(!app.skip_editor_close_request(&["other".into()]));
        app.close_editors(
            editor_close::Target::Pane("file".into()),
            vec!["file".into()],
            editor_close::Mode::Discard,
        );
        assert!(matches!(
            requests.try_recv().unwrap(),
            Job::CloseEditors(_, _, editor_close::Mode::Discard, _)
        ));
        app.close_editors(
            editor_close::Target::Pane("file".into()),
            vec!["file".into()],
            editor_close::Mode::Discard,
        );
        assert!(requests.try_recv().is_err());
    }

    #[test]
    fn failed_close_updates_the_same_prompt() {
        let (mut app, _, _dir) = fixture();
        app.upsert_unsaved_close(
            editor_close::Target::Pane("file".into()),
            vec!["file".into()],
            "Unsaved changes".into(),
        );
        app.editors_closed(
            editor_close::Target::Pane("file".into()),
            vec!["file".into()],
            Err("Editor did not close. Check for unsaved buffers or running editor jobs.".into()),
        );
        assert_eq!(app.editor_close_prompts.len(), 1);
        assert!(
            app.editor_close_prompts[0]
                .2
                .contains("Editor did not close")
        );
        app.editors_closed(
            editor_close::Target::Pane("other".into()),
            vec!["other".into()],
            Err("Unsaved changes".into()),
        );
        assert_eq!(app.editor_close_prompts.len(), 2);
        assert!(app.unsaved_close_prompt("file").is_some());
        assert!(app.unsaved_close_prompt("other").is_some());
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
                        |ui| app.inline_rename(ui, "named", surface, rect),
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
                    !input.events.iter().any(|event| {
                        matches!(event, egui::Event::Text(_) | egui::Event::Key { .. })
                    })
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
        app.process_updates(&ctx);
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
    fn legacy_daemon_diff_never_sends_an_unsupported_creation_request() {
        let (mut app, ctx, _dir) = fixture();
        app.state.settings.review_mode = ReviewMode::Neovim;
        app.context = Some(services::ContextData {
            cwd: "/a".into(),
            root: Some("/a".into()),
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
    fn git_click_opens_a_native_diff() {
        let (mut app, ctx, _dir) = fixture();
        app.context = Some(services::ContextData {
            cwd: "/a".into(),
            root: Some("/a".into()),
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
}
