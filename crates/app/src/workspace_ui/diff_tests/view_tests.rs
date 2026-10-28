use super::*;

#[test]
fn diff_neighbors_walks_the_sibling_list() {
    let paths: Vec<std::path::PathBuf> = ["/repo/a.rs", "/repo/b.rs", "/repo/c.rs"]
        .iter()
        .map(std::path::PathBuf::from)
        .collect();
    let (prev, next, pos, total) =
        super::super::diff_view::diff_neighbors(&paths, std::path::Path::new("/repo/b.rs"));
    assert_eq!(prev.as_deref(), Some(std::path::Path::new("/repo/a.rs")));
    assert_eq!(next.as_deref(), Some(std::path::Path::new("/repo/c.rs")));
    assert_eq!((pos, total), (2, 3));
    let (prev, next, pos, _) =
        super::super::diff_view::diff_neighbors(&paths, std::path::Path::new("/repo/a.rs"));
    assert!(prev.is_none());
    assert!(next.is_some());
    assert_eq!(pos, 1);
    let (prev, next, pos, total) =
        super::super::diff_view::diff_neighbors(&paths, std::path::Path::new("/repo/z.rs"));
    assert!(prev.is_none() && next.is_none());
    assert_eq!((pos, total), (0, 3));
}

#[test]
fn staged_diff_switches_to_working_tree_after_dock_returns() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    let (jobs, requests) = std::sync::mpsc::channel();
    app.jobs = jobs.into();
    app.selected = Some("project".into());
    let staged = Tab::Diff {
        remote: None,
        cwd: "/repo".into(),
        path: "/repo/file.rs".into(),
        staged: true,
    };
    let working = Tab::Diff {
        remote: None,
        cwd: "/repo".into(),
        path: "/repo/file.rs".into(),
        staged: false,
    };
    app.layouts.insert(
        "project".into(),
        Workspace::from_layout(egui_dock::DockState::new(vec![staged.clone()])),
    );

    // The active dock is absent while its viewer runs. The request must
    // survive until the dock is returned at the end of the frame.
    let dock = app.layouts.remove("project").unwrap();
    app.open_working_diff(&staged);
    app.apply_diff_replacement();
    assert!(app.diff_replacement.is_some());
    assert!(requests.try_recv().is_err());

    app.layouts.insert("project".into(), dock);
    app.apply_diff_replacement();
    assert!(app.diff_replacement.is_none());
    let workspace = &app.layouts["project"];
    assert_eq!(workspace.tabs[0].primary, Some(working.clone()));
    assert!(workspace.iter_all_tabs().any(|(_, tab)| tab == &working));
    assert!(app.loading.contains(&working.key()));
    assert_eq!(app.focus_tab, Some(working.clone()));
    let Job::Diff(requested) = requests.try_recv().unwrap() else {
        panic!("working-tree diff job was not queued");
    };
    assert_eq!(requested, working);
}

#[test]
fn ide_mode_icon_differs_from_sidebar_toggles() {
    let ide = header_action_view(HeaderAction::IdeMode).icon;
    assert_ne!(ide, "PanelRight");
    assert_ne!(ide, "PanelLeft");
}

#[test]
fn agents_header_uses_the_bell() {
    let agents = header_action_view(HeaderAction::Tool(SidebarTool::Agents)).icon;
    assert_eq!(agents, "Bell");
}

#[test]
fn terminal_theme_cache_tracks_preview_and_revert() {
    let mut cache = None;
    let original = AppearanceConfig::default();
    let mut preview = original.clone();
    preview.terminal_foreground = "#123456".into();
    preview.terminal_background = "#abcdef".into();
    cached_terminal_theme(&mut cache, &original);
    cached_terminal_theme(&mut cache, &preview);
    let cached = cache.as_ref().unwrap();
    assert_eq!(cached.0, preview.terminal_background);
    assert_eq!(cached.1, preview.terminal_foreground);
    cached_terminal_theme(&mut cache, &original);
    let cached = cache.as_ref().unwrap();
    assert_eq!(cached.0, original.terminal_background);
    assert_eq!(cached.1, original.terminal_foreground);
}

#[test]
fn clicking_diff_text_keeps_accessibility_focus_on_an_existing_node() {
    for split in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        let tab = Tab::Diff {
            remote: None,
            cwd: dir.path().into(),
            path: dir.path().join("file.txt"),
            staged: false,
        };
        let doc = diff::build_with_context(diff::BuildWithContextInput {
            path: std::path::Path::new("file.txt"),
            left: "old text\n",
            right: "new text\n",
            staged: false,
            context: 3,
        });
        app.diffs.insert(tab.key(), Ok(doc.into()));
        if split {
            app.diff_split.insert(tab.key());
        }
        let painted = paint_diff_view(&mut app, &ctx, &tab);
        let pos = require_text(&painted, "new text").0 + egui::vec2(12.0, 5.0);
        let mut nodes = std::collections::HashSet::new();
        for frame in 0..4 {
            let mut events = vec![egui::Event::PointerMoved(pos)];
            if frame == 1 || frame == 2 {
                events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: frame == 1,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events,
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ui| app.diff_view(ui, &tab),
            );
            output.textures_delta.clear();
            let update = output.platform_output.accesskit_update.unwrap();
            nodes.extend(update.nodes.iter().map(|(id, _)| *id));
            assert!(
                nodes.contains(&update.focus),
                "focused accessibility node is absent; split={split}, frame={frame}"
            );
        }
    }
}

