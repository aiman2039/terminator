//! File tabs like the `JetBrains` editor tabs: the file-type icon, the
//! unsaved dot in the close button's place, and the parent folder after a
//! file name that two open tabs share.
use super::*;
use crate::native_editor::NativeDoc;
use egui_dock::TabViewer;

fn native(path: &str) -> Tab {
    Tab::NativeEditor {
        path: PathBuf::from(path),
    }
}

fn leaf(node: usize) -> egui_dock::NodePath {
    egui_dock::NodePath {
        surface: egui_dock::SurfaceIndex::main(),
        node: NodeIndex(node),
    }
}

/// A Neovim editor session of `file`, labeled with its file name.
fn editor_session(sid: &str, file: &str) -> Session {
    let mut session = session_fixture(sid, SessionKind::Editor);
    session.file = Some(PathBuf::from(file));
    session.label = Path::new(file)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    session
}

fn viewer(app: &mut App, window: Option<egui::ViewportId>) -> crate::workspace_ui::Viewer<'_> {
    crate::workspace_ui::Viewer {
        app,
        strip: false,
        project: Some("a".into()),
        tab: None,
        window,
        render_path: None,
        render_tab: None,
    }
}

fn title(viewer: &mut crate::workspace_ui::Viewer<'_>, tab: &Tab) -> String {
    viewer.title(&mut tab.clone()).text().to_owned()
}

#[test]
fn file_tabs_show_their_file_type_icon() {
    let (mut app, _, _dir) = fixture();
    let icon = |app: &App, tab: Tab| app.tab_face(Some(&tab)).icon;
    assert_eq!(icon(&app, native("/a/src/main.rs")), "lang:rust");
    assert_eq!(icon(&app, native("/a/README.md")), "lang:markdown");
    assert_eq!(icon(&app, native("/a/notes.zzz")), "File");
    let image = Tab::Image {
        path: PathBuf::from("/a/logo.png"),
    };
    assert_eq!(icon(&app, image), "lang:image");
    // Neovim editors show their file's icon too; Git reviews and shells
    // keep their own glyphs.
    let mut review = editor_session("review", "/a/app.py");
    review.review = true;
    review.label = "Diff: app.py".into();
    app.state.sessions.extend([
        editor_session("edit", "/a/app.py"),
        review,
        session_fixture("shell", SessionKind::Shell),
    ]);
    assert_eq!(icon(&app, Tab::Terminal("edit".into())), "lang:python");
    assert_eq!(icon(&app, Tab::Terminal("review".into())), "FileCode");
    assert_eq!(icon(&app, Tab::Terminal("shell".into())), "Terminal");
}

#[test]
fn pane_tabs_reserve_the_file_icon_and_flag_unsaved_native_editors() {
    let (mut app, _, _dir) = fixture();
    let path = PathBuf::from("/a/main.rs");
    app.native_docs.insert(
        path.clone(),
        NativeDoc::loaded(path.clone(), "fn main() {}\n"),
    );
    let image = Tab::Image {
        path: PathBuf::from("/a/logo.png"),
    };
    let mut viewer = viewer(&mut app, None);
    let slot = appearance::TERMINAL_LEADING_SLOT;
    assert_eq!(viewer.tab_leading_width(&native("/a/main.rs")), slot);
    assert_eq!(viewer.tab_leading_width(&image), slot);
    assert_eq!(viewer.tab_leading_width(&Tab::Player), 0.0);
    assert!(!viewer.tab_modified(&native("/a/main.rs")), "saved");
    viewer
        .app
        .native_docs
        .insert(path.clone(), NativeDoc::dirty_for_test(path.clone()));
    assert!(viewer.tab_modified(&native("/a/main.rs")), "unsaved");
    assert!(!viewer.tab_modified(&image));
    // The title no longer carries its own unsaved marker: the dot in the
    // close button's place is the one indicator.
    assert_eq!(title(&mut viewer, &native("/a/main.rs")), "main.rs");
}

