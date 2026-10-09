use eframe::egui::{self};
#[cfg(test)]
use egui_dock::DockState;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use terminator_core::*;

use super::super::*;
use super::nav_common::*;
#[cfg(test)]
mod tests {
    use super::*;
    /// A tab reorder drag paints a tab-sized ghost that tracks the pointer.
    #[cfg(feature = "test-support")]
    #[test]
    fn tab_drag_ghost_follows_the_pointer() {
        let (mut app, ctx, _dir) = fixture();
        for sid in ["one", "two"] {
            app.state
                .sessions
                .push(session_fixture(sid, SessionKind::Shell));
        }
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        workspace.add("tB".into(), Tab::Terminal("two".into()));
        app.layouts.insert("a".into(), workspace);
        app.selected = Some("a".into());
        app.tab_drag = Some("tB".to_owned());
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
                app.paint_tab_ghost(ui);
            },
        );
        output.textures_delta.clear();
        let ghost = app.fixture_rect(&ctx, "tab-ghost").expect("tab ghost");
        assert!(
            ghost[2] < 180.0 && ghost[2] > 60.0,
            "short tab ghost must hug the title, got {ghost:?}"
        );
        assert_eq!(ghost[3], 32.0);
        assert!(
            (ghost[0] - (pos.x - ghost[2] / 2.0)).abs() < 2.0
                && (ghost[1] - (pos.y - 16.0)).abs() < 2.0,
            "ghost must track the pointer, got {ghost:?}"
        );
        assert_eq!(app.tab_drag, Some("tB".to_owned()));
    }

    /// Short titles stay narrow, long titles grow, and the active tab's
    /// accent underline is painted inside the tab rather than on its edge.
    #[cfg(feature = "test-support")]
    #[test]
    fn workspace_tabs_hug_their_labels_and_underline_inside() {
        let (mut app, ctx, _dir) = fixture();
        let mut short = session_fixture("one", SessionKind::Shell);
        short.label = "one".into();
        let mut long = session_fixture("two", SessionKind::Shell);
        long.label = "codex supervisor".into();
        app.state.sessions.push(short);
        app.state.sessions.push(long);
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        workspace.add("tB".into(), Tab::Terminal("two".into()));
        workspace.active = workspace.tabs[0].id.clone();
        app.layouts.insert("a".into(), workspace);
        app.selected = Some("a".into());
        combined_frame(&mut app, &ctx, vec![]);
        let one = app
            .fixture_rect(&ctx, "workspace-tab:one")
            .expect("short tab");
        let long = app
            .fixture_rect(&ctx, "workspace-tab:codex supervisor")
            .expect("long tab");
        assert!(
            one[2] < long[2] && one[2] < 160.0 && long[2] < 220.0,
            "tabs must hug the label, got one={one:?} long={long:?}"
        );
        let underline = app
            .fixture_rect(&ctx, "workspace-tab-underline:one")
            .expect("active underline");
        let tab_bottom = one[1] + one[3];
        assert!(
            underline[1] > one[1]
                && underline[1] + underline[3] < tab_bottom
                && underline[0] >= one[0]
                && underline[0] + underline[2] <= one[0] + one[2] + 0.5,
            "underline must sit inside the tab, tab={one:?} bar={underline:?}"
        );
        assert!(
            app.fixture_rect(&ctx, "pane-caption:one").is_some(),
            "a lone pane row stays addressable for its menu"
        );
        assert!(
            app.fixture_rect(&ctx, "pane-title:one").is_none(),
            "a lone pane must not repeat the workspace tab title"
        );
    }

    /// Split panes keep a caption so each one can be named and dragged.
    #[cfg(feature = "test-support")]
    #[test]
    fn split_panes_keep_their_captions() {
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
        combined_frame(&mut app, &ctx, vec![]);
        assert!(app.fixture_rect(&ctx, "pane-caption:left").is_some());
        assert!(app.fixture_rect(&ctx, "pane-caption:right").is_some());
    }

    /// Every terminal caption carries Git and both split buttons on one row.
    #[cfg(feature = "test-support")]
    #[test]
    fn terminal_bars_keep_git_and_splits_on_one_row() {
        let (mut app, ctx, _dir) = fixture();
        for sid in ["left", "right"] {
            app.state
                .sessions
                .push(session_fixture(sid, SessionKind::Shell));
        }
        app.metadata = Some(metadata::Metadata {
            cwd: "/a".into(),
            branch: Some("master".into()),
            ..Default::default()
        });
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("left".into())]));
        workspace.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("right".into())],
        );
        app.layouts.insert("a".into(), workspace);
        app.selected = Some("a".into());
        combined_frame(&mut app, &ctx, vec![]);
        for sid in ["left", "right"] {
            let caption = app
                .fixture_rect(&ctx, &format!("pane-caption:{sid}"))
                .expect("caption");
            for (name, action) in [
                ("git", "pane-git"),
                ("vertical", "pane-split-vertical"),
                ("horizontal", "pane-split-horizontal"),
            ] {
                let rect = app
                    .fixture_rect(&ctx, &format!("{action}:{sid}"))
                    .unwrap_or_else(|| panic!("missing {name} on {sid}"));
                assert!(
                    (rect[1] + rect[3] / 2.0 - (caption[1] + caption[3] / 2.0)).abs() < 4.0,
                    "{name} on {sid} should sit on the caption row"
                );
                assert!(
                    rect[0] >= caption[0] && rect[0] + rect[2] <= caption[0] + caption[2] + 1.0,
                    "{name} on {sid} should stay inside the caption"
                );
            }
        }
        let git = frame_center(&app, &ctx, "pane-git:left");
        combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(git)]);
        combined_frame(&mut app, &ctx, vec![frame_press(git, true)]);
        combined_frame(&mut app, &ctx, vec![frame_press(git, false)]);
        assert!(app.preferences.visible);
        assert_eq!(app.preferences.tool, SidebarTool::Git);
        let split = frame_center(&app, &ctx, "pane-split-horizontal:right");
        combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(split)]);
        combined_frame(&mut app, &ctx, vec![frame_press(split, true)]);
        combined_frame(&mut app, &ctx, vec![frame_press(split, false)]);
        assert_eq!(
            app.add_tab
                .as_ref()
                .map(|(_, direction)| direction.as_deref()),
            Some(Some("down"))
        );
    }

    /// A pane dragged over a strip gap shows the tab-sized ghost (the
    /// new-tab outcome), never stacked with the pane snapshot ghost.
    #[cfg(feature = "test-support")]
    #[test]
    fn pane_drag_strip_gap_shows_tab_ghost() {
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
        assert!(!app.strip_tab_hover);
        assert!(app.strip_new_tab_hover);
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 600.0),
                )),
                events: vec![egui::Event::PointerMoved(gap)],
                ..Default::default()
            },
            |ui| {
                app.paint_drag_ghost(ui);
                app.paint_tab_ghost(ui);
            },
        );
        output.textures_delta.clear();
        let ghost = app.fixture_rect(&ctx, "tab-ghost").expect("tab ghost");
        assert!(
            ghost[2] < 180.0 && ghost[2] > 60.0,
            "short tab ghost must hug the title, got {ghost:?}"
        );
        assert_eq!(ghost[3], 32.0);
        assert!(
            (ghost[0] - (gap.x - ghost[2] / 2.0)).abs() < 2.0,
            "tab ghost must track the pointer, got {ghost:?}"
        );
        assert!(
            app.fixture_rect(&ctx, "pane-ghost").is_none(),
            "pane ghost must yield to the tab ghost over the strip"
        );
        assert_eq!(app.pane_drag, Some(Tab::Terminal("one".into())));
    }

    /// Dragging a terminal caption onto another split leaf rearranges the
    /// active tab (single panes swap). Runs headless with synthetic pointer
    /// events; needs test-support for the caption geometry records.
    #[cfg(feature = "test-support")]
    #[test]
    fn pane_caption_drag_swaps_split_terminals() {
        let (mut app, ctx, _dir) = fixture();
        app.state
            .sessions
            .push(session_fixture("left", SessionKind::Shell));
        app.state
            .sessions
            .push(session_fixture("right", SessionKind::Shell));
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("left".into())]));
        workspace.main_surface_mut().split_right(
            egui_dock::NodeIndex::root(),
            0.5,
            vec![Tab::Terminal("right".into())],
        );
        let left_home = workspace
            .find_tab(&Tab::Terminal("left".into()))
            .unwrap()
            .node_path();
        let right_home = workspace
            .find_tab(&Tab::Terminal("right".into()))
            .unwrap()
            .node_path();
        app.layouts.insert("a".into(), workspace);
        app.selected = Some("a".into());
        app.active_session = Some("left".into());
        fn dock_frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let mut dock = app.layouts.remove("a").unwrap_or_else(Workspace::empty);
                    app.paint_dock(ui, "a", &mut dock);
                    app.layouts.insert("a".into(), dock);
                },
            );
            output.textures_delta.clear();
        }
        fn center(rect: [f32; 4]) -> egui::Pos2 {
            egui::pos2(rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0)
        }
        let press = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        dock_frame(&mut app, &ctx, vec![]);
        let start = center(
            app.fixture_rect(&ctx, "pane-drag:left")
                .expect("left caption geometry"),
        );
        let end = center(
            app.fixture_rect(&ctx, "pane-drag:right")
                .expect("right caption geometry"),
        );
        // Aim at the middle of the right split: the caption sits in the top
        // edge band, which would split instead of swapping.
        let middle = egui::pos2(end.x, 300.0);
        dock_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start)]);
        dock_frame(&mut app, &ctx, vec![press(start, true)]);
        assert!(app.pane_drag.is_none());
        for step in 1..=4 {
            let k = f32::from(i16::try_from(step).unwrap_or(0)) / 4.0;
            dock_frame(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(egui::pos2(
                    start.x + (middle.x - start.x) * k,
                    start.y + (middle.y - start.y) * k,
                ))],
            );
        }
        assert_eq!(
            app.pane_drag,
            Some(Tab::Terminal("left".into())),
            "caption drag did not start"
        );
        dock_frame(&mut app, &ctx, vec![press(middle, false)]);
        assert!(app.pane_drag.is_none());
        // A middle drop swaps the two single panes in place: same leaves,
        // exchanged terminals.
        let dock = app.layouts.get("a").unwrap();
        assert_eq!(
            dock.find_tab(&Tab::Terminal("left".into()))
                .unwrap()
                .node_path(),
            right_home,
            "middle drop did not swap into the right split"
        );
        assert_eq!(
            dock.find_tab(&Tab::Terminal("right".into()))
                .unwrap()
                .node_path(),
            left_home,
            "middle drop did not swap into the left split"
        );
    }

    /// Dragging a terminal caption onto another workspace strip tab moves it
    /// across top-level tabs. Needs test-support for geometry records.
    #[cfg(feature = "test-support")]
    #[test]
    fn pane_caption_drop_on_strip_tab_moves_across_groups() {
        let (mut app, ctx, _dir) = fixture();
        app.state
            .sessions
            .push(session_fixture("one", SessionKind::Shell));
        app.state
            .sessions
            .push(session_fixture("two", SessionKind::Shell));
        let mut workspace =
            Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
        workspace.add("tB".into(), Tab::Terminal("two".into()));
        let group_a = workspace.tabs[0].id.clone();
        workspace.active = group_a.clone();
        app.layouts.insert("a".into(), workspace);
        app.selected = Some("a".into());
        app.active_session = Some("one".into());
        fn combined_frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1000.0, 600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    // Same order as the real panels: strip first, dock after.
                    let mut dock = app.layouts.remove("a").unwrap_or_else(Workspace::empty);
                    app.workspace_bar(ui, "a", &mut dock);
                    app.paint_dock(ui, "a", &mut dock);
                    app.layouts.insert("a".into(), dock);
                },
            );
            output.textures_delta.clear();
        }
        fn center(rect: [f32; 4]) -> egui::Pos2 {
            egui::pos2(rect[0] + rect[2] / 2.0, rect[1] + rect[3] / 2.0)
        }
        let press = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        combined_frame(&mut app, &ctx, vec![]);
        let start = center(
            app.fixture_rect(&ctx, "pane-drag:one")
                .expect("caption geometry"),
        );
        let dest = center(
            app.fixture_rect(&ctx, "workspace-tab:two")
                .expect("strip tab geometry"),
        );
        combined_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(start)]);
        combined_frame(&mut app, &ctx, vec![press(start, true)]);
        for step in 1..=4 {
            let k = f32::from(i16::try_from(step).unwrap_or(0)) / 4.0;
            combined_frame(
                &mut app,
                &ctx,
                vec![egui::Event::PointerMoved(egui::pos2(
                    start.x + (dest.x - start.x) * k,
                    start.y + (dest.y - start.y) * k,
                ))],
            );
        }
        assert_eq!(
            app.pane_drag,
            Some(Tab::Terminal("one".into())),
            "caption drag did not start"
        );
        combined_frame(&mut app, &ctx, vec![press(dest, false)]);
        assert!(app.pane_drag.is_none());
        let dock = app.layouts.get("a").unwrap();
        // Both sides held one terminal: they swap instead of hiding one.
        assert_eq!(dock.tabs.len(), 2, "swap must keep both tabs");
        assert_eq!(dock.active, "tB");
        assert!(dock.contains(&Tab::Terminal("one".into())));
        assert!(dock.contains(&Tab::Terminal("two".into())));
    }

    #[test]
    fn attention_migration_retries_without_ack_and_preserves_later_choices() {
        let (mut app, ctx, dir) = fixture();
        let (jobs, requests) = std::sync::mpsc::channel();
        app.jobs = jobs.into();
        app.preferences_writable = true;
        app.migrate_attention();
        assert!(matches!(
            requests.try_recv().unwrap(),
            Job::MigrateAttention
        ));
        assert!(app.attention_requested.is_some());
        assert!(!app.preferences.attention_migrated);
        let unacknowledged = Instant::now().checked_sub(Duration::from_secs(6)).unwrap();
        app.attention_requested = Some(unacknowledged);
        app.migrate_attention();
        assert_eq!(app.attention_requested, Some(unacknowledged));
        assert!(
            requests.try_recv().is_err(),
            "Pending migration must not be duplicated"
        );
        app.update_tx
            .send(Update::AttentionMigrated(Err("rejected".into())))
            .unwrap();
        drain_updates(&mut app, &ctx);
        assert!(!app.preferences.attention_migrated);
        assert!(!app.attention_pending);
        assert!(app.error.as_deref().unwrap().contains("rejected"));
        app.attention_requested = Some(
            Instant::now()
                .checked_sub(Duration::from_secs(6))
                .unwrap_or_else(Instant::now),
        );
        let previous_request = app.attention_requested.unwrap();
        app.migrate_attention();
        assert!(matches!(
            requests.try_recv().unwrap(),
            Job::MigrateAttention
        ));
        assert!(app.attention_pending);
        assert!(app.attention_requested.unwrap() > previous_request);
        app.update_tx
            .send(Update::AttentionMigrated(Ok(())))
            .unwrap();
        drain_updates(&mut app, &ctx);
        app.preferences.save(dir.path()).unwrap();
        assert!(UiPreferences::load(dir.path()).unwrap().attention_migrated);
        app.state.settings.notifications_side = false;
        app.attention_requested = None;
        app.migrate_attention();
        assert!(app.attention_requested.is_none());
        assert!(!app.state.settings.notifications_side);
        assert!(
            requests.try_recv().is_err(),
            "Acknowledged migration must preserve later choices"
        );
    }

    #[test]
    fn opening_settings_keeps_selected_sidebar_and_custom_editor() {
        let (mut app, _, _dir) = fixture();
        app.preferences.tool = SidebarTool::Git;
        app.preferences.visible = false;
        app.state.settings.external_editor = "/custom/editor".into();
        app.state.settings.external_args = vec!["a b".into()];
        app.open_settings();
        assert_eq!(app.preferences.tool, SidebarTool::Git);
        assert!(!app.preferences.visible);
        assert_eq!(app.editor_preset, external_editor::CUSTOM);
        assert_eq!(app.settings_draft.external_args, vec!["a b"]);
    }

    #[test]
    fn failed_migration_does_not_set_marker() {
        let (mut app, ctx, _dir) = fixture();
        app.update_tx
            .send(Update::Error("Settings rejected".into()))
            .unwrap();
        drain_updates(&mut app, &ctx);
        assert!(!app.preferences.typography_migrated);
        app.update_tx.send(Update::TypographyMigrated).unwrap();
        drain_updates(&mut app, &ctx);
        assert!(app.preferences.typography_migrated);
    }

    #[test]
    fn hidden_agent_terminal_keeps_owner_and_directory_context() {
        let (mut app, _, _dir) = fixture();
        app.state.sessions.push(Session {
            review: false,
            id: "hidden".into(),
            project_id: "a".into(),
            label: "Shell".into(),
            cwd: "/b".into(),
            kind: SessionKind::Shell,
            file: None,
            lifecycle: Lifecycle::Running,
            created: 0,
            exit_code: None,
            rows: 24,
            cols: 80,
            generation: "same".into(),
            pid: Some(123),
            truncated: false,
            cwd_confirmed: true,
        });
        app.select_project("b".into());
        app.go_session("hidden");
        assert_eq!(app.selected.as_deref(), Some("a"));
        assert_eq!(app.cwd(), Some(PathBuf::from("/b")));
        assert!(
            app.layouts["a"]
                .find_tab(&Tab::Terminal("hidden".into()))
                .is_some()
        );
        assert_eq!(app.state.sessions[0].pid, Some(123));
        app.terminal_context.insert("a".into(), "hidden".into());
        app.active_session = None; // diff/editor focus retains the preceding shell.
        assert_eq!(app.cwd(), Some(PathBuf::from("/b")));
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn inline_attention_preserves_input_but_missing_session_details_remain_modal() {
        let (mut app, _, _dir) = fixture();
        app.preferences.left_agents = true;
        app.state.sessions = vec![session_fixture("live-shell", SessionKind::Shell)];
        app.state.notifications = vec![notice_fixture(
            "wait",
            "live-shell",
            AgentState::WaitingPermission,
            now(),
        )];
        app.detail = Some("wait".into());
        assert!(!app.notice_detail_modal_open());
        app.preferences.left_agents = false;
        app.preferences.visible = false;
        assert!(app.notice_detail_modal_open());
        app.preferences.left_agents = true;
        app.selected = Some("different-project".into());
        // The inbox spans all projects, so switching projects keeps inline detail.
        assert!(!app.notice_detail_modal_open());
        app.selected = None;
        // Only a missing session drops the notice out of scope.
        let sessions = std::mem::take(&mut app.state.sessions);
        assert!(app.notice_detail_modal_open());
        app.state.sessions = sessions;
        app.state.notifications[0].snoozed_until = now() + 600;
        assert!(app.notice_detail_modal_open());
        app.state.notifications[0].snoozed_until = 0;
        app.state.notifications[0].resolved = true;
        assert!(app.notice_detail_modal_open());
        app.state.notifications[0].resolved = false;
        app.state.notifications[0].dismissed = true;
        assert!(app.notice_detail_modal_open());
    }

    #[test]
    #[cfg(feature = "test-support")]
    fn attention_bell_lives_on_the_left_sidebar_only() {
        let (mut app, ctx, _dir) = fixture();
        app.state.settings.notifications_side = true;
        app.preferences.visible = true;
        app.state.sessions = vec![session_fixture("live-shell", SessionKind::Shell)];
        app.state.notifications = vec![notice_fixture(
            "wait",
            "live-shell",
            AgentState::WaitingPermission,
            now(),
        )];
        let target = |name: &str| {
            ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new(("fixture-target", name))))
        };
        // The right sidebar never paints the bell, even with waiting notices.
        for tool in [
            SidebarTool::History,
            SidebarTool::Git,
            SidebarTool::Explorer,
            SidebarTool::Agents,
            SidebarTool::Info,
        ] {
            app.preferences.tool = tool;
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                if app.preferences.visible {
                    app.sidebar(ui);
                }
            });
            output.textures_delta.clear();
            assert!(target("attention-bell").is_none());
        }
        assert!(target("agent-go:live-shell").is_some());
        // The project header keeps the bell, beside the hide control.
        app.project_width = 360.0;
        paint_header(&mut app, &ctx, &[]);
        assert!(target("left-agent-bar").is_some());
    }
}