#[test]
fn unsaved_diff_edits_block_closing_and_quit() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    let mut edit = diff::review::Edit::new(std::sync::Arc::new(sample_doc()));
    edit.text = "unsaved".into();
    app.diff_reviews.entry("diff".into()).or_default().buffer = Some(edit);
    assert!(app.block_unsaved_diff(Some("diff")));
    app.begin_exit();
    assert!(!app.exit.active());
    app.diff_reviews.clear();
    assert!(!app.block_unsaved_diff(None));
}

#[cfg(feature = "test-support")]
#[test]
fn split_divider_drag_rebalances_panes() {
    let ctx = egui::Context::default();
    let doc = long_doc();
    let colors = DiffColors {
        added: Color32::GREEN,
        deleted: Color32::RED,
        accent: Color32::BLUE,
        text: Color32::WHITE,
    };
    let mut ratio = 0.5;
    let frame = |events: Vec<egui::Event>, ratio: &mut f32| {
        let mut sync = 0.0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 300.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                paint_diff_document(PaintDiffDocumentInput {
                    ui,
                    doc: &doc,
                    split: true,
                    colors,
                    scroll_key: "drag-test",
                    ratio,
                    sync: &mut sync,
                });
            },
        );
        output.textures_delta.clear();
    };
    frame(Vec::new(), &mut ratio);
    let handle = ctx
        .data(|d| {
            d.get_temp::<egui::Rect>(egui::Id::unique(("fixture-target", "diff-split-handle")))
        })
        .expect("split handle must be recorded");
    let start = handle.center();
    let dest = start + egui::vec2(150.0, 0.0);
    let press = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    };
    frame(vec![egui::Event::PointerMoved(start)], &mut ratio);
    frame(vec![press(start, true)], &mut ratio);
    frame(vec![egui::Event::PointerMoved(dest)], &mut ratio);
    frame(vec![press(dest, false)], &mut ratio);
    assert!(
        ratio > 0.5,
        "divider drag should widen the left pane: {ratio}"
    );
}

#[test]
fn markdown_diff_preview_renders_both_sides() {
    let doc = diff::DiffDocument {
        left_label: "Left".into(),
        right_label: "Right".into(),
        left_text: "# Hello\n\nThis is the old **markdown**.".into(),
        right_text: "# Hello\n\nThis is the *updated* markdown.".into(),
        unified: vec![],
        split: vec![],
        ..Default::default()
    };
    let ctx = egui::Context::default();
    appearance::install(&ctx);
    let mut previews = markdown::Previews::new(&ctx);
    for (index, text) in [&doc.left_text, &doc.right_text].into_iter().enumerate() {
        previews.snapshot(
            &format!("diff-preview:test-preview:{index}"),
            std::path::Path::new("/repo/test.md"),
            text,
        );
    }
    previews.wait_prepared();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 400.0),
            )),
            ..Default::default()
        },
        |ui| {
            paint_markdown_diff_preview(PaintMarkdownDiffPreviewInput {
                ui,
                doc: &doc,
                previews: &mut previews,
                path: std::path::Path::new("/repo/test.md"),
                scroll_key: "test-preview",
            });
        },
    );
    output.textures_delta.clear();
    let painted = painted_text(&output.shapes);
    assert!(painted.iter().any(|(_, t)| t.contains("Hello")));
    assert!(painted.iter().any(|(_, t)| t.contains("Left")));
    assert!(painted.iter().any(|(_, t)| t.contains("Right")));
    assert!(painted.iter().any(|(_, t)| t.contains("updated")));
}

#[test]
fn perforce_working_diff_is_read_only() {
    let mut doc = diff::build_with_context(diff::BuildWithContextInput {
        path: std::path::Path::new("file.rs"),
        left: "old\n",
        right: "new\n",
        staged: false,
        context: 3,
    });
    doc.read_only = true;
    doc.left_label = "Have revision".into();
    doc.right_label = "Workspace".into();
    let painted = paint_doc(doc, false);
    assert!(painted.iter().any(|(_, text)| text == "Read-only"));
    assert!(!painted.iter().any(|(_, text)| text == "Editable"));
}