#[test]
fn pane_tabs_with_the_same_file_name_show_their_parent_folder() {
    let (mut app, _, _dir) = fixture();
    app.state
        .sessions
        .push(editor_session("nvim", "/a/z/mod.rs"));
    app.pane_tabs
        .insert(leaf(1), vec![native("/a/x/mod.rs"), native("/a/x/lib.rs")]);
    app.pane_tabs.insert(leaf(2), vec![native("/a/y/mod.rs")]);
    app.pane_tabs
        .insert(leaf(3), vec![Tab::Terminal("nvim".into())]);
    let mut main = viewer(&mut app, None);
    assert_eq!(title(&mut main, &native("/a/x/mod.rs")), "mod.rs x");
    assert_eq!(title(&mut main, &native("/a/y/mod.rs")), "mod.rs y");
    assert_eq!(title(&mut main, &native("/a/x/lib.rs")), "lib.rs");
    assert_eq!(title(&mut main, &Tab::Terminal("nvim".into())), "mod.rs z");
    // A renamed Neovim tab no longer shows its file name, so no hint.
    if let Some(session) = main.app.state.sessions.last_mut() {
        session.label = "notes".into();
    }
    assert_eq!(title(&mut main, &Tab::Terminal("nvim".into())), "notes");
    // A floating window compares only its own tabs.
    main.app
        .float_pane_tabs
        .insert(leaf(1), vec![native("/a/x/mod.rs")]);
    let float = egui::ViewportId::from_hash_of("file-tabs-float");
    let mut float = viewer(main.app, Some(float));
    assert_eq!(title(&mut float, &native("/a/x/mod.rs")), "mod.rs");
}

#[test]
fn a_file_moved_in_the_explorer_hints_from_its_new_path() {
    let (mut app, _, _dir) = fixture();
    let dock = Workspace::from_layout(DockState::new(vec![
        native("/a/x/mod.rs"),
        native("/a/lib.rs"),
    ]));
    app.layouts.insert("a".into(), dock);
    let refresh = |app: &mut App| {
        let dock = app.layouts.remove("a").expect("dock");
        app.refresh_pane_maps("a", &dock);
        app.layouts.insert("a".into(), dock);
    };
    refresh(&mut app);
    // Moving folder x to y keeps the tab count and the focused pane.
    app.remap_moved_path(Path::new("/a/x"), Path::new("/a/y"));
    refresh(&mut app);
    let tabs: Vec<&Tab> = app.pane_tabs.values().flatten().collect();
    assert!(tabs.contains(&&native("/a/y/mod.rs")), "{tabs:?}");
    let mut main = viewer(&mut app, None);
    assert_eq!(title(&mut main, &native("/a/y/mod.rs")), "mod.rs");
}

#[test]
fn remote_file_hints_show_only_remote_folders() {
    let (app, _, _dir) = fixture();
    // Local copies of SSH files live under the project's cache folder.
    let cache = app.paths.data.join("remote-files").join("project-id");
    let (root, docs) = (cache.join("README.md"), cache.join("docs/README.md"));
    let open = [root.as_path(), docs.as_path()];
    assert_eq!(app.tab_folder_hint(&root, open), None, "the remote root");
    assert_eq!(app.tab_folder_hint(&docs, open).as_deref(), Some("docs"));
}

/// One frame of the workspace strip and the main dock, in panel order.
#[cfg(feature = "test-support")]
fn tabs_frame(
    app: &mut App,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
) -> Vec<egui::epaint::ClippedShape> {
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
            app.workspace_bar(ui, "a", &mut dock);
            app.paint_dock(ui, "a", &mut dock);
            app.layouts.insert("a".into(), dock);
        },
    );
    output.textures_delta.clear();
    output.shapes
}

#[cfg(feature = "test-support")]
fn each_shape(shapes: &[egui::epaint::ClippedShape], mut visit: impl FnMut(&egui::Shape)) {
    fn walk(shape: &egui::Shape, visit: &mut impl FnMut(&egui::Shape)) {
        if let egui::Shape::Vec(shapes) = shape {
            for shape in shapes {
                walk(shape, visit);
            }
        } else {
            visit(shape);
        }
    }
    for clipped in shapes {
        walk(&clipped.shape, &mut visit);
    }
}

/// Filled dots centered inside `rect`.
#[cfg(feature = "test-support")]
fn dots_in(shapes: &[egui::epaint::ClippedShape], rect: egui::Rect) -> usize {
    let mut count = 0_usize;
    each_shape(shapes, |shape| {
        if let egui::Shape::Circle(circle) = shape
            && circle.fill != Color32::TRANSPARENT
            && rect.contains(circle.center)
        {
            count = count.saturating_add(1);
        }
    });
    count
}

