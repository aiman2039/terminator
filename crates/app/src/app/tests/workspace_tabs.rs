use super::*;

#[test]
#[cfg(feature = "test-support")]
fn single_terminal_tab_paints_the_terminal_without_a_node() {
    let (mut app, ctx, _dir) = fixture();
    app.state.sessions = vec![
        session_fixture("s1", SessionKind::Shell),
        session_fixture("s2", SessionKind::Shell),
    ];
    let tab = |id: &str, sid: &str| crate::workspace::WorkspaceTab {
        id: id.into(),
        name: None,
        pinned: false,
        primary: Some(Tab::Terminal(sid.into())),
        layout: egui_dock::DockState::new(vec![Tab::Terminal(sid.into())]),
    };
    let mut workspace =
        Workspace::from_layout(egui_dock::DockState::new(vec![Tab::Terminal("s1".into())]));
    workspace.tabs[0] = tab("g1", "s1");
    workspace.tabs.push(tab("g2", "s2"));
    workspace.active = "g1".into();
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());

    let mut time = 0.0_f64;
    let mut paint = |app: &mut App| {
        time += 0.05;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(360.0, 600.0),
                )),
                time: Some(time),
                ..Default::default()
            },
            |ui| app.projects(ui),
        );
        output.textures_delta.clear();
    };
    paint(&mut app);
    paint(&mut app);
    // Both terminals are painted at the tab level; no tab nodes.
    assert!(header_target(&ctx)("session-row:s1").is_some());
    assert!(header_target(&ctx)("session-row:s2").is_some());
    assert!(header_target(&ctx)("sidebar-tab:g1").is_none());
    assert!(header_target(&ctx)("sidebar-tab:g2").is_none());

    // A custom name restores the node, so the name stays visible.
    app.layouts.get_mut("a").unwrap().tabs[0].name = Some("Agents".into());
    paint(&mut app);
    paint(&mut app);
    assert!(header_target(&ctx)("sidebar-tab:g1").is_some());
    assert!(header_target(&ctx)("session-row:s1").is_some());
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
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
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

/// Only a user-chosen title color paints a terminal tab's bottom line; a
/// tab without one has no accent line, so the accent marks the active tab.
#[cfg(feature = "test-support")]
#[test]
fn terminal_tabs_without_a_chosen_color_paint_no_accent_line() {
    let (mut app, ctx, _dir) = fixture();
    for sid in ["one", "two"] {
        app.state
            .sessions
            .push(session_fixture(sid, SessionKind::Shell));
    }
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
    workspace.add("tB".into(), Tab::Terminal("two".into()));
    workspace.active = workspace.tabs[0].id.clone();
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());
    let line_colors = |app: &mut App| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 200.0),
                )),
                ..Default::default()
            },
            |ui| {
                let mut dock = app.layouts.remove("a").unwrap_or_else(Workspace::empty);
                app.workspace_bar(ui, "a", &mut dock);
                app.layouts.insert("a".into(), dock);
            },
        );
        output.textures_delta.clear();
        output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::LineSegment { stroke, .. } => Some(stroke.color),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    line_colors(&mut app);
    let accent = appearance::color(&app.theme.accent);
    assert!(
        !line_colors(&mut app).contains(&accent),
        "no terminal tab may paint an accent bottom line"
    );
    app.preferences
        .terminal_styles
        .entry("two".into())
        .or_default()
        .color = Some("#5AA9FF".into());
    assert!(
        line_colors(&mut app).contains(&appearance::color("#5AA9FF")),
        "a chosen color paints the tab line"
    );
}

/// A finished agent is not painted in the accent: in Carbon the accent is
/// almost the red of a failure.
#[test]
fn completed_agents_use_the_text_color_not_the_accent() {
    let theme = AppearanceConfig::default();
    let done = crate::sidebar_ui::state_color(AgentState::Completed, &theme);
    assert_eq!(done, appearance::color(&theme.text));
    assert_ne!(done, appearance::color(&theme.accent));
    assert_ne!(
        done,
        crate::sidebar_ui::state_color(AgentState::Failed, &theme)
    );
}

