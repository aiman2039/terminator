use super::super::appearance;
use eframe::egui::{self};
#[cfg(test)]
use egui_dock::DockState;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::mpsc::{self},
};
use terminator_core::*;

use super::super::*;
use super::nav_common::*;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_created_inserts_at_requested_index() {
        let (mut app, ctx, _dir) = fixture();
        let workspace = app.layouts.get_mut("a").unwrap();
        workspace.add("t0".into(), Tab::Terminal("s0".into()));
        workspace.add("t1".into(), Tab::Terminal("s1".into()));
        app.workspace_insert.insert("mid".into(), 1);
        app.update_tx
            .send(Update::WorkspaceCreated(
                session_fixture("mid-session", SessionKind::Shell),
                "mid".into(),
                vec![],
            ))
            .unwrap();
        app.process_updates(&ctx);
        assert_eq!(
            app.layouts["a"]
                .tabs
                .iter()
                .map(|tab| tab.id.as_str())
                .collect::<Vec<_>>(),
            ["t0", "mid", "t1"]
        );
        assert_eq!(app.layouts["a"].active, "mid");
        assert!(app.workspace_insert.is_empty());
    }

    #[test]
    fn leaf_plus_opens_stacked_in_that_split() {
        let (mut app, _, _dir) = fixture();
        let (jobs, received) = mpsc::channel();
        app.jobs = jobs.into();
        app.selected = Some("a".into());
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        workspace.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("two".into())],
        );
        app.layouts.insert("a".into(), workspace);
        let path = app.layouts["a"]
            .find_tab(&Tab::Terminal("two".into()))
            .expect("right leaf present")
            .node_path();
        app.add_tab = Some((path, None));
        let mut dock = app.layouts.remove("a").unwrap();
        app.apply_add_tab("a", &mut dock);
        app.layouts.insert("a".into(), dock);
        let Job::Control(_, After::CreateAt(_, split)) = received.try_recv().unwrap() else {
            panic!("leaf plus stacks into the split");
        };
        assert_eq!(split, None);
    }

    #[test]
    fn insert_here_stacks_into_focused_leaf() {
        let (mut app, _, _dir) = fixture();
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        workspace.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("two".into())],
        );
        app.layouts.insert("a".into(), workspace);
        let editor = Tab::NativeEditor {
            path: "/a/note.md".into(),
        };
        app.insert("a", editor.clone(), Some(App::SPLIT_HERE));
        // No new top-level tab and no side split: the file stacks into
        // the focused leaf alongside its existing pane.
        assert_eq!(app.layouts["a"].tabs.len(), 1);
        let path = app.layouts["a"]
            .find_tab(&editor)
            .expect("editor placed")
            .node_path();
        let leaf = app.layouts["a"].leaf(path).expect("leaf present");
        assert_eq!(leaf.tabs.len(), 2);
    }

    #[test]
    fn file_open_here_routes_stacked_create() {
        let (mut app, ctx, _dir) = fixture();
        let (jobs, received) = mpsc::channel();
        app.jobs = jobs.into();
        let session = session_fixture("one", SessionKind::Shell);
        app.state.sessions.push(session.clone());
        let target = services::Target::File("/a/note.md".into(), None, None);
        app.terminal_action(&ctx, &session, &target, FileAction::Here);
        let Job::Control(_, After::CreateAt(_, split)) = received.try_recv().unwrap() else {
            panic!("Here opens stacked in the current split");
        };
        assert_eq!(split.as_deref(), Some("here"));
    }

    #[test]
    fn floatable_excludes_browser_and_player() {
        assert!(App::floatable(&Tab::Terminal("one".into())));
        assert!(App::floatable(&Tab::NativeEditor {
            path: "/a/note.md".into()
        }));
        assert!(App::floatable(&Tab::Diff {
            cwd: "/a".into(),
            path: "/a/note.md".into(),
            staged: false,
        }));
        assert!(!App::floatable(&Tab::browser_file("/a/page.html".into())));
        assert!(!App::floatable(&Tab::Player));
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
        app.floating.push(FloatingPane {
            viewport: egui::ViewportId::from_hash_of("float-native"),
            tab: Some(Tab::NativeEditor { path: path.clone() }),
            home: ("a".into(), "home".into()),
        });
        // A plain close issued from the dock removes that copy and keeps
        // the floated buffer: the floating window stays open.
        app.close_native_tab(
            &path,
            false,
            Some(crate::native_editor::CloseIssuer::Docked("a".into())),
        );
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
        app.floating.push(FloatingPane {
            viewport: egui::ViewportId::from_hash_of("float-forced"),
            tab: Some(Tab::NativeEditor { path: path.clone() }),
            home: ("a".into(), "home".into()),
        });
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
        for viewport in [first, second] {
            app.floating.push(FloatingPane {
                viewport,
                tab: Some(Tab::NativeEditor { path: path.clone() }),
                home: ("a".into(), "home".into()),
            });
        }
        // `:q` from the first floating view closes that window only:
        // the second view — and its unsaved buffer — survive.
        let floating = crate::native_editor::CloseIssuer::Floating;
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
        app.floating.push(FloatingPane {
            viewport: egui::ViewportId::from_hash_of("float-qa"),
            tab: Some(Tab::NativeEditor { path: path.clone() }),
            home: ("a".into(), "home".into()),
        });
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
        app.floating.push(FloatingPane {
            viewport: egui::ViewportId::from_hash_of("float-only"),
            tab: Some(Tab::NativeEditor { path: path.clone() }),
            home: ("a".into(), "home".into()),
        });
        // `:qa!` leaves no floating window and no buffer behind, even
        // when no workspace tab shows the file.
        app.pending_quit_all = Some(true);
        app.drain_pending_native_close();
        assert!(app.floating.is_empty());
        assert!(!app.native_docs.contains_key(&path));
    }

    fn docked_copies(app: &App, project: &str, path: &Path) -> usize {
        app.layouts
            .get(project)
            .map(|workspace| {
                workspace
                    .tabs
                    .iter()
                    .flat_map(|tab| tab.layout.iter_all_tabs())
                    .filter(|(_, tab)| {
                        matches!(tab, Tab::NativeEditor { path: current } if current == path)
                    })
                    .count()
            })
            .unwrap_or(0)
    }

    #[test]
    fn plain_close_keeps_other_docked_copies() {
        use crate::native_editor::CloseIssuer;
        let (mut app, _, _dir) = fixture();
        let path = PathBuf::from("/a/shared.md");
        app.native_docs.insert(
            path.clone(),
            crate::native_editor::NativeDoc::dirty_for_test(path.clone()),
        );
        // Two splits on the same file in `a`, one copy in `b`.
        let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::NativeEditor {
            path: path.clone(),
        }]));
        workspace.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.5,
            vec![Tab::NativeEditor { path: path.clone() }],
        );
        app.layouts.insert("a".into(), workspace);
        app.layouts.insert(
            "b".into(),
            Workspace::from_layout(DockState::new(vec![Tab::NativeEditor {
                path: path.clone(),
            }])),
        );
        // A plain close issued in `a` removes one copy there; the other
        // split and project `b` keep theirs, and the buffer survives.
        app.close_native_tab(&path, false, Some(CloseIssuer::Docked("a".into())));
        assert_eq!(docked_copies(&app, "a", &path), 1);
        assert_eq!(docked_copies(&app, "b", &path), 1);
        assert!(app.native_docs.contains_key(&path));
    }

    #[test]
    fn plain_close_takes_focused_split_not_first_match() {
        use crate::native_editor::CloseIssuer;
        let (mut app, _, _dir) = fixture();
        let path = PathBuf::from("/a/focused.md");
        let other = PathBuf::from("/a/other.md");
        for file in [&path, &other] {
            app.native_docs.insert(
                file.clone(),
                crate::native_editor::NativeDoc::dirty_for_test(file.clone()),
            );
        }
        // Left leaf holds only the file; the focused right leaf holds
        // the file plus another tab.
        let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::NativeEditor {
            path: path.clone(),
        }]));
        let [_, right] = workspace.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.5,
            vec![
                Tab::NativeEditor {
                    path: other.clone(),
                },
                Tab::NativeEditor { path: path.clone() },
            ],
        );
        workspace.tabs[0]
            .layout
            .set_focused_node_and_surface(egui_dock::NodePath {
                surface: egui_dock::SurfaceIndex::main(),
                node: right,
            });
        app.layouts.insert("a".into(), workspace);
        // `:q` in the focused split removes that copy: the focused leaf
        // keeps only the other tab, the first leaf keeps the file.
        app.close_native_tab(&path, false, Some(CloseIssuer::Docked("a".into())));
        let workspace = app.layouts.get("a").expect("workspace");
        let mut focused_tabs = Vec::new();
        let mut copies = 0;
        for (at, tab) in workspace.tabs[0].layout.iter_all_tabs() {
            if matches!(tab, Tab::NativeEditor { path: current } if current.as_path() == path.as_path())
            {
                copies += 1;
            }
            if at.node == right
                && let Tab::NativeEditor { path: current } = tab
            {
                focused_tabs.push(current.clone());
            }
        }
        assert_eq!(copies, 1);
        assert_eq!(focused_tabs, vec![other]);
        assert!(app.native_docs.contains_key(&path));
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn wq_close_uses_view_issuing_the_save() {
        use crate::native_editor::CloseIssuer;
        let (mut app, ctx, _dir) = fixture();
        let path = PathBuf::from("/a/wq-issuer.md");
        app.native_docs.insert(
            path.clone(),
            crate::native_editor::NativeDoc::new(path.clone(), false),
        );
        let float = egui::ViewportId::from_hash_of("wq-float");
        app.floating.push(FloatingPane {
            viewport: float,
            tab: Some(Tab::NativeEditor { path: path.clone() }),
            home: ("a".into(), "home".into()),
        });
        app.layouts.insert(
            "a".into(),
            Workspace::from_layout(DockState::new(vec![Tab::NativeEditor {
                path: path.clone(),
            }])),
        );
        // `:wq` typed in the floating window: the save settles later.
        app.native_close_after_save = Some(path.clone());
        app.native_close_after_save_issuer = Some(CloseIssuer::Floating(float));
        // A docked view renders first. The queued close must still target
        // the issuing floating pane — not the rendering docked copy.
        combined_frame(&mut app, &ctx, vec![]);
        // A second frame before the drain must not queue a second close
        // for the same request: it is consumed when first queued.
        combined_frame(&mut app, &ctx, vec![]);
        assert_eq!(
            app.pending_native_close.len(),
            1,
            "one close per :wq, not one per rendering copy"
        );
        app.drain_pending_native_close();
        assert!(app.floating.is_empty());
        assert_eq!(docked_copies(&app, "a", &path), 1);
        assert!(app.native_docs.contains_key(&path));
    }

    #[test]
    fn pending_closes_drain_with_no_project_selected() {
        let (mut app, ctx, _dir) = fixture();
        app.selected = None;
        let path = PathBuf::from("/a/hidden-float.md");
        app.native_docs.insert(
            path.clone(),
            crate::native_editor::NativeDoc::dirty_for_test(path.clone()),
        );
        app.floating.push(FloatingPane {
            viewport: egui::ViewportId::from_hash_of("hidden-float"),
            tab: Some(Tab::NativeEditor { path: path.clone() }),
            home: ("a".into(), "home".into()),
        });
        // Hiding the last project must not stall floating closes: with
        // nothing checked out, the drain still runs.
        app.pending_native_close.push((path.clone(), true, None));
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 480.0),
                )),
                ..Default::default()
            },
            |ui| app.window_header_tabs(ui),
        );
        output.textures_delta.clear();
        assert!(app.pending_native_close.is_empty());
        assert!(app.floating.is_empty());
        assert!(!app.native_docs.contains_key(&path));
    }

    #[test]
    fn deferred_goto_jump_opens_cross_file_target() {
        use terminator_native_edit::lsp::{GotoTarget, Position, Range};
        let (mut app, _, _dir) = fixture();
        let from = PathBuf::from("/a/from.rs");
        let to = PathBuf::from("/a/to.rs");
        let jump = || crate::lsp_manager::GotoJump {
            from: from.clone(),
            targets: vec![GotoTarget {
                uri: "file:///a/to.rs".into(),
                range: Range {
                    start: Position {
                        line: 4,
                        character: 0,
                    },
                    end: Position {
                        line: 4,
                        character: 1,
                    },
                },
            }],
        };
        app.layouts.insert(
            "a".into(),
            Workspace::from_layout(DockState::new(vec![Tab::NativeEditor {
                path: from.clone(),
            }])),
        );
        // One frame: the workspace is checked out for rendering, so the
        // finished jump queues instead of resolving against the missing
        // project (which dropped it before).
        let dock = app.layouts.remove("a").expect("checked out");
        app.pending_goto.push(jump());
        app.layouts.insert("a".into(), dock);
        // Deferred work runs with the workspace checked back in: the
        // jump resolves and opens the target.
        app.drain_pending_native_close();
        assert_eq!(app.native_pending_line.get(&to), Some(&4));
        let absolute = std::path::absolute(&to).unwrap();
        assert_eq!(app.native_pending_col.get(&absolute), Some(&0));
        // A jump whose project is truly gone applies nothing.
        app.layouts.remove("a");
        app.pending_goto.push(jump());
        app.drain_pending_native_close();
        assert_eq!(app.native_pending_line.len(), 1);
    }

    #[test]
    fn floated_panes_seed_visibility_before_prune() {
        let (mut app, _, _dir) = fixture();
        app.selected = Some("a".into());
        app.floating.push(FloatingPane {
            viewport: egui::ViewportId::from_hash_of("float-terminal"),
            tab: Some(Tab::Terminal("one".into())),
            home: ("a".into(), "home".into()),
        });
        app.floating.push(FloatingPane {
            viewport: egui::ViewportId::from_hash_of("float-image"),
            tab: Some(Tab::Image {
                path: "/tmp/a.png".into(),
            }),
            home: ("a".into(), "home".into()),
        });
        let mut doc = session_fixture("doc", SessionKind::Editor);
        doc.file = Some("/a/doc.md".into());
        app.state.sessions.push(doc);
        app.floating.push(FloatingPane {
            viewport: egui::ViewportId::from_hash_of("float-doc"),
            tab: Some(Tab::Terminal("doc".into())),
            home: ("a".into(), "home".into()),
        });
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

    #[test]
    fn add_tab_to_the_left_records_insert_index() {
        let (mut app, _, _dir) = fixture();
        let (jobs, received) = mpsc::channel();
        app.jobs = jobs.into();
        app.selected = Some("a".into());
        app.create_workspace_tab(Some(1));
        let Job::Control(_, After::Workspace(id, anchors)) = received.try_recv().unwrap() else {
            panic!("Expected workspace create")
        };
        assert!(anchors.is_empty());
        assert_eq!(app.workspace_insert.get(&id), Some(&1));
    }

    #[test]
    fn close_tabs_to_the_left_queues_then_cancel_keeps_the_rest() {
        let (mut app, _, _dir) = fixture();
        let workspace = app.layouts.get_mut("a").unwrap();
        workspace.add("t0".into(), Tab::Terminal("s0".into()));
        workspace.add("t1".into(), Tab::Terminal("s1".into()));
        workspace.add("t2".into(), Tab::Terminal("s2".into()));
        app.begin_workspace_close_tabs("a", vec!["t0".into(), "t1".into()]);
        assert_eq!(app.close_workspace, Some(("a".into(), "t0".into())));
        assert_eq!(app.close_workspace_queue, ["t1"]);
        app.close_workspace_tab_now("a", "t0");
        assert_eq!(app.close_workspace, Some(("a".into(), "t1".into())));
        assert!(!app.layouts["a"].tabs.iter().any(|tab| tab.id == "t0"));
        assert!(app.layouts["a"].tabs.iter().any(|tab| tab.id == "t2"));
        app.abort_workspace_close();
        assert!(app.close_workspace.is_none());
        assert!(app.close_workspace_queue.is_empty());
        assert!(app.layouts["a"].tabs.iter().any(|tab| tab.id == "t1"));
    }

    #[test]
    fn close_all_tabs_queues_every_tab() {
        let (mut app, _, _dir) = fixture();
        let workspace = app.layouts.get_mut("a").unwrap();
        workspace.add("t0".into(), Tab::Terminal("s0".into()));
        workspace.add("t1".into(), Tab::Terminal("s1".into()));
        workspace.add("t2".into(), Tab::Terminal("s2".into()));
        let ids = app.layouts["a"].ids();
        app.begin_workspace_close_tabs("a", ids);
        assert_eq!(app.close_workspace, Some(("a".into(), "t0".into())));
        assert_eq!(app.close_workspace_queue, ["t1", "t2"]);
    }

    #[test]
    fn close_all_empty_tabs_leaves_an_empty_workspace() {
        let (mut app, ctx, _dir) = fixture();
        let workspace = app.layouts.get_mut("a").unwrap();
        workspace.add(
            "t0".into(),
            Tab::Image {
                path: "/a.png".into(),
            },
        );
        workspace.add(
            "t1".into(),
            Tab::Image {
                path: "/b.png".into(),
            },
        );
        workspace.add(
            "t2".into(),
            Tab::Image {
                path: "/c.png".into(),
            },
        );
        let ids = app.layouts["a"].ids();
        app.begin_workspace_close_tabs("a", ids);
        app.poll_workspace_close(&ctx);
        assert!(app.close_workspace.is_none());
        assert!(app.close_workspace_queue.is_empty());
        assert_eq!(app.layouts["a"].tabs.len(), 1);
        assert_eq!(app.layouts["a"].iter_all_tabs().count(), 0);
    }

    #[test]
    fn close_all_live_tabs_wait_then_keep_running_advances() {
        let (mut app, ctx, _dir) = fixture();
        let workspace = app.layouts.get_mut("a").unwrap();
        workspace.add("t0".into(), Tab::Terminal("s0".into()));
        workspace.add("t1".into(), Tab::Terminal("s1".into()));
        workspace.add("t2".into(), Tab::Terminal("s2".into()));
        app.state.sessions.extend([
            session_fixture("s0", SessionKind::Shell),
            session_fixture("s1", SessionKind::Shell),
            session_fixture("s2", SessionKind::Shell),
        ]);
        let ids = app.layouts["a"].ids();
        app.begin_workspace_close_tabs("a", ids);
        poll_workspace_close_in_frame(&mut app, &ctx);
        assert_eq!(app.close_workspace, Some(("a".into(), "t0".into())));
        assert_eq!(app.close_workspace_queue, ["t1", "t2"]);
        assert_eq!(app.layouts["a"].ids(), ["t0", "t1", "t2"]);
        app.close_workspace_tab_now("a", "t0");
        assert_eq!(app.close_workspace, Some(("a".into(), "t1".into())));
        assert_eq!(app.close_workspace_queue, ["t2"]);
        assert_eq!(app.layouts["a"].ids(), ["t1", "t2"]);
    }

    #[test]
    fn close_all_drains_empty_tabs_until_a_live_session() {
        let (mut app, ctx, _dir) = fixture();
        let workspace = app.layouts.get_mut("a").unwrap();
        workspace.add(
            "t0".into(),
            Tab::Image {
                path: "/a.png".into(),
            },
        );
        workspace.add("t1".into(), Tab::Terminal("s1".into()));
        workspace.add(
            "t2".into(),
            Tab::Image {
                path: "/c.png".into(),
            },
        );
        app.state
            .sessions
            .push(session_fixture("s1", SessionKind::Shell));
        let ids = app.layouts["a"].ids();
        app.begin_workspace_close_tabs("a", ids);
        poll_workspace_close_in_frame(&mut app, &ctx);
        assert_eq!(app.close_workspace, Some(("a".into(), "t1".into())));
        assert_eq!(app.close_workspace_queue, ["t2"]);
        assert_eq!(app.layouts["a"].ids(), ["t1", "t2"]);
    }

    #[test]
    fn close_all_ended_sessions_leave_an_empty_workspace() {
        let (mut app, ctx, _dir) = fixture();
        let workspace = app.layouts.get_mut("a").unwrap();
        workspace.add("t0".into(), Tab::Terminal("s0".into()));
        workspace.add("t1".into(), Tab::Terminal("s1".into()));
        app.state.sessions.extend(["s0", "s1"].map(|id| {
            let mut session = session_fixture(id, SessionKind::Shell);
            session.lifecycle = Lifecycle::Ended;
            session
        }));
        let ids = app.layouts["a"].ids();
        app.begin_workspace_close_tabs("a", ids);
        app.poll_workspace_close(&ctx);
        assert!(app.close_workspace.is_none());
        assert!(app.close_workspace_queue.is_empty());
        assert_eq!(app.layouts["a"].tabs.len(), 1);
        assert_eq!(app.layouts["a"].iter_all_tabs().count(), 0);
    }

    #[test]
    fn empty_workspace_tabs_drain_without_prompt() {
        let (mut app, ctx, _dir) = fixture();
        let workspace = app.layouts.get_mut("a").unwrap();
        workspace.add(
            "t0".into(),
            Tab::Image {
                path: "/a.png".into(),
            },
        );
        workspace.add(
            "t1".into(),
            Tab::Image {
                path: "/b.png".into(),
            },
        );
        workspace.add(
            "t2".into(),
            Tab::Image {
                path: "/c.png".into(),
            },
        );
        app.begin_workspace_close_tabs("a", vec!["t0".into(), "t1".into()]);
        app.poll_workspace_close(&ctx);
        assert!(app.close_workspace.is_none());
        assert!(app.close_workspace_queue.is_empty());
        assert_eq!(
            app.layouts["a"]
                .tabs
                .iter()
                .map(|tab| tab.id.as_str())
                .collect::<Vec<_>>(),
            ["t2"]
        );
    }

    #[test]
    fn unsaved_close_cancel_aborts_remaining_workspace_tabs() {
        let (mut app, _, _dir) = fixture();
        app.close_workspace_queue = vec!["t1".into()];
        app.apply_unsaved_close_choice(
            appearance::UnsavedCloseChoice::Cancel,
            editor_close::Target::Workspace("a".into(), "t0".into()),
            vec!["editor".into()],
        );
        assert!(app.close_workspace_queue.is_empty());
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
        });
        let context = services::ContextData {
            cwd: path.clone(),
            root: None,
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
        app.process_updates(&ctx);
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
        app.process_updates(&ctx);
        assert!(app.dirs[&path].is_empty());
        assert!(!app.directory_errors.contains_key(&path));
    }

    #[test]
    fn stale_refresh_is_ignored_even_for_same_directory() {
        let (mut app, ctx, _dir) = fixture();
        app.refresh_generation = 3;
        app.refresh_request = Some(refresh::Request {
            cwd: "/a".into(),
            generation: 3,
            directories: vec![],
        });
        app.update_tx
            .send(Update::Refresh(
                2,
                services::ContextData {
                    cwd: "/a".into(),
                    root: None,
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
        app.process_updates(&ctx);
        assert!(app.context.is_none());
    }

    #[test]
    fn commit_switch_and_refresh_invalidate_git_lists() {
        let (mut app, ctx, _dir) = fixture();
        let root = PathBuf::from("/repo");
        app.git_list_root = Some(root.clone());
        app.update_tx
            .send(Update::Workspace(
                workspace_ops::Op::Stage(root.join("a.txt")),
                Ok(workspace_ops::Report::Message("Staged".into())),
            ))
            .unwrap();
        app.process_updates(&ctx);
        assert_eq!(app.git_list_root, Some(root.clone()));
        app.update_tx
            .send(Update::Workspace(
                workspace_ops::Op::Commit("nope".into()),
                Err("commit failed".into()),
            ))
            .unwrap();
        app.process_updates(&ctx);
        assert_eq!(app.git_list_root, Some(root.clone()));
        app.update_tx
            .send(Update::Workspace(
                workspace_ops::Op::Commit("yes".into()),
                Ok(workspace_ops::Report::Message("Committed".into())),
            ))
            .unwrap();
        app.process_updates(&ctx);
        assert!(app.git_list_root.is_none());
        app.git_list_root = Some(root.clone());
        app.update_tx
            .send(Update::Workspace(
                workspace_ops::Op::Switch("feature".into()),
                Ok(workspace_ops::Report::Message("Switched".into())),
            ))
            .unwrap();
        app.process_updates(&ctx);
        assert!(app.git_list_root.is_none());
        app.git_list_root = Some(root.clone());
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
        assert!(app.git_list_root.is_none());
    }

    #[test]
    fn delayed_creation_uses_original_project_after_navigation_and_pane_removal() {
        let (mut app, ctx, _dir) = fixture();
        app.insert("a", Tab::Terminal("anchor".into()), None);
        app.select_project("b".into());
        app.remove_tab("anchor");
        let session = Session {
            review: false,
            id: "created".into(),
            project_id: "a".into(),
            label: "new".into(),
            cwd: "/a/subdir".into(),
            kind: SessionKind::Shell,
            file: None,
            lifecycle: Lifecycle::Running,
            created: 0,
            exit_code: None,
            rows: 24,
            cols: 80,
            generation: "fixture".into(),
            pid: None,
            truncated: false,
            cwd_confirmed: true,
        };
        app.update_tx
            .send(Update::Created(
                session,
                None,
                Some(vec![Tab::Terminal("anchor".into())]),
            ))
            .unwrap();
        app.process_updates(&ctx);
        assert_eq!(app.selected.as_deref(), Some("b"));
        assert!(
            app.layouts["a"]
                .find_tab(&Tab::Terminal("created".into()))
                .is_some()
        );
    }

    #[test]
    fn cancelled_picker_and_error_keep_workspace_and_layout() {
        let (mut app, ctx, _dir) = fixture();
        app.insert("a", Tab::Terminal("original".into()), None);
        app.update_tx.send(Update::PickedProject(None, 0)).unwrap();
        app.update_tx
            .send(Update::Error("folder unavailable".into()))
            .unwrap();
        app.process_updates(&ctx);
        assert_eq!(app.selected.as_deref(), Some("a"));
        assert!(
            app.layouts["a"]
                .find_tab(&Tab::Terminal("original".into()))
                .is_some()
        );
    }

    #[test]
    fn delayed_open_refreshes_inventory_without_overriding_new_selection() {
        let (mut app, ctx, _dir) = fixture();
        app.select_project("b".into());
        app.select_project("a".into());
        app.update_tx
            .send(Update::OpenedProject(
                Box::new(app.state.clone()),
                "b".into(),
                0,
            ))
            .unwrap();
        app.process_updates(&ctx);
        assert_eq!(app.selected.as_deref(), Some("a"));
        app.update_tx
            .send(Update::OpenedProject(
                Box::new(app.state.clone()),
                "b".into(),
                app.selection_generation,
            ))
            .unwrap();
        app.process_updates(&ctx);
        assert_eq!(app.selected.as_deref(), Some("b"));
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
        app.open_file("/a/image.PNG".into(), None, None, false);
        app.select_project("b".into());
        pump_until(&mut app, &ctx, |app| {
            app.layouts["a"].contains(&Tab::Image {
                path: "/a/image.PNG".into(),
            })
        });
        assert_eq!(app.selected.as_deref(), Some("b"));
        assert!(app.layouts["a"].contains(&Tab::Image {
            path: "/a/image.PNG".into()
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
        app.open_file("/a/index.HTML".into(), None, None, false);
        app.select_project("b".into());
        pump_until(&mut app, &ctx, |app| {
            app.layouts["a"].contains(&Tab::browser_file("/a/index.HTML".into()))
        });
        assert_eq!(app.selected.as_deref(), Some("b"));
        assert!(app.layouts["a"].contains(&Tab::browser_file("/a/index.HTML".into())));
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
}