/// Line segments that lie entirely inside `rect`: the dock's close X.
#[cfg(feature = "test-support")]
fn segments_in(shapes: &[egui::epaint::ClippedShape], rect: egui::Rect) -> usize {
    let mut count = 0_usize;
    each_shape(shapes, |shape| {
        if let egui::Shape::LineSegment { points, .. } = shape
            && points.iter().all(|point| rect.contains(*point))
        {
            count = count.saturating_add(1);
        }
    });
    count
}

/// The tint of the texture (icon) painted with its center inside `rect`.
#[cfg(feature = "test-support")]
fn icon_tint_in(shapes: &[egui::epaint::ClippedShape], rect: egui::Rect) -> Option<Color32> {
    let mut tint = None;
    each_shape(shapes, |shape| {
        if let egui::Shape::Rect(icon) = shape
            && icon.brush.is_some()
            && rect.contains(icon.rect.center())
        {
            tint = Some(icon.fill);
        }
    });
    tint
}

#[cfg(feature = "test-support")]
fn press(app: &mut App, ctx: &egui::Context, pos: egui::Pos2) {
    tabs_frame(app, ctx, vec![frame_press(pos, true)]);
    tabs_frame(app, ctx, vec![frame_press(pos, false)]);
}

/// A clean native editor of `/a/main.rs` as the selected project's tab.
#[cfg(feature = "test-support")]
fn main_rs_tab(app: &mut App) -> PathBuf {
    let path = PathBuf::from("/a/main.rs");
    app.native_docs.insert(
        path.clone(),
        NativeDoc::loaded(path.clone(), "fn main() {}\n"),
    );
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(DockState::new(vec![native("/a/main.rs")])),
    );
    app.selected = Some("a".into());
    path
}

#[cfg(feature = "test-support")]
#[test]
fn unsaved_pane_tabs_show_a_dot_until_the_close_button_is_hovered() {
    let (mut app, ctx, _dir) = fixture();
    let path = main_rs_tab(&mut app);
    tabs_frame(&mut app, &ctx, vec![]);
    let shapes = tabs_frame(&mut app, &ctx, vec![]);
    let key = format!("leaf-tab:{}", native("/a/main.rs").key());
    let tab = crate::diagnostics::current_rect(&ctx, &key).expect("pane tab");
    let close = egui::Rect::from_center_size(
        egui::pos2(tab.right() - 12.0, tab.center().y),
        egui::Vec2::splat(24.0),
    );
    let icon = format!("file-tab-icon:{}", path.display());
    assert!(crate::diagnostics::current_rect(&ctx, &icon).is_some());
    assert_eq!(dots_in(&shapes, close), 0, "a saved file shows the close X");
    assert_eq!(segments_in(&shapes, close), 2);
    app.native_docs
        .insert(path.clone(), NativeDoc::dirty_for_test(path.clone()));
    let shapes = tabs_frame(&mut app, &ctx, vec![]);
    assert_eq!(dots_in(&shapes, close), 1, "unsaved: a dot holds the slot");
    assert_eq!(segments_in(&shapes, close), 0);
    // Pointing at the dot brings the close button back.
    tabs_frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(close.center())],
    );
    let shapes = tabs_frame(&mut app, &ctx, vec![]);
    assert_eq!(dots_in(&shapes, close), 0, "hovered: the close X returns");
    assert_eq!(segments_in(&shapes, close), 2);
    // A click there still closes through the unsaved-changes prompt.
    press(&mut app, &ctx, close.center());
    assert!(
        matches!(&app.native_close_prompt, Some((prompt, _)) if prompt == &path),
        "closing an unsaved tab asks first"
    );
}