/// The tab close glyph uses the same icon size as the strip's `+` and menu
/// controls, which keep the compact toolbar square.
#[cfg(feature = "test-support")]
#[test]
fn workspace_tab_close_glyph_matches_the_strip_controls() {
    let (mut app, ctx, _dir) = fixture();
    app.state
        .sessions
        .push(session_fixture("one", SessionKind::Shell));
    let workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())]));
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());
    let paint = |app: &mut App| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 200.0),
                )),
                ..Default::default()
            },
            |ui| {
                let mut dock = app.layouts.remove("a").unwrap_or_else(Workspace::empty);
                app.workspace_bar(ui, "a", &mut dock);
                app.layouts.insert("a".into(), dock);
            },
        );
        output.textures_delta.clear();
        painted_icon_rects(&output.shapes)
    };
    // The first frame loads the icon textures.
    paint(&mut app);
    let icons = paint(&mut app);
    let close = agent_target(&ctx, "workspace-close:one").expect("close slot");
    let glyph = icons
        .iter()
        .find(|icon| close.contains(icon.center()))
        .expect("the active tab paints its close glyph");
    assert_eq!(glyph.size(), egui::Vec2::splat(appearance::TOOLBAR_ICON));
    let plus = agent_target(&ctx, "workspace-plus").expect("plus button");
    assert_eq!(plus.size(), egui::Vec2::splat(appearance::TOOLBAR_BUTTON));
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
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("left".into())]));
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

/// The focused caption carries the branch and both split buttons on one
/// row; a split button of another pane works once the pointer is on it.
#[cfg(feature = "test-support")]
#[test]
fn focused_caption_keeps_branch_and_splits_on_one_row() {
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
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("left".into())]));
    workspace.main_surface_mut().split_right(
        egui_dock::NodeIndex::root(),
        0.5,
        vec![Tab::Terminal("right".into())],
    );
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());
    app.active_session = Some("left".into());
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
        app.dock_ops.iter().find_map(|op| match op {
            DockOp::AddTab {
                target: DockTarget::Main,
                split,
                ..
            } => Some(split.as_deref()),
            _ => None,
        }),
        Some(Some("down"))
    );
}

// Pane captions: buttons only on the focused or hovered pane, a folded
// menu on narrow panes, and the steady focus rule.

/// Two terminals side by side in project `a`, `left` focused.
fn split_pair(app: &mut App) {
    for sid in ["left", "right"] {
        app.state
            .sessions
            .push(session_fixture(sid, SessionKind::Shell));
    }
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("left".into())]));
    workspace.main_surface_mut().split_right(
        egui_dock::NodeIndex::root(),
        0.5,
        vec![Tab::Terminal("right".into())],
    );
    // The dock's focused leaf drives the active session each frame.
    let surface = workspace.main_surface_mut();
    let left = surface.find_tab(&Tab::Terminal("left".into())).unwrap().0;
    surface.set_focused_node(left);
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());
    app.focus_terminal("left".into());
}

/// One frame of the workspace at `width`; returns the painted icon rects.
#[cfg(feature = "test-support")]
fn paint_at(
    app: &mut App,
    ctx: &egui::Context,
    width: f32,
    events: Vec<egui::Event>,
) -> Vec<egui::Rect> {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 600.0),
            )),
            events,
            ..Default::default()
        },
        |ui| app.workspace_project(ui, "a".into()),
    );
    output.textures_delta.clear();
    painted_icon_rects(&output.shapes)
}

#[cfg(feature = "test-support")]
fn rect_of(app: &App, ctx: &egui::Context, name: &str) -> egui::Rect {
    let [x, y, w, h] = app
        .fixture_rect(ctx, name)
        .unwrap_or_else(|| panic!("missing {name}"));
    egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h))
}

/// Toolbar-sized glyphs right of the caption's title: its buttons.
#[cfg(feature = "test-support")]
fn caption_buttons(icons: &[egui::Rect], caption: egui::Rect) -> usize {
    icons
        .iter()
        .filter(|icon| {
            caption.contains(icon.center())
                && icon.center().x > caption.center().x
                && (icon.width() - appearance::TOOLBAR_ICON).abs() < 0.5
        })
        .count()
}

