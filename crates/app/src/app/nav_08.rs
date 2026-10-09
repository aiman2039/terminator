use eframe::egui::{self};
#[cfg(test)]
use egui_dock::DockState;
use std::sync::mpsc::{self};
use terminator_core::*;

use super::super::*;
use super::nav_common::*;
#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn background_player_advances_its_own_project_playlist() {
        let (mut app, _, _dir) = fixture();
        app.selected = Some("b".into());
        app.preferences.playlists = vec![crate::preferences::Playlist {
            name: "Default".into(),
            tracks: vec!["/a/one.wav".into(), "/a/two.wav".into()],
        }];
        app.preferences.selected_playlist = "Default".into();
        app.player = player::Controller::finished_fixture("a", Some(0));
        app.poll_player();
        assert_eq!(app.player.project.as_deref(), Some("a"));
        assert_eq!(app.preferences.player_index.get("Default"), Some(&1));
    }

    #[test]
    fn radio_completion_does_not_start_a_playlist() {
        let (mut app, _, _dir) = fixture();
        app.preferences.playlists = vec![crate::preferences::Playlist {
            name: "Default".into(),
            tracks: vec!["/a/one.wav".into(), "/a/two.wav".into()],
        }];
        app.preferences.selected_playlist = "Default".into();
        app.player = player::Controller::finished_fixture("a", None);
        app.poll_player();
        assert!(!app.preferences.player_index.contains_key("Default"));
    }

    #[test]
    fn closing_a_player_tab_does_not_stop_playback() {
        let (mut app, _, _dir) = fixture();
        app.layouts
            .get_mut("a")
            .unwrap()
            .add("player-a".into(), Tab::Player);
        app.player = player::Controller::finished_fixture("a", Some(0));
        app.layouts.get_mut("a").unwrap().close("player-a");
        app.reconcile_gui_resources();
        assert_eq!(app.player.project.as_deref(), Some("a"));
    }

    #[test]
    fn radio_next_wraps_bundled_stations() {
        let (mut app, _, _dir) = fixture();
        app.play_station_at("a", 0);
        assert_eq!(app.player.station_index, Some(0));
        app.play_station_offset("a", 1);
        assert_eq!(app.player.station_index, Some(1));
        app.play_station_offset("a", -1);
        assert_eq!(app.player.station_index, Some(0));
        app.play_station_offset("a", -1);
        assert_eq!(
            app.player.station_index,
            Some(player::radio::catalog().len() - 1)
        );
    }

    #[test]
    fn navigation_retains_identity_and_updates_the_originating_project() {
        let (mut app, ctx, _dir) = fixture();
        app.open_browser_url("a", "https://example.com/start", None)
            .unwrap();
        pump_until(&mut app, &ctx, |app| {
            matches!(app.layouts["a"].active_pane(), Some(Tab::Browser { .. }))
        });
        let key = app.layouts["a"].active_pane().unwrap().key();
        app.selected = Some("b".into());
        let target = BrowserTarget::from_http_url("https://example.com/next").unwrap();
        app.apply_browser_navigation(&key, target.clone());
        assert_eq!(app.layouts["a"].active_pane().unwrap().key(), key);
        assert_eq!(app.browser_urls[&key], "https://example.com/next");
        assert!(
            matches!(app.layouts["a"].active_pane(), Some(Tab::Browser { target: current, .. }) if *current == target)
        );
        assert_eq!(
            app.layouts["a"].tabs[0].primary.as_ref().unwrap().key(),
            key
        );
        let tab_id = app.layouts["a"].active.clone();
        app.layouts.get_mut("a").unwrap().close(&tab_id);
        app.reconcile_gui_resources();
        assert!(!app.browser_urls.contains_key(&key));
    }

    #[test]
    fn audio_open_plays_without_a_player_tab_and_keeps_original_project() {
        let (mut app, ctx, _dir) = fixture();
        let (jobs, requests) = mpsc::channel();
        app.jobs = jobs.into();
        app.open_file(crate::test_path("/a/song.MP3"), None, None, false);
        app.select_project("b".into());
        app.process_updates(&ctx);
        assert_eq!(app.selected.as_deref(), Some("b"));
        assert!(!app.layouts["a"].contains(&Tab::Player));
        assert_eq!(app.player.project.as_deref(), Some("a"));
        assert!(app.state.sessions.is_empty());
        assert!(!requests.try_iter().any(|j|matches!(j,Job::Control(request,_) if matches!(*request,Request::Create { editor:true,.. }))));
        assert_eq!(app.preferences.selected_playlist, "Default");
        assert_eq!(
            app.preferences.selected_tracks(),
            [crate::test_path("/a/song.MP3")].as_slice()
        );
    }

    #[test]
    fn adding_audio_files_appends_to_the_selected_playlist() {
        let (mut app, _, _dir) = fixture();
        app.add_audio_files(vec![
            crate::test_path("/a/one.MP3"),
            crate::test_path("/a/two.flac"),
            crate::test_path("/a/notes.txt"),
        ]);
        assert_eq!(
            app.preferences.selected_tracks(),
            [
                crate::test_path("/a/one.MP3"),
                crate::test_path("/a/two.flac")
            ]
            .as_slice()
        );
        assert_eq!(app.player.project.as_deref(), Some("a"));
        app.add_audio_files(vec![crate::test_path("/a/three.ogg")]);
        assert_eq!(app.preferences.selected_tracks().len(), 3);
        assert_eq!(app.player.project.as_deref(), Some("a"));
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
        app.process_updates(&ctx);
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

    #[test]
    fn drag_ghost_paints_without_changing_state() {
        let (mut app, ctx, _dir) = fixture();
        app.state
            .sessions
            .push(session_fixture("one", SessionKind::Shell));
        app.pane_drag = Some(Tab::Terminal("one".into()));
        let pos = egui::pos2(500.0, 300.0);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 600.0),
                )),
                events: vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::default(),
                    },
                ],
                ..Default::default()
            },
            |ui| {
                app.paint_drag_ghost(ui);
            },
        );
        output.textures_delta.clear();
        assert_eq!(app.pane_drag, Some(Tab::Terminal("one".into())));
    }

    /// Hovering another strip tab mid-drag previews its splits; dropping on
    /// one of its leaves lands the terminal precisely there.
    #[cfg(feature = "test-support")]
    #[test]
    fn pane_drag_preview_switches_tab_and_drops_into_its_leaf() {
        let (mut app, ctx, _dir) = fixture();
        for sid in ["one", "two", "three"] {
            app.state
                .sessions
                .push(session_fixture(sid, SessionKind::Shell));
        }
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        workspace.add("tB".into(), Tab::Terminal("two".into()));
        workspace.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("three".into())],
        );
        let group_a = workspace.tabs[0].id.clone();
        workspace.active = group_a.clone();
        app.layouts.insert("a".into(), workspace);
        app.selected = Some("a".into());
        app.active_session = Some("one".into());
        combined_frame(&mut app, &ctx, vec![]);
        let start = frame_center(&app, &ctx, "pane-drag:one");
        let dest = frame_center(&app, &ctx, "workspace-tab:two");
        combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start)]);
        combined_frame(&mut app, &ctx, vec![frame_press(start, true)]);
        frame_glide(&mut app, &ctx, start, dest, 4);
        // The destination tab content is previewed while hovering its strip
        // tab, remembering the origin tab.
        assert_eq!(app.layouts["a"].active, "tB");
        assert_eq!(
            app.drop_preview_origin,
            Some(("a".to_owned(), group_a.clone()))
        );
        let leaf_caption = frame_center(&app, &ctx, "pane-drag:three");
        // Aim at the middle of the previewed split: its caption sits in the
        // top edge band, which would split instead of swapping.
        let leaf = egui::pos2(leaf_caption.x, 320.0);
        frame_glide(&mut app, &ctx, dest, leaf, 3);
        combined_frame(&mut app, &ctx, vec![frame_press(leaf, false)]);
        assert!(app.pane_drag.is_none());
        assert!(app.drop_preview_origin.is_none());
        let dock = app.layouts.get("a").unwrap();
        // Single panes swap: both tabs survive with everything visible.
        assert_eq!(dock.tabs.len(), 2);
        assert_eq!(dock.active, "tB");
        // "one" landed in the targeted previewed leaf.
        let landed = dock
            .find_tab(&Tab::Terminal("one".into()))
            .unwrap()
            .node_path();
        assert!(
            dock.leaf(landed).unwrap().rect.contains(leaf),
            "drop missed the targeted leaf"
        );
        // "three" swapped back into the origin tab.
        let origin = dock
            .tabs
            .iter()
            .find(|tab| tab.id != "tB")
            .expect("origin tab survives the swap");
        assert!(
            origin
                .layout
                .find_tab(&Tab::Terminal("three".into()))
                .is_some()
        );
    }

    /// Dropping a pane near the edge of another split opens it in a new
    /// split beside that leaf instead of swapping.
    #[cfg(feature = "test-support")]
    #[test]
    fn pane_drag_edge_drop_opens_a_split() {
        let (mut app, ctx, _dir) = fixture();
        for sid in ["left", "right"] {
            app.state
                .sessions
                .push(session_fixture(sid, SessionKind::Shell));
        }
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("left".into())]));
        workspace.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("right".into())],
        );
        app.layouts.insert("a".into(), workspace);
        app.selected = Some("a".into());
        app.active_session = Some("left".into());
        combined_frame(&mut app, &ctx, vec![]);
        let start = frame_center(&app, &ctx, "pane-drag:left");
        let right = app
            .fixture_rect(&ctx, "pane-drag:right")
            .expect("caption geometry");
        // Near the right edge of the right split, vertically centered on
        // its caption so the top band cannot win the zone.
        let edge = egui::pos2(right[0] + right[2] - 4.0, right[1] + right[3] / 2.0);
        combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start)]);
        combined_frame(&mut app, &ctx, vec![frame_press(start, true)]);
        frame_glide(&mut app, &ctx, start, edge, 4);
        assert_eq!(
            app.pane_drag,
            Some(Tab::Terminal("left".into())),
            "caption drag did not start"
        );
        combined_frame(&mut app, &ctx, vec![frame_press(edge, false)]);
        assert!(app.pane_drag.is_none());
        // One more frame so leaf rectangles reflect the new split.
        combined_frame(&mut app, &ctx, vec![]);
        let dock = app.layouts.get("a").unwrap();
        assert_eq!(dock.iter_leaves().count(), 2);
        let left_rect = dock
            .find_tab(&Tab::Terminal("left".into()))
            .map(|path| dock.leaf(path.node_path()).unwrap().rect)
            .unwrap();
        let right_rect = dock
            .find_tab(&Tab::Terminal("right".into()))
            .map(|path| dock.leaf(path.node_path()).unwrap().rect)
            .unwrap();
        assert!(
            left_rect.center().x > right_rect.center().x,
            "edge drop did not split right"
        );
        assert_eq!(dock.active_pane(), Some(&Tab::Terminal("left".into())));
    }

    /// Dragging a lower-pane tab up into the main dock lands that shell there.
    /// IDE mode stays on and the editor stays.
    #[cfg(feature = "test-support")]
    #[test]
    fn strip_tab_drag_lands_in_the_main_pane() {
        let (mut app, ctx, _dir) = fixture();
        app.selected = Some("a".into());
        app.preferences.ide_mode = true;
        app.state.sessions = vec![
            session_fixture("low", SessionKind::Shell),
            session_fixture("edit", SessionKind::Editor),
        ];
        app.preferences.ide_strip_docks.0.insert(
            "a".into(),
            egui_dock::DockState::new(vec![Tab::Terminal("low".into())]),
        );
        app.layouts.insert(
            "a".into(),
            Workspace::from_layout(egui_dock::DockState::new(vec![Tab::Terminal(
                "edit".into(),
            )])),
        );
        cross_dock_frame(&mut app, &ctx, vec![]);
        let start = frame_center(&app, &ctx, "strip-tab:low");
        let caption = frame_center(&app, &ctx, "pane-drag:edit");
        let strip = app
            .fixture_rect(&ctx, "ide-terminal-strip")
            .expect("strip geometry");
        let drop_at = egui::pos2(caption.x, (caption.y + strip[1]) * 0.5);
        cross_dock_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start)]);
        cross_dock_frame(&mut app, &ctx, vec![frame_press(start, true)]);
        cross_glide(&mut app, &ctx, start, drop_at, 8);
        assert!(
            app.pane_drag_from_strip,
            "leaving the strip did not promote the tab drag"
        );
        cross_dock_frame(&mut app, &ctx, vec![frame_press(drop_at, false)]);
        assert!(app.pane_drag.is_none());
        assert!(app.preferences.ide_mode);
        assert!(!app.is_strip_session("a", "low"));
        let dock = app.layouts.get("a").unwrap();
        assert!(dock.contains(&Tab::Terminal("low".into())));
        assert!(dock.contains(&Tab::Terminal("edit".into())));
    }

    /// Dragging a main-pane shell down onto the IDE strip moves only that shell.
    #[cfg(feature = "test-support")]
    #[test]
    fn main_caption_drag_lands_in_the_strip() {
        let (mut app, ctx, _dir) = fixture();
        app.selected = Some("a".into());
        app.preferences.ide_mode = true;
        app.preferences.ide_terminal_collapsed = true;
        app.state.sessions = vec![
            session_fixture("up", SessionKind::Shell),
            session_fixture("edit", SessionKind::Editor),
        ];
        let mut main = egui_dock::DockState::new(vec![Tab::Terminal("edit".into())]);
        main.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("up".into())],
        );
        app.layouts.insert("a".into(), Workspace::from_layout(main));
        cross_dock_frame(&mut app, &ctx, vec![]);
        let start = frame_center(&app, &ctx, "pane-drag:up");
        let strip = app
            .fixture_rect(&ctx, "ide-terminal-strip")
            .expect("strip geometry");
        let drop_at = egui::pos2(strip[0] + strip[2] * 0.5, strip[1] + strip[3] * 0.5);
        cross_dock_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start)]);
        cross_dock_frame(&mut app, &ctx, vec![frame_press(start, true)]);
        cross_glide(&mut app, &ctx, start, drop_at, 8);
        assert_eq!(app.pane_drag, Some(Tab::Terminal("up".into())));
        assert!(!app.pane_drag_from_strip);
        cross_dock_frame(&mut app, &ctx, vec![frame_press(drop_at, false)]);
        assert!(app.pane_drag.is_none());
        assert!(app.preferences.ide_mode);
        assert!(!app.preferences.ide_terminal_collapsed);
        assert!(app.is_strip_session("a", "up"));
        assert!(!app.is_strip_session("a", "edit"));
        let dock = app.layouts.get("a").unwrap();
        assert!(!dock.contains(&Tab::Terminal("up".into())));
        assert!(dock.contains(&Tab::Terminal("edit".into())));
    }

    /// Cancelling a pane drag (Esc) after previewing another tab switches
    /// back to the origin tab without moving anything.
    #[cfg(feature = "test-support")]
    #[test]
    fn pane_drag_cancel_restores_origin_tab() {
        let (mut app, ctx, _dir) = fixture();
        for sid in ["one", "two"] {
            app.state
                .sessions
                .push(session_fixture(sid, SessionKind::Shell));
        }
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        workspace.add("tB".into(), Tab::Terminal("two".into()));
        let group_a = workspace.tabs[0].id.clone();
        workspace.active = group_a.clone();
        app.layouts.insert("a".into(), workspace);
        app.selected = Some("a".into());
        combined_frame(&mut app, &ctx, vec![]);
        let start = frame_center(&app, &ctx, "pane-drag:one");
        let dest = frame_center(&app, &ctx, "workspace-tab:two");
        combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start)]);
        combined_frame(&mut app, &ctx, vec![frame_press(start, true)]);
        combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(dest)]);
        combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(dest)]);
        assert_eq!(app.layouts["a"].active, "tB");
        combined_frame(
            &mut app,
            &ctx,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::default(),
            }],
        );
        assert!(app.pane_drag.is_none());
        assert!(app.drop_preview_origin.is_none());
        let dock = app.layouts.get("a").unwrap();
        assert_eq!(dock.active, group_a);
        assert_eq!(dock.tabs.len(), 2);
        assert!(dock.contains(&Tab::Terminal("one".into())));
    }

    /// Dropping a dragged pane into a gap between strip tabs opens it in a
    /// fresh top-level tab at that slot instead of appending at the end.
    #[cfg(feature = "test-support")]
    #[test]
    fn pane_drag_gap_drop_creates_tab_at_slot() {
        let (mut app, ctx, _dir) = fixture();
        for sid in ["one", "two", "three"] {
            app.state
                .sessions
                .push(session_fixture(sid, SessionKind::Shell));
        }
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        workspace.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("three".into())],
        );
        workspace.add("tB".into(), Tab::Terminal("two".into()));
        let group_a = workspace.tabs[0].id.clone();
        workspace.active = group_a.clone();
        app.layouts.insert("a".into(), workspace);
        app.selected = Some("a".into());
        app.active_session = Some("one".into());
        combined_frame(&mut app, &ctx, vec![]);
        let tab_a = app
            .fixture_rect(&ctx, "workspace-tab:one")
            .expect("tab geometry");
        let tab_b = app
            .fixture_rect(&ctx, "workspace-tab:two")
            .expect("tab geometry");
        let (left, right) = if tab_a[0] < tab_b[0] {
            (tab_a, tab_b)
        } else {
            (tab_b, tab_a)
        };
        let gap = egui::pos2(
            (left[0] + left[2] + right[0]) / 2.0,
            left[1] + left[3] / 2.0,
        );
        let start = frame_center(&app, &ctx, "pane-drag:one");
        combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start)]);
        combined_frame(&mut app, &ctx, vec![frame_press(start, true)]);
        frame_glide(&mut app, &ctx, start, gap, 4);
        combined_frame(&mut app, &ctx, vec![frame_press(gap, false)]);
        assert!(app.pane_drag.is_none());
        assert!(app.drop_preview_origin.is_none());
        let dock = app.layouts.get("a").unwrap();
        assert_eq!(dock.tabs.len(), 3);
        assert!(
            dock.tabs[0]
                .layout
                .find_tab(&Tab::Terminal("three".into()))
                .is_some()
        );
        assert!(
            dock.tabs[1]
                .layout
                .find_tab(&Tab::Terminal("one".into()))
                .is_some()
        );
        assert!(
            dock.tabs[2]
                .layout
                .find_tab(&Tab::Terminal("two".into()))
                .is_some()
        );
        assert_eq!(dock.active, dock.tabs[1].id);
    }

    /// Dragging a strip tab reorders the top-level tabs to the drop slot.
    #[cfg(feature = "test-support")]
    #[test]
    fn strip_tab_drag_reorders_top_level_tabs() {
        let (mut app, ctx, _dir) = fixture();
        for sid in ["one", "two"] {
            app.state
                .sessions
                .push(session_fixture(sid, SessionKind::Shell));
        }
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        workspace.add("tB".into(), Tab::Terminal("two".into()));
        let group_a = workspace.tabs[0].id.clone();
        app.layouts.insert("a".into(), workspace);
        app.selected = Some("a".into());
        app.active_session = Some("two".into());
        combined_frame(&mut app, &ctx, vec![]);
        let start = frame_center(&app, &ctx, "workspace-tab:two");
        let first = app
            .fixture_rect(&ctx, "workspace-tab:one")
            .expect("tab geometry");
        // Inside the first tab's left edge band: a gap for insertion math
        // while still on the strip for the drop.
        let dest = egui::pos2(first[0] + 5.0, first[1] + first[3] / 2.0);
        combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start)]);
        combined_frame(&mut app, &ctx, vec![frame_press(start, true)]);
        frame_glide(&mut app, &ctx, start, dest, 4);
        assert_eq!(app.tab_drag, Some("tB".to_owned()));
        combined_frame(&mut app, &ctx, vec![frame_press(dest, false)]);
        assert!(app.tab_drag.is_none());
        let dock = app.layouts.get("a").unwrap();
        assert_eq!(dock.ids(), vec!["tB".to_owned(), group_a.clone()]);
        assert_eq!(dock.active, "tB");
    }

    /// Hovering a strip tab mid pane-drag washes the previewed tab's
    /// focused leaf at real size, showing where a release would land.
    #[cfg(feature = "test-support")]
    #[test]
    fn pane_drag_strip_hover_washes_the_landing_leaf() {
        let (mut app, ctx, _dir) = fixture();
        for sid in ["one", "two", "three"] {
            app.state
                .sessions
                .push(session_fixture(sid, SessionKind::Shell));
        }
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        workspace.add("tB".into(), Tab::Terminal("two".into()));
        workspace.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("three".into())],
        );
        let group_a = workspace.tabs[0].id.clone();
        workspace.active = group_a.clone();
        app.layouts.insert("a".into(), workspace);
        app.selected = Some("a".into());
        app.active_session = Some("one".into());
        combined_frame(&mut app, &ctx, vec![]);
        let start = frame_center(&app, &ctx, "pane-drag:one");
        let dest = frame_center(&app, &ctx, "workspace-tab:two");
        combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start)]);
        combined_frame(&mut app, &ctx, vec![frame_press(start, true)]);
        frame_glide(&mut app, &ctx, start, dest, 4);
        combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(dest)]);
        combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(dest)]);
        // Still dragging: the destination tab is previewed, nothing moved.
        assert_eq!(app.pane_drag, Some(Tab::Terminal("one".into())));
        assert_eq!(app.layouts["a"].active, "tB");
        let wash = app
            .fixture_rect(&ctx, "strip-drop-wash")
            .expect("landing wash");
        assert!(
            wash[2] > 200.0 && wash[3] > 100.0,
            "wash must cover the landing leaf at real size, got {wash:?}"
        );
    }

    /// Dragging a pane caption onto the strip + opens it in a fresh
    /// top-level tab (the GUI drag-detach flow: press-hold on the
    /// caption, glide to the strip +, release).
    #[cfg(feature = "test-support")]
    #[test]
    fn pane_drag_onto_strip_plus_opens_new_tab() {
        let (mut app, ctx, _dir) = fixture();
        for sid in ["one", "two"] {
            app.state
                .sessions
                .push(session_fixture(sid, SessionKind::Shell));
        }
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        workspace.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("two".into())],
        );
        app.layouts.insert("a".into(), workspace);
        app.selected = Some("a".into());
        app.active_session = Some("one".into());
        combined_frame(&mut app, &ctx, vec![]);
        let start = frame_center(&app, &ctx, "pane-caption:one");
        let dest = frame_center(&app, &ctx, "workspace-plus");
        combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start)]);
        combined_frame(&mut app, &ctx, vec![frame_press(start, true)]);
        frame_glide(&mut app, &ctx, start, dest, 4);
        assert_eq!(app.pane_drag, Some(Tab::Terminal("one".into())));
        combined_frame(&mut app, &ctx, vec![frame_press(dest, false)]);
        assert!(app.pane_drag.is_none());
        let dock = app.layouts.get("a").unwrap();
        assert_eq!(dock.tabs.len(), 2, "drop missed the strip +");
        assert!(
            dock.tabs
                .iter()
                .any(|tab| tab.layout.find_tab(&Tab::Terminal("one".into())).is_some())
                && dock
                    .tabs
                    .iter()
                    .any(|tab| tab.layout.find_tab(&Tab::Terminal("two".into())).is_some()),
            "panes lost in the detach"
        );
    }

    /// A lone terminal pane offers a caption `+` that stacks a tab in its
    /// own split: the same leaf-anchored request as the leaf tab bar `+`,
    /// which a lone pane hides.
    #[cfg(feature = "test-support")]
    #[test]
    fn caption_stack_button_queues_in_split_tab() {
        let (mut app, ctx, _dir) = fixture();
        app.state
            .sessions
            .push(session_fixture("one", SessionKind::Shell));
        app.layouts.insert(
            "a".into(),
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())])),
        );
        app.selected = Some("a".into());
        combined_frame(&mut app, &ctx, vec![]);
        let at = frame_center(&app, &ctx, "pane-stack:one");
        combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
        combined_frame(&mut app, &ctx, vec![frame_press(at, true)]);
        combined_frame(&mut app, &ctx, vec![frame_press(at, false)]);
        let path = app.layouts["a"]
            .find_tab(&Tab::Terminal("one".into()))
            .expect("pane placed")
            .node_path();
        assert_eq!(app.add_tab, Some((path, None)));
    }
}