#[cfg(feature = "test-support")]
#[test]
fn unsaved_strip_tabs_show_a_dot_that_turns_into_the_close_button() {
    let (mut app, ctx, _dir) = fixture();
    let path = main_rs_tab(&mut app);
    tabs_frame(&mut app, &ctx, vec![]);
    let shapes = tabs_frame(&mut app, &ctx, vec![]);
    let close = crate::diagnostics::current_rect(&ctx, "workspace-close:main.rs").expect("slot");
    let tab = crate::diagnostics::current_rect(&ctx, "workspace-tab:main.rs").expect("tab");
    let identity = egui::Rect::from_center_size(
        egui::pos2(tab.left() + 16.0, tab.center().y),
        egui::Vec2::splat(16.0),
    );
    assert_eq!(
        icon_tint_in(&shapes, identity),
        Some(Color32::WHITE),
        "the file-type icon keeps its own colors"
    );
    assert_eq!(dots_in(&shapes, close), 0);
    assert!(icon_tint_in(&shapes, close).is_some(), "the active tab's X");
    app.native_docs
        .insert(path.clone(), NativeDoc::dirty_for_test(path.clone()));
    // The unsaved dot also wins over the pin mark.
    app.layouts.get_mut("a").expect("workspace").tabs[0].pinned = true;
    let shapes = tabs_frame(&mut app, &ctx, vec![]);
    assert_eq!(dots_in(&shapes, close), 1, "unsaved: a dot holds the slot");
    assert!(icon_tint_in(&shapes, close).is_none());
    assert!(!painted_strings(&shapes).iter().any(|text| text == "◆"));
    tabs_frame(
        &mut app,
        &ctx,
        vec![egui::Event::PointerMoved(close.center())],
    );
    let shapes = tabs_frame(&mut app, &ctx, vec![]);
    assert_eq!(dots_in(&shapes, close), 0, "hovered: the close X returns");
    assert!(icon_tint_in(&shapes, close).is_some());
    press(&mut app, &ctx, close.center());
    assert_eq!(
        app.close_workspace
            .as_ref()
            .map(|(project, _)| project.as_str()),
        Some("a"),
        "the click still starts the tab close"
    );
}

#[cfg(feature = "test-support")]
#[test]
fn strip_dot_marks_an_unsaved_file_anywhere_in_the_tab() {
    let (mut app, ctx, _dir) = fixture();
    app.state
        .sessions
        .push(session_fixture("shell", SessionKind::Shell));
    let path = PathBuf::from("/a/lib.rs");
    app.native_docs
        .insert(path.clone(), NativeDoc::dirty_for_test(path.clone()));
    let mut workspace = Workspace::from_layout(DockState::new(vec![Tab::Terminal("shell".into())]));
    workspace
        .main_surface_mut()
        .split_right(NodeIndex::root(), 0.5, vec![native("/a/lib.rs")]);
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());
    tabs_frame(&mut app, &ctx, vec![]);
    let shapes = tabs_frame(&mut app, &ctx, vec![]);
    let close = crate::diagnostics::current_rect(&ctx, "workspace-close:shell").expect("slot");
    assert_eq!(
        dots_in(&shapes, close),
        1,
        "the shell tab holds an unsaved split"
    );
}

#[cfg(feature = "test-support")]
#[test]
fn strip_tabs_with_the_same_file_name_show_their_parent_folder() {
    let (mut app, ctx, _dir) = fixture();
    let mut workspace = Workspace::from_layout(DockState::new(vec![native("/a/x/mod.rs")]));
    workspace.add("y".into(), native("/a/y/mod.rs"));
    workspace.add("lib".into(), native("/a/x/lib.rs"));
    app.layouts.insert("a".into(), workspace);
    app.selected = Some("a".into());
    tabs_frame(&mut app, &ctx, vec![]);
    let text = painted_strings(&tabs_frame(&mut app, &ctx, vec![]));
    for label in ["mod.rs x", "mod.rs y", "lib.rs"] {
        assert!(text.iter().any(|line| line == label), "{label}: {text:?}");
    }
    let hinted = crate::diagnostics::current_rect(&ctx, "workspace-tab:mod.rs").expect("tab");
    // With one `mod.rs` left, its tab loses the hint and narrows again.
    app.layouts
        .get_mut("a")
        .expect("workspace")
        .tabs
        .retain(|group| group.id != "y");
    tabs_frame(&mut app, &ctx, vec![]);
    let text = painted_strings(&tabs_frame(&mut app, &ctx, vec![]));
    assert!(text.iter().any(|line| line == "mod.rs"), "{text:?}");
    assert!(!text.iter().any(|line| line == "mod.rs x"), "{text:?}");
    let plain = crate::diagnostics::current_rect(&ctx, "workspace-tab:mod.rs").expect("tab");
    assert!(hinted.width() > plain.width(), "{hinted:?} vs {plain:?}");
}