#[cfg(feature = "test-support")]
#[test]
fn only_the_focused_or_hovered_caption_shows_its_buttons() {
    let (mut app, ctx, _dir) = fixture();
    split_pair(&mut app);
    paint_at(&mut app, &ctx, 1000.0, vec![]);
    let icons = paint_at(&mut app, &ctx, 1000.0, vec![]);
    let left = rect_of(&app, &ctx, "pane-caption:left");
    let right = rect_of(&app, &ctx, "pane-caption:right");
    // Split right, split down, zoom and close: no Git and no New tab.
    assert_eq!(caption_buttons(&icons, left), 4, "focused pane");
    assert_eq!(caption_buttons(&icons, right), 0, "pane without focus");
    assert!(app.fixture_rect(&ctx, "pane-stack:left").is_none());
    let hover = vec![egui::Event::PointerMoved(right.center())];
    paint_at(&mut app, &ctx, 1000.0, hover.clone());
    let icons = paint_at(&mut app, &ctx, 1000.0, hover);
    assert_eq!(caption_buttons(&icons, right), 4, "hovered pane");
}

#[cfg(feature = "test-support")]
#[test]
fn a_narrow_pane_folds_its_buttons_into_one_menu() {
    let (mut app, ctx, _dir) = fixture();
    split_pair(&mut app);
    paint_at(&mut app, &ctx, 360.0, vec![]);
    let icons = paint_at(&mut app, &ctx, 360.0, vec![]);
    let left = rect_of(&app, &ctx, "pane-caption:left");
    let more = rect_of(&app, &ctx, "pane-more:left");
    assert!(app.fixture_rect(&ctx, "pane-split-vertical:left").is_none());
    assert_eq!(caption_buttons(&icons, left), 1);
    assert!(left.contains(more.center()));
    // The title keeps its minimum width before the menu button.
    assert!(more.left() - left.left() >= crate::appearance::TERMINAL_TITLE_MIN);
}

#[cfg(feature = "test-support")]
#[test]
fn the_focused_pane_has_a_steady_accent_rule_on_its_top_edge() {
    let (mut app, ctx, _dir) = fixture();
    split_pair(&mut app);
    paint_at(&mut app, &ctx, 1000.0, vec![]);
    paint_at(&mut app, &ctx, 1000.0, vec![]);
    let left = rect_of(&app, &ctx, "pane-caption:left");
    let rule = rect_of(&app, &ctx, "pane-focus-rule");
    assert!((rule.height() - 2.0).abs() < 0.01);
    assert!(rule.bottom() <= left.top() + 0.5, "rule above the caption");
    assert!(rule.left() <= left.left() && rule.right() >= left.right());
    // Long after the focus flash faded, the rule is still there.
    let now = std::time::Instant::now();
    app.highlight_since = now
        .checked_sub(std::time::Duration::from_secs(5))
        .unwrap_or(now);
    paint_at(&mut app, &ctx, 1000.0, vec![]);
    assert!(app.fixture_rect(&ctx, "pane-focus-rule").is_some());
}

#[cfg(feature = "test-support")]
#[test]
fn a_lone_pane_gets_no_focus_rule() {
    let (mut app, ctx, _dir) = fixture();
    app.state
        .sessions
        .push(session_fixture("one", SessionKind::Shell));
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(DockState::new(vec![Tab::Terminal("one".into())])),
    );
    app.selected = Some("a".into());
    app.active_session = Some("one".into());
    paint_at(&mut app, &ctx, 1000.0, vec![]);
    paint_at(&mut app, &ctx, 1000.0, vec![]);
    assert!(app.fixture_rect(&ctx, "pane-focus-rule").is_none());
}

#[test]
fn a_focused_editor_pane_is_marked_but_a_strip_terminal_marks_no_main_pane() {
    let (mut app, _ctx, _dir) = fixture();
    split_pair(&mut app);
    let editor = Tab::NativeEditor {
        path: "/a/main.rs".into(),
    };
    let surface = app.layouts.get_mut("a").unwrap().main_surface_mut();
    let right = surface.find_tab(&Tab::Terminal("right".into())).unwrap().0;
    surface.set_focused_node(right);
    if let Ok(leaf) = surface.leaf_mut(right) {
        leaf.tabs = vec![editor];
    }
    app.focus_non_terminal();
    let surface = app.layouts["a"].main_surface();
    assert!(
        app.focused_main_pane(surface).is_some(),
        "editor pane marked"
    );
    app.focus_terminal("strip-shell".into());
    let surface = app.layouts["a"].main_surface();
    assert!(
        app.focused_main_pane(surface).is_none(),
        "a terminal outside the main dock marks no main pane"
    );
}

