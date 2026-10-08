use eframe::egui::{self};
#[cfg(test)]
use egui_dock::DockState;
use egui_dock::NodeIndex;
use std::{
    collections::HashMap,
    fs,
    path::Path,
    sync::mpsc::{self},
};
use terminator_core::*;

use super::super::*;
use super::nav_common::*;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(feature = "test-support")]
    fn git_panel_clicks_reuse_an_open_diff_instead_of_duplicating() {
        let (mut app, ctx, _dir) = fixture();
        app.context = Some(services::ContextData {
            cwd: "/a".into(),
            root: Some("/a".into()),
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
                        egui::vec2(260.0, 200.0),
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
                            branches: &[],
                            commit_draft: &mut draft,
                            collapse_generation: 0,
                            open_shortcut: "",
                            compare: None,
                            base_ref: None,
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
            app.process_updates(&ctx);
            if app.layouts["a"].contains(&expected) {
                break;
            }
        }
        assert!(app.layouts["a"].contains(&expected));
        assert!(!requests.try_iter().any(|job| matches!(job, Job::Diff(_))));
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
            let Job::Control(request, After::Workspace(id, anchors)) = requests.recv().unwrap()
            else {
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
            app.process_updates(&ctx);
            assert_eq!(app.selected.as_deref(), Some("b"));
        }
        assert_ne!(ids[0], ids[1]);
        assert!(app.layouts["a"].contains(&Tab::Terminal("working".into())));
        assert!(app.layouts["a"].contains(&Tab::Terminal("staged".into())));
    }

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
    fn sidebar_navigation_selects_owning_top_level_tab() {
        let (mut app, _, _dir) = fixture();
        app.state.sessions.extend([
            session_fixture("one", SessionKind::Shell),
            session_fixture("two", SessionKind::Shell),
        ]);
        app.insert("a", Tab::Terminal("one".into()), None);
        let first = app.layouts["a"].active.clone();
        app.layouts
            .get_mut("a")
            .unwrap()
            .add("second".into(), Tab::Terminal("two".into()));
        app.go_session("one");
        assert_eq!(app.layouts["a"].active, first);
        assert_eq!(app.active_session.as_deref(), Some("one"));
        app.go_session("two");
        assert_eq!(app.layouts["a"].active, "second");
        assert_eq!(app.layouts["a"].tabs.len(), 2);
    }

    #[test]
    fn delayed_split_stays_in_origin_tab_without_stealing_tab_selection() {
        let (mut app, ctx, _dir) = fixture();
        app.insert("a", Tab::Terminal("one".into()), None);
        let first = app.layouts["a"].active.clone();
        app.layouts
            .get_mut("a")
            .unwrap()
            .add("second".into(), Tab::Terminal("two".into()));
        app.active_session = Some("two".into());
        app.update_tx
            .send(Update::Created(
                session_fixture("split", SessionKind::Shell),
                Some("right".into()),
                Some(vec![Tab::Terminal("one".into())]),
            ))
            .unwrap();
        app.process_updates(&ctx);
        assert_eq!(app.layouts["a"].active, "second");
        assert_eq!(app.active_session.as_deref(), Some("two"));
        assert_eq!(
            app.layouts["a"]
                .tabs
                .iter()
                .find(|tab| tab.id == first)
                .unwrap()
                .layout
                .iter_all_tabs()
                .count(),
            2
        );
    }

    #[test]
    fn closing_either_split_direction_expands_the_remaining_pane() {
        for direction in ["left", "right", "up", "down"] {
            let (mut app, _, _dir) = fixture();
            app.state.sessions.extend([
                session_fixture("remaining", SessionKind::Shell),
                session_fixture("closed", SessionKind::Shell),
            ]);
            app.insert("a", Tab::Terminal("remaining".into()), None);
            app.insert("a", Tab::Terminal("closed".into()), Some(direction));
            app.active_session = Some("closed".into());
            let mut next = app.state.clone();
            next.sessions
                .iter_mut()
                .find(|s| s.id == "closed")
                .unwrap()
                .lifecycle = Lifecycle::Ended;
            app.apply_state(next);
            let leaf = app.layouts["a"].main_surface()[NodeIndex::root()]
                .get_leaf()
                .expect("Remaining pane should replace the split root");
            assert_eq!(leaf.tabs, vec![Tab::Terminal("remaining".into())]);
            assert_eq!(app.active_session.as_deref(), Some("remaining"));
            assert!(
                app.state
                    .sessions
                    .iter()
                    .any(|s| s.id == "closed" && s.lifecycle == Lifecycle::Ended)
            );
        }
    }

    #[test]
    fn restart_cleans_ended_panes_but_history_can_be_reopened() {
        let (mut app, _, _dir) = fixture();
        let mut dock = DockState::new(vec![Tab::Terminal("remaining".into())]);
        dock.main_surface_mut().split_below(
            NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("ended".into())],
        );
        let mut state = app.state.clone();
        state.projects[0].layout = sanitize_layout(serde_json::to_value(&dock).unwrap());
        let mut ended = session_fixture("ended", SessionKind::Shell);
        ended.lifecycle = Lifecycle::Ended;
        state.sessions = vec![session_fixture("remaining", SessionKind::Shell), ended];
        app.layouts.clear();
        app.state_loaded = false;
        app.apply_state(state.clone());
        assert!(app.layouts["a"].main_surface()[NodeIndex::root()].is_leaf());
        app.go_session("ended");
        app.apply_state(state);
        assert!(
            app.layouts["a"]
                .find_tab(&Tab::Terminal("ended".into()))
                .is_some()
        );
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
            app.process_updates(&ctx);
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
        app.process_updates(&ctx);
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
    fn appearance_preview_cancel_and_external_conflict() {
        let (mut app, ctx, _dir) = fixture();
        app.open_settings();
        app.theme_draft.text = "#123456".into();
        app.preview_appearance(&ctx);
        assert_eq!(app.theme.text, "#123456");
        app.hide_center_overlay();
        app.preview_appearance(&ctx);
        assert_eq!(app.theme, app.theme_committed);
        app.open_settings();
        app.theme_draft.text = "#123456".into();
        let external = AppearanceConfig {
            text: "#654321".into(),
            ..Default::default()
        };
        app.update_tx
            .send(Update::Appearance(Box::new(AppearanceFile {
                config: external.clone(),
                source: "external".into(),
            })))
            .unwrap();
        app.process_updates(&ctx);
        assert!(app.theme_conflict);
        assert_eq!(app.theme_draft.text, "#123456");
        assert_eq!(app.theme_committed, external);
    }

    #[test]
    fn idle_close_deduplicates_and_preserves_new_tab_contents() {
        let (mut app, _, _dir) = fixture();
        let (jobs, received) = mpsc::channel();
        app.jobs = jobs.into();
        app.state
            .sessions
            .push(session_fixture("shell", SessionKind::Shell));
        app.state
            .capabilities
            .push(terminator_core::idle_close::CAPABILITY.into());
        app.insert("a", Tab::Terminal("shell".into()), None);
        let tab = app.layouts["a"].active.clone();
        let target = editor_close::Target::Workspace("a".into(), tab.clone());
        assert!(app.check_idle_close(target.clone(), vec!["shell".into()]));
        assert!(app.check_idle_close(target.clone(), vec!["shell".into()]));
        assert!(matches!(received.try_recv().unwrap(), Job::CloseIdle(..)));
        assert!(received.try_recv().is_err());
        app.insert("a", Tab::Terminal("new-shell".into()), Some("right"));
        app.idle_closed(
            target,
            vec!["shell".into()],
            Ok(vec![terminator_core::idle_close::Outcome {
                session: "shell".into(),
                status: terminator_core::idle_close::Status::Closed,
                reason: "Exited".into(),
            }]),
        );
        assert!(app.layouts["a"].tabs.iter().any(|t| t.id == tab));
        assert!(app.layouts["a"].contains(&Tab::Terminal("new-shell".into())));
    }

    #[test]
    fn older_daemons_and_mixed_editor_tabs_keep_confirmation() {
        let (mut app, _, _dir) = fixture();
        let (jobs, received) = mpsc::channel();
        app.jobs = jobs.into();
        app.state
            .sessions
            .push(session_fixture("shell", SessionKind::Shell));
        app.state
            .sessions
            .push(session_fixture("editor", SessionKind::Editor));
        let target = editor_close::Target::Pane("shell".into());
        assert!(!app.check_idle_close(target.clone(), vec!["shell".into()]));
        app.state
            .capabilities
            .push(terminator_core::idle_close::CAPABILITY.into());
        assert!(!app.check_idle_close(target, vec!["shell".into(), "editor".into()]));
        assert!(received.try_recv().is_err());
    }

    #[test]
    fn idle_close_busy_shell_asks_without_error() {
        let (mut app, _, _dir) = fixture();
        let (jobs, received) = mpsc::channel();
        app.jobs = jobs.into();
        app.state
            .sessions
            .push(session_fixture("shell", SessionKind::Shell));
        app.state
            .capabilities
            .push(terminator_core::idle_close::CAPABILITY.into());
        app.insert("a", Tab::Terminal("shell".into()), None);
        let tab = app.layouts["a"].active.clone();
        let target = editor_close::Target::Workspace("a".into(), tab.clone());
        app.close_workspace = Some(("a".into(), tab.clone()));
        assert!(app.check_idle_close(target.clone(), vec!["shell".into()]));
        assert!(matches!(received.try_recv().unwrap(), Job::CloseIdle(..)));
        app.idle_closed(
            target.clone(),
            vec!["shell".into()],
            Ok(vec![terminator_core::idle_close::Outcome {
                session: "shell".into(),
                status: terminator_core::idle_close::Status::Busy,
                reason: "A foreground command owns the terminal".into(),
            }]),
        );
        assert_eq!(
            app.close_workspace.as_ref(),
            Some(&(String::from("a"), tab))
        );
        assert_eq!(app.idle_close_fallback.as_ref(), Some(&target));
        assert!(app.error.is_none());
        assert!(app.layouts["a"].contains(&Tab::Terminal("shell".into())));
    }

    #[test]
    fn idle_close_failed_signal_reports_error() {
        let (mut app, _, _dir) = fixture();
        let (jobs, received) = mpsc::channel();
        app.jobs = jobs.into();
        app.state
            .sessions
            .push(session_fixture("shell", SessionKind::Shell));
        app.state
            .capabilities
            .push(terminator_core::idle_close::CAPABILITY.into());
        app.insert("a", Tab::Terminal("shell".into()), None);
        let target = editor_close::Target::Pane("shell".into());
        assert!(app.check_idle_close(target.clone(), vec!["shell".into()]));
        assert!(matches!(received.try_recv().unwrap(), Job::CloseIdle(..)));
        app.idle_closed(
            target.clone(),
            vec!["shell".into()],
            Ok(vec![terminator_core::idle_close::Outcome {
                session: "shell".into(),
                status: terminator_core::idle_close::Status::Failed,
                reason: "Signal delivered, but exit was not confirmed; view preserved".into(),
            }]),
        );
        assert_eq!(app.idle_close_fallback.as_ref(), Some(&target));
        assert!(app.error.as_deref().is_some_and(|e| e.contains("Failed")));
        assert!(app.layouts["a"].contains(&Tab::Terminal("shell".into())));
    }
}