#[cfg(feature = "test-support")]
#[test]
fn a_hidden_caption_button_takes_no_click_but_a_shown_one_does() {
    let (mut app, ctx, _dir) = fixture();
    split_pair(&mut app);
    combined_frame(&mut app, &ctx, vec![]);
    combined_frame(&mut app, &ctx, vec![]);
    let split = frame_center(&app, &ctx, "pane-split-horizontal:right");
    let adds = |app: &App| {
        app.dock_ops
            .iter()
            .filter(|op| matches!(op, DockOp::AddTab { .. }))
            .count()
    };
    // The pointer arrives and presses in one frame: the button was hidden.
    combined_frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(split), frame_press(split, true)],
    );
    combined_frame(&mut app, &ctx, vec![frame_press(split, false)]);
    assert_eq!(adds(&app), 0, "a hidden split button must not split");
    // Now it shows under the pointer, so a click splits.
    combined_frame(&mut app, &ctx, vec![frame_press(split, true)]);
    combined_frame(&mut app, &ctx, vec![frame_press(split, false)]);
    assert_eq!(adds(&app), 1);
}

#[cfg(feature = "test-support")]
#[test]
fn a_stacked_leaf_shows_its_tab_bar_without_a_second_caption_row() {
    let (mut app, ctx, _dir) = fixture();
    split_pair(&mut app);
    app.state
        .sessions
        .push(session_fixture("third", SessionKind::Shell));
    let surface = app.layouts.get_mut("a").unwrap().main_surface_mut();
    let right = surface.find_tab(&Tab::Terminal("right".into())).unwrap().0;
    if let Ok(leaf) = surface.leaf_mut(right) {
        leaf.tabs.push(Tab::Terminal("third".into()));
    }
    for _ in 0..3 {
        combined_frame(&mut app, &ctx, vec![]);
    }
    // The left leaf has one pane: its caption stays.
    assert!(app.fixture_rect(&ctx, "pane-caption:left").is_some());
    // The stacked leaf titles its panes on its tab bar.
    assert!(app.fixture_rect(&ctx, "pane-caption:right").is_none());
    assert!(app.fixture_rect(&ctx, "pane-caption:third").is_none());
}

/// Thirteen one-terminal tabs in project `a`, the last one active.
#[cfg(feature = "test-support")]
fn many_tabs(app: &mut App) {
    let tab = |n: usize| crate::workspace::WorkspaceTab {
        id: format!("g{n}"),
        name: None,
        pinned: false,
        primary: Some(Tab::Terminal(format!("s{n}"))),
        layout: egui_dock::DockState::new(vec![Tab::Terminal(format!("s{n}"))]),
    };
    let mut workspace =
        Workspace::from_layout(egui_dock::DockState::new(vec![Tab::Terminal("s1".into())]));
    workspace.tabs = (1..=13).map(tab).collect();
    workspace.active = "g13".into();
    for n in 1..=13 {
        app.state
            .sessions
            .push(session_fixture(&format!("s{n}"), SessionKind::Shell));
    }
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());
}

#[cfg(feature = "test-support")]
#[test]
fn the_all_tabs_button_stays_in_view_when_tabs_overflow() {
    let (mut app, ctx, _dir) = fixture();
    many_tabs(&mut app);
    for _ in 0..3 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(500.0, 120.0),
                )),
                ..Default::default()
            },
            |ui| app.window_header_tabs(ui),
        );
        output.textures_delta.clear();
    }
    let strip = rect_of(&app, &ctx, "workspace-strip");
    let all = rect_of(&app, &ctx, "workspace-all-tabs");
    assert!(app.fixture_rect(&ctx, "tabs-right").is_some(), "overflow");
    assert!(
        all.right() <= strip.right() + 0.5,
        "All tabs {all:?} must fit in the strip {strip:?}"
    );
}
