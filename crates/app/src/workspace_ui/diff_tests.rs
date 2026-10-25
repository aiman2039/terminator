use super::super::*;
use super::diff_paint::{
    DiffColors, diff_gutter_digits, paint_diff_document, paint_markdown_diff_preview,
};
use super::header::{HeaderAction, header_action_view};
use super::tabs::cached_terminal_theme;
#[cfg(test)]
pub(crate) mod tests {

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
            cwd: "/repo".into(),
            path: "/repo/file.rs".into(),
            staged: true,
        };
        let working = Tab::Diff {
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
    fn highlighted_editor_job_keeps_buffer_text_and_tints() {
        let doc = diff::build_with_context(
            std::path::Path::new("file.rs"),
            "let a = 1;\n",
            "let a = 2;\n",
            false,
            3,
        );
        let colors = DiffColors {
            added: Color32::GREEN,
            deleted: Color32::RED,
            accent: Color32::BLUE,
            text: Color32::WHITE,
        };
        let right_tints =
            super::super::diff_edit::live_tints(&doc.left_text, &doc.right_text, colors);
        let spans = diff::highlight_spans(std::path::Path::new("file.rs"), &doc.right_text);
        let valid: Vec<bool> = spans.iter().map(|line| !line.is_empty()).collect();
        let job = super::super::diff_edit::highlighted_job(
            &doc.right_text,
            &spans,
            &valid,
            &right_tints,
            egui::FontId::monospace(12.0),
            Color32::WHITE,
            f32::INFINITY,
        );
        // A TextEdit layouter must reproduce the buffer text exactly.
        assert_eq!(job.text, doc.right_text);
        // The changed line carries a diff tint background.
        assert!(right_tints.iter().any(|tint| tint.is_some()));
    }

    #[test]
    fn live_tints_track_edits_as_they_happen() {
        let colors = DiffColors {
            added: Color32::GREEN,
            deleted: Color32::RED,
            accent: Color32::BLUE,
            text: Color32::WHITE,
        };
        let tints = super::super::diff_edit::live_tints("a\nb\nc\n", "a\nB\nc\nd\n", colors);
        assert_eq!(tints[0], None);
        assert!(tints[1].is_some(), "replaced line is tinted");
        assert_eq!(tints[2], None);
        assert!(tints[3].is_some(), "inserted line is tinted");
        // Fixing the change removes its tint immediately.
        let clean = super::super::diff_edit::live_tints("a\nb\n", "a\nb\n", colors);
        assert!(clean.iter().all(|tint| tint.is_none()));
        // An insertion shifts later tints with the text.
        let shifted = super::super::diff_edit::live_tints("a\nb\n", "x\na\nb\n", colors);
        assert!(shifted[0].is_some());
        assert_eq!(shifted[1], None);
    }

    #[cfg(feature = "test-support")]
    fn fixture_target(ctx: &egui::Context, name: &str) -> Option<egui::Rect> {
        ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("fixture-target", name))))
    }

    #[cfg(feature = "test-support")]
    fn run_diff_frames(
        app: &mut App,
        ctx: &egui::Context,
        tab: &Tab,
        frames: &[(f64, Vec<egui::Event>)],
    ) {
        for (time, events) in frames {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(900.0, 600.0),
                    )),
                    time: Some(*time),
                    events: events.clone(),
                    ..Default::default()
                },
                |ui| {
                    egui::ScrollArea::new([false, false]).show(ui, |ui| {
                        let available = ui.available_rect_before_wrap();
                        ui.expand_to_include_rect(available);
                        app.diff_view(ui, tab);
                    });
                },
            );
            output.textures_delta.clear();
        }
    }

    #[cfg(feature = "test-support")]
    fn click_and_type(app: &mut App, ctx: &egui::Context, tab: &Tab, target: &str, text: &str) {
        // Times keep moving forward across calls within one test.
        let base = ctx.input(|input| input.time);
        run_diff_frames(app, ctx, tab, &[(base + 0.1, vec![]), (base + 0.2, vec![])]);
        let rect = fixture_target(ctx, target)
            .unwrap_or_else(|| panic!("editor target {target} was not painted"));
        let pos = rect.center();
        let click = |pressed: bool| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        run_diff_frames(
            app,
            ctx,
            tab,
            &[
                (base + 0.3, click(true)),
                (base + 0.4, click(false)),
                (base + 0.45, vec![]),
                (base + 0.5, vec![egui::Event::Text(text.to_owned())]),
                (base + 0.6, vec![]),
                (base + 0.7, vec![]),
            ],
        );
    }

    #[cfg(feature = "test-support")]
    fn buffer_text(app: &App, tab: &Tab) -> String {
        app.diff_reviews
            .get(&tab.key())
            .and_then(|review| review.buffer.as_ref())
            .map(|buffer| buffer.text.clone())
            .unwrap_or_default()
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn typing_in_unified_diff_edits_the_buffer_and_tints_live() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        crate::appearance::install(&ctx);
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        let tab = Tab::Diff {
            cwd: "/repo".into(),
            path: "/repo/file.rs".into(),
            staged: false,
        };
        let doc = diff::build_with_context(
            std::path::Path::new("file.rs"),
            "let a = 1;\nlet b = 2;\n",
            "let a = 1;\nlet b = 3;\n",
            false,
            3,
        );
        app.diffs.insert(tab.key(), Ok(doc.into()));
        // Edit the already-changed line 2.
        click_and_type(&mut app, &ctx, &tab, "diff-unified-edit-2", "X");
        let text = buffer_text(&app, &tab);
        assert!(
            text.contains('X'),
            "unified typing must reach the buffer, got {text:?}"
        );
        // Changing the untouched line 1 must tint it in the same session.
        click_and_type(&mut app, &ctx, &tab, "diff-unified-edit-1", "Y");
        assert!(
            buffer_text(&app, &tab).contains('Y'),
            "line 1 typing must reach the buffer, got {:?}",
            buffer_text(&app, &tab)
        );
        let review = app.diff_reviews.get(&tab.key()).expect("review state");
        let buffer = review.buffer.as_ref().expect("buffer");
        let (_, tints) = buffer.live_tints.as_ref().expect("live tints cached");
        assert!(
            tints.first().is_some_and(|tint| tint.is_some()),
            "line 1 must tint live after editing it, tints: {tints:?}"
        );
        assert!(buffer_text(&app, &tab).contains('Y'));
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn typing_in_split_diff_edits_the_buffer_and_tints_live() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        crate::appearance::install(&ctx);
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        let tab = Tab::Diff {
            cwd: "/repo".into(),
            path: "/repo/file.rs".into(),
            staged: false,
        };
        app.diff_split.insert(tab.key());
        let doc = diff::build_with_context(
            std::path::Path::new("file.rs"),
            "let a = 1;\nlet b = 2;\n",
            "let a = 1;\nlet b = 3;\n",
            false,
            3,
        );
        app.diffs.insert(tab.key(), Ok(doc.into()));
        click_and_type(&mut app, &ctx, &tab, "diff-right-editor", "Z");
        let text = buffer_text(&app, &tab);
        assert!(
            text.contains('Z'),
            "split typing must reach the buffer, got {text:?}"
        );
        let review = app.diff_reviews.get(&tab.key()).expect("review state");
        let buffer = review.buffer.as_ref().expect("buffer");
        // Tint and syntax caches must describe the text just typed, not the
        // document the tab was opened with.
        let tint_key = buffer.live_tints.as_ref().expect("live tints cached").0;
        let span_key = buffer.right_spans.as_ref().expect("spans cached").0;
        assert_eq!(tint_key, span_key, "caches must track the edited revision");
        assert!(
            buffer
                .live_tints
                .as_ref()
                .is_some_and(|(_, tints)| tints.iter().any(|tint| tint.is_some())),
            "edited lines must carry a live tint"
        );
    }

    #[test]
    fn live_rows_cover_the_whole_file_and_track_blocks() {
        use diff::review::{LiveRow, LiveTint};
        let rows = super::super::diff_edit::live_rows("a\nb\nc\nd\n", "a\nB\nc\nd\ne\n");
        // Delete of b (Gone) + B (Changed) + appended e (Added), rest equal.
        assert!(matches!(rows[0], LiveRow::Edit { changed: None, .. }));
        assert!(matches!(rows[1], LiveRow::Gone { old: 1, .. }));
        assert!(matches!(
            rows[2],
            LiveRow::Edit {
                changed: Some(LiveTint::Changed),
                ..
            }
        ));
        assert!(matches!(
            rows[5],
            LiveRow::Edit {
                changed: Some(LiveTint::Added),
                ..
            }
        ));
        let edits = rows
            .iter()
            .filter(|row| matches!(row, LiveRow::Edit { .. }))
            .count();
        assert_eq!(edits, 5, "every buffer line is an editable row");
        assert_eq!(
            super::super::diff_edit::replace_line("", 0, "x"),
            "x",
            "an empty file gains its first line"
        );
    }

    #[test]
    fn row_ops_split_and_join_lines() {
        use super::super::diff_edit::{RowOp, apply_row_op};
        let split = apply_row_op(
            "alpha\nbeta\n",
            &RowOp::Split {
                line: 0,
                head: "al".into(),
                tail: "pha".into(),
            },
        );
        assert_eq!(split, "al\npha\nbeta\n");
        let joined = apply_row_op(&split, &RowOp::JoinPrev { line: 1 });
        assert_eq!(joined, "alpha\nbeta\n");
        let joined_next = apply_row_op("alpha\nbeta\n", &RowOp::JoinNext { line: 0 });
        assert_eq!(joined_next, "alphabeta\n");
        // Out-of-range ops are no-ops.
        assert_eq!(apply_row_op("a\n", &RowOp::JoinPrev { line: 0 }), "a\n");
    }

    #[cfg(feature = "test-support")]
    fn click_and_key(app: &mut App, ctx: &egui::Context, tab: &Tab, target: &str, key: egui::Key) {
        let base = ctx.input(|input| input.time);
        run_diff_frames(app, ctx, tab, &[(base + 0.1, vec![]), (base + 0.2, vec![])]);
        let rect = fixture_target(ctx, target)
            .unwrap_or_else(|| panic!("editor target {target} was not painted"));
        let pos = rect.center();
        let click = |pressed: bool| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        run_diff_frames(
            app,
            ctx,
            tab,
            &[
                (base + 0.3, click(true)),
                (base + 0.4, click(false)),
                (base + 0.45, vec![]),
                (
                    base + 0.5,
                    vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                ),
                (base + 0.6, vec![]),
                (base + 0.7, vec![]),
            ],
        );
    }

    #[cfg(feature = "test-support")]
    fn click_and_shortcut(
        app: &mut App,
        ctx: &egui::Context,
        tab: &Tab,
        target: &str,
        shift: bool,
    ) {
        let base = ctx.input(|input| input.time);
        run_diff_frames(app, ctx, tab, &[(base + 0.1, vec![]), (base + 0.2, vec![])]);
        let rect = fixture_target(ctx, target)
            .unwrap_or_else(|| panic!("editor target {target} was not painted"));
        let pos = rect.center();
        let click = |pressed: bool| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        let mut modifiers = egui::Modifiers::COMMAND;
        modifiers.shift = shift;
        run_diff_frames(
            app,
            ctx,
            tab,
            &[
                (base + 0.3, click(true)),
                (base + 0.4, click(false)),
                (base + 0.45, vec![]),
                (
                    base + 0.5,
                    vec![egui::Event::Key {
                        key: egui::Key::Z,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    }],
                ),
                (base + 0.6, vec![]),
                (base + 0.7, vec![]),
                (base + 0.8, vec![]),
            ],
        );
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn undo_redo_in_unified_restores_buffer_text() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        crate::appearance::install(&ctx);
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        let tab = Tab::Diff {
            cwd: "/repo".into(),
            path: "/repo/file.rs".into(),
            staged: false,
        };
        let original = "let a = 1;\nlet b = 3;\n";
        let doc = diff::build_with_context(
            std::path::Path::new("file.rs"),
            "let a = 1;\nlet b = 2;\n",
            original,
            false,
            3,
        );
        app.diffs.insert(tab.key(), Ok(doc.into()));
        click_and_type(&mut app, &ctx, &tab, "diff-unified-edit-1", "X");
        let edited = buffer_text(&app, &tab);
        assert_ne!(edited, original, "typing must change the buffer first");
        click_and_shortcut(&mut app, &ctx, &tab, "diff-unified-edit-1", false);
        assert_eq!(
            buffer_text(&app, &tab),
            original,
            "Cmd+Z must restore the pre-edit buffer"
        );
        click_and_shortcut(&mut app, &ctx, &tab, "diff-unified-edit-1", true);
        assert_eq!(
            buffer_text(&app, &tab),
            edited,
            "Cmd+Shift+Z must redo the edit"
        );
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn enter_in_unified_inserts_a_new_line() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        crate::appearance::install(&ctx);
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        let tab = Tab::Diff {
            cwd: "/repo".into(),
            path: "/repo/file.rs".into(),
            staged: false,
        };
        let doc = diff::build_with_context(
            std::path::Path::new("file.rs"),
            "let a = 1;\nlet b = 2;\n",
            "let a = 1;\nlet b = 3;\n",
            false,
            3,
        );
        app.diffs.insert(tab.key(), Ok(doc.into()));
        click_and_key(
            &mut app,
            &ctx,
            &tab,
            "diff-unified-edit-2",
            egui::Key::Enter,
        );
        let text = buffer_text(&app, &tab);
        assert!(
            text.contains(";\n") && text.lines().count() >= 3,
            "Enter must split the unified row into a new line, got {text:?}"
        );
        // Focus must follow the new line so typing continues there.
        let focused = ctx.memory(|memory| memory.focused());
        assert_eq!(
            focused,
            Some(egui::Id::unique((tab.key(), "unified-line", 3_u32))),
            "focus must move to the split line"
        );
    }

    #[cfg(feature = "test-support")]
    #[test]
    fn typing_in_unified_gap_line_edits_and_highlights_its_block() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        crate::appearance::install(&ctx);
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        let tab = Tab::Diff {
            cwd: "/repo".into(),
            path: "/repo/file.rs".into(),
            staged: false,
        };
        let left: String = (1..=12).fold(String::new(), |mut acc, n| {
            use std::fmt::Write as _;
            let _ = writeln!(acc, "line {n}");
            acc
        });
        let mut right = left.clone();
        right = right.replace("line 12", "line twelve");
        let doc =
            diff::build_with_context(std::path::Path::new("file.rs"), &left, &right, false, 3);
        app.diffs.insert(tab.key(), Ok(doc.into()));
        // Line 1 sits in a collapsed gap in the old grouped view; it must be
        // editable here exactly like in split view.
        click_and_type(&mut app, &ctx, &tab, "diff-unified-edit-1", "Q");
        assert!(
            buffer_text(&app, &tab).lines().next() == Some("line 1Q"),
            "gap line must accept edits, got {:?}",
            buffer_text(&app, &tab)
        );
        let review = app.diff_reviews.get(&tab.key()).expect("review state");
        let buffer = review.buffer.as_ref().expect("buffer");
        let (_, rows) = buffer.live_rows.as_ref().expect("live rows cached");
        assert!(
            rows.iter().any(|row| matches!(
                row,
                diff::review::LiveRow::Edit {
                    new: 0,
                    changed: Some(_),
                    ..
                }
            )),
            "the edited gap line must join a highlighted block live: {rows:?}"
        );
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

    use super::*;

    fn span(text: &str) -> diff::DiffSpan {
        diff::DiffSpan {
            text: text.into(),
            rgb: [209, 211, 217],
            intra: diff::Intra::None,
        }
    }

    fn line(
        kind: diff::LineKind,
        old_no: Option<u32>,
        new_no: Option<u32>,
        text: &str,
    ) -> diff::DiffLine {
        diff::DiffLine {
            kind,
            old_no,
            new_no,
            spans: vec![span(text)],
        }
    }

    fn sample_line() -> diff::DiffLine {
        line(diff::LineKind::Insert, None, Some(1), "visible-diff-marker")
    }

    fn sample_doc() -> diff::DiffDocument {
        diff::DiffDocument {
            left_label: "Index".into(),
            right_label: "Working tree".into(),
            left_text: String::new(),
            right_text: "visible-diff-marker\n".into(),
            unified: vec![sample_line()],
            split: vec![],
            ..Default::default()
        }
    }

    #[test]
    fn clicking_diff_text_keeps_accessibility_focus_on_an_existing_node() {
        for split in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
            let tab = Tab::Diff {
                cwd: dir.path().into(),
                path: dir.path().join("file.txt"),
                staged: false,
            };
            let doc = diff::build_with_context(
                std::path::Path::new("file.txt"),
                "old text\n",
                "new text\n",
                false,
                3,
            );
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
        app.diff_reviews.entry("diff".into()).or_default().buffer = Some(diff::review::Edit {
            document: std::sync::Arc::new(sample_doc()),
            text: "unsaved".into(),
            right_spans: None,
            live_tints: None,
            live_rows: None,
            undo: Vec::new(),
            redo: Vec::new(),
            burst: 0.0,
            left_spans: None,
        });
        assert!(app.block_unsaved_diff(Some("diff")));
        app.begin_exit();
        assert!(!app.exit.active());
        app.diff_reviews.clear();
        assert!(!app.block_unsaved_diff(None));
    }

    fn paint_diff_view(app: &mut App, ctx: &egui::Context, tab: &Tab) -> Vec<(egui::Pos2, String)> {
        let mut painted = Vec::new();
        for _ in 0..2 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    // egui_dock tab body: no pane scrollbars, expand to the leaf,
                    // then the native viewer owns its own ScrollArea.
                    egui::ScrollArea::new([false, false]).show(ui, |ui| {
                        let available = ui.available_rect_before_wrap();
                        ui.expand_to_include_rect(available);
                        app.diff_view(ui, tab);
                    });
                },
            );
            painted = painted_text(&output.shapes);
            output.textures_delta.clear();
        }
        painted
    }

    fn paint_doc(doc: diff::DiffDocument, split: bool) -> Vec<(egui::Pos2, String)> {
        paint_doc_staged(doc, split, false)
    }

    fn paint_doc_staged(
        doc: diff::DiffDocument,
        split: bool,
        staged: bool,
    ) -> Vec<(egui::Pos2, String)> {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        let tab = Tab::Diff {
            cwd: "/repo".into(),
            path: "/repo/file.rs".into(),
            staged,
        };
        if split {
            app.diff_split.insert(tab.key());
        }
        app.diffs.insert(tab.key(), Ok(doc.into()));
        paint_diff_view(&mut app, &ctx, &tab)
    }

    pub(crate) fn painted_text(shapes: &[egui::epaint::ClippedShape]) -> Vec<(egui::Pos2, String)> {
        fn walk(out: &mut Vec<(egui::Pos2, String)>, shape: &egui::Shape) {
            match shape {
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        walk(out, shape);
                    }
                }
                egui::Shape::Text(text) => out.push((text.pos, text.galley.text().to_owned())),
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in shapes {
            walk(&mut out, &clipped.shape);
        }
        out
    }

    fn require_text<'a>(
        painted: &'a [(egui::Pos2, String)],
        needle: &str,
    ) -> &'a (egui::Pos2, String) {
        painted
            .iter()
            .find(|(_, text)| text == needle || text.lines().any(|line| line.trim_end() == needle))
            .unwrap_or_else(|| panic!("{needle} was not painted: {painted:?}"))
    }

    #[test]
    fn reopened_diff_uses_unified_after_default_changes() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        app.selected = Some("project".into());
        let tab = Tab::Diff {
            cwd: dir.path().into(),
            path: dir.path().join("file.md"),
            staged: false,
        };
        let Tab::Diff { cwd, path, staged } = &tab else {
            unreachable!()
        };
        app.context = Some(services::ContextData {
            cwd: cwd.clone(),
            root: Some(cwd.clone()),
            perforce: None,
            git_dirs: vec![],
            branch: "main".into(),
            changes: vec![],
            decorations: std::collections::HashMap::default(),
            stats: std::collections::HashMap::default(),
            error: None,
        });
        let open_native = |app: &mut App| {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                app.file_action(
                    ui,
                    if *staged {
                        FileAction::NativeStagedDiff
                    } else {
                        FileAction::NativeWorkingDiff
                    },
                    path,
                    None,
                );
            });
            output.textures_delta.clear();
        };
        app.state.settings.diff_split_default = true;
        open_native(&mut app);
        assert!(app.diff_split.contains(&tab.key()));
        app.layouts.clear();
        app.state.settings.diff_split_default = false;
        open_native(&mut app);
        assert!(!app.diff_split.contains(&tab.key()));
    }

    #[test]
    fn unified_diff_text_stays_in_the_viewport() {
        let painted = paint_doc(sample_doc(), false);
        let (pos, text) = require_text(&painted, "visible-diff-marker");
        assert!(
            (40.0..200.0).contains(&pos.x),
            "code must start at the gutter, not centered or at x=0, got {pos:?} {text}"
        );
        assert!(
            !text.chars().any(|c| c.is_ascii_digit()),
            "code galley must not include line numbers, got {text:?}"
        );
    }

    #[test]
    fn diff_view_paints_hunks_in_a_dock_pane() {
        let painted = paint_doc(sample_doc(), false);
        let (pos, _) = require_text(&painted, "visible-diff-marker");
        assert!(
            (40.0..200.0).contains(&pos.x) && (0.0..500.0).contains(&pos.y),
            "hunk text must stay in the pane, got {pos:?}"
        );
    }

    #[test]
    fn equal_lines_share_a_gutter_x() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "Index".into(),
                right_label: "Working tree".into(),
                left_text: "f\nf\nf\nf\nf\nf\nf\nx\nthis-is-a-much-longer-equal-line\n".into(),
                right_text: "f\nf\nf\nf\nf\nf\nf\nx\nthis-is-a-much-longer-equal-line\n".into(),
                unified: vec![
                    line(diff::LineKind::Equal, Some(8), Some(8), "x"),
                    line(
                        diff::LineKind::Equal,
                        Some(9),
                        Some(9),
                        "this-is-a-much-longer-equal-line",
                    ),
                ],
                split: vec![],
                ..Default::default()
            },
            false,
        );
        let short = require_text(&painted, "x");
        let long = require_text(&painted, "this-is-a-much-longer-equal-line");
        assert!(
            (short.0.x - long.0.x).abs() < 1.0,
            "short and long lines must share a gutter, got {} vs {}",
            short.0.x,
            long.0.x
        );
    }

    #[test]
    fn delete_sign_stays_right_of_line_numbers() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "Index".into(),
                right_label: "Working tree".into(),
                // 100 lines force a 3-digit gutter; the deleted row is visible.
                left_text: format!("f\nf\nremoved-line\n{}", "f\n".repeat(97)),
                right_text: "f\n".repeat(99),
                unified: vec![line(
                    diff::LineKind::Delete,
                    Some(100),
                    None,
                    "removed-line",
                )],
                split: vec![],
                ..Default::default()
            },
            false,
        );
        let number = require_text(&painted, "3");
        let sign = require_text(&painted, "-");
        let code = require_text(&painted, "removed-line");
        assert!(
            sign.0.x > number.0.x + 8.0,
            "minus must sit in its own column, got number={} sign={}",
            number.0.x,
            sign.0.x
        );
        assert!(
            code.0.x > sign.0.x,
            "code must start after the sign, got sign={} code={}",
            sign.0.x,
            code.0.x
        );
    }

    #[test]
    fn insert_sign_stays_right_of_line_numbers() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "Index".into(),
                right_label: "Working tree".into(),
                // 102 lines force a 3-digit gutter; the inserted row is visible.
                left_text: "f\n".repeat(101),
                right_text: format!("f\nf\nadded-line\n{}", "f\n".repeat(99)),
                unified: vec![line(diff::LineKind::Insert, None, Some(102), "added-line")],
                split: vec![],
                ..Default::default()
            },
            false,
        );
        let number = require_text(&painted, "3");
        let sign = require_text(&painted, "+");
        assert!(
            sign.0.x > number.0.x,
            "plus must sit to the right of the new number, got number={} sign={}",
            number.0.x,
            sign.0.x
        );
    }

    #[test]
    fn split_paints_one_number_per_side() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "Index".into(),
                right_label: "Working tree".into(),
                left_text: "\n\n\n\nleft-only\n".into(),
                right_text: "\n\n\n\n\nright-only\n".into(),
                unified: vec![],
                split: vec![diff::SplitRow {
                    left: Some(line(diff::LineKind::Delete, Some(5), None, "left-only")),
                    right: Some(line(diff::LineKind::Insert, None, Some(6), "right-only")),
                }],
                ..Default::default()
            },
            true,
        );
        let left = require_text(&painted, "left-only");
        let right = require_text(&painted, "right-only");
        let old_no = require_text(&painted, "5");
        let new_no = require_text(&painted, "6");
        assert!(
            left.0.x < right.0.x,
            "split sides must not stack, got left={} right={}",
            left.0.x,
            right.0.x
        );
        assert!(
            old_no.0.x < left.0.x && old_no.0.x < 200.0,
            "old number must stay on the left gutter, got {old_no:?}"
        );
        assert!(
            new_no.0.x > left.0.x && new_no.0.x < right.0.x,
            "new number must stay on the right gutter, got old={} new={} left={} right={}",
            old_no.0.x,
            new_no.0.x,
            left.0.x,
            right.0.x
        );
    }

    #[test]
    fn diff_rows_use_font_height_not_item_spacing() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "Index".into(),
                right_label: "Working tree".into(),
                left_text: "row-a\nrow-b\n".into(),
                right_text: "row-a\nrow-b\n".into(),
                unified: vec![
                    line(diff::LineKind::Equal, Some(1), Some(1), "row-a"),
                    line(diff::LineKind::Equal, Some(2), Some(2), "row-b"),
                ],
                split: vec![],
                ..Default::default()
            },
            false,
        );
        let a = require_text(&painted, "row-a");
        let b = require_text(&painted, "row-b");
        let pitch = b.0.y - a.0.y;
        assert!(
            (12.0..22.0).contains(&pitch),
            "row pitch must be the font line height, not height+8 item_spacing, got {pitch}"
        );
    }

    #[test]
    fn equal_line_keeps_old_and_new_numbers_apart() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "Index".into(),
                right_label: "Working tree".into(),
                left_text: format!("f\nf\nunchanged\n{}", "f\n".repeat(94)),
                right_text: format!("f\nf\nunchanged\n{}", "f\n".repeat(94)),
                unified: vec![line(diff::LineKind::Equal, Some(97), Some(97), "unchanged")],
                split: vec![],
                ..Default::default()
            },
            false,
        );
        let numbers: Vec<_> = painted.iter().filter(|(_, text)| text == "3").collect();
        assert_eq!(
            numbers.len(),
            2,
            "unified equal lines paint old and new numbers separately, got {painted:?}"
        );
        let gap = (numbers[1].0.x - numbers[0].0.x).abs();
        assert!(
            gap > 8.0,
            "old and new numbers must be distinct columns, got {} and {}",
            numbers[0].0.x,
            numbers[1].0.x
        );
    }

    #[test]
    fn hunk_header_paints_no_line_numbers() {
        // Grouped (staged, read-only) unified view still paints hunk headers.
        let painted = paint_doc_staged(
            diff::DiffDocument {
                left_label: "Index".into(),
                right_label: "Working tree".into(),
                left_text: String::new(),
                right_text: String::new(),
                unified: vec![line(diff::LineKind::Hunk, None, None, "@@ -3,2 +3,2 @@")],
                split: vec![],
                ..Default::default()
            },
            false,
            true,
        );
        let header = require_text(&painted, "@@ -3,2 +3,2 @@");
        assert!(
            (40.0..200.0).contains(&header.0.x),
            "hunk text must align with code, got {header:?}"
        );
        assert!(
            painted.iter().all(|(_, text)| text != "3"),
            "hunk rows must not paint fake line numbers, got {painted:?}"
        );
    }

    #[test]
    fn five_digit_line_numbers_still_clear_the_sign() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "Index".into(),
                right_label: "Working tree".into(),
                // 10,000 lines force a 5-digit gutter; the inserted row is visible.
                left_text: "f\n".repeat(9999),
                right_text: format!("f\nf\nwide-line\n{}", "f\n".repeat(9997)),
                unified: vec![line(diff::LineKind::Insert, None, Some(10000), "wide-line")],
                split: vec![],
                ..Default::default()
            },
            false,
        );
        let number = require_text(&painted, "3");
        let sign = require_text(&painted, "+");
        assert!(
            sign.0.x > number.0.x + 8.0,
            "5-digit numbers must not overflow into the sign column, got number={} sign={}",
            number.0.x,
            sign.0.x
        );
    }

    #[test]
    fn gutter_digit_columns_grow_with_line_numbers() {
        assert_eq!(diff_gutter_digits(std::iter::empty()), 4);
        assert_eq!(
            diff_gutter_digits(std::iter::once(&line(
                diff::LineKind::Equal,
                Some(9999),
                Some(9999),
                "n",
            ))),
            4
        );
        assert_eq!(
            diff_gutter_digits(std::iter::once(&line(
                diff::LineKind::Equal,
                Some(10000),
                Some(10000),
                "n",
            ))),
            5
        );
    }
    #[test]
    fn insertion_only_split_rows_keep_the_right_column() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "old".into(),
                right_label: "new".into(),
                left_text: String::new(),
                right_text: "insert-only\npaired-right\n".into(),
                unified: vec![],
                split: vec![
                    diff::SplitRow {
                        left: None,
                        right: Some(line(diff::LineKind::Insert, None, Some(1), "insert-only")),
                    },
                    diff::SplitRow {
                        left: Some(line(diff::LineKind::Equal, Some(1), Some(2), "paired-left")),
                        right: Some(line(
                            diff::LineKind::Equal,
                            Some(1),
                            Some(2),
                            "paired-right",
                        )),
                    },
                    diff::SplitRow {
                        left: Some(line(diff::LineKind::Delete, Some(2), None, "delete-only")),
                        right: None,
                    },
                ],
                ..Default::default()
            },
            true,
        );
        assert_eq!(
            require_text(&painted, "insert-only").0.x,
            require_text(&painted, "paired-right").0.x
        );
        assert_eq!(
            require_text(&painted, "delete-only").0.x,
            require_text(&painted, "paired-left").0.x
        );
    }

    fn scroll_doc(
        ctx: &egui::Context,
        doc: &diff::DiffDocument,
        split: bool,
    ) -> (
        Vec<egui::scroll_area::ScrollAreaOutput<()>>,
        Vec<egui::epaint::ClippedShape>,
    ) {
        let mut scroll = None;
        let mut ratio = 0.5;
        let mut sync = 0.0;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 300.0),
                )),
                ..Default::default()
            },
            |ui| {
                scroll = Some(paint_diff_document(
                    ui,
                    doc,
                    split,
                    DiffColors {
                        added: Color32::GREEN,
                        deleted: Color32::RED,
                        accent: Color32::BLUE,
                        text: Color32::WHITE,
                    },
                    "scroll-regression",
                    &mut ratio,
                    &mut sync,
                ));
            },
        );
        output.textures_delta.clear();
        (scroll.unwrap(), output.shapes)
    }

    pub(crate) fn long_doc() -> diff::DiffDocument {
        let unified: Vec<_> = (1..=150)
            .map(|n| {
                line(
                    diff::LineKind::Equal,
                    Some(n),
                    Some(n),
                    &if n == 1 {
                        format!("{}END", "long-line-".repeat(30))
                    } else {
                        format!("short-{n}")
                    },
                )
            })
            .collect();
        let split = unified
            .iter()
            .map(|line| diff::SplitRow {
                left: Some(line.clone()),
                right: Some(line.clone()),
            })
            .collect();
        diff::DiffDocument {
            left_label: "old".into(),
            right_label: "new".into(),
            left_text: String::new(),
            right_text: String::new(),
            unified,
            split,
            ..Default::default()
        }
    }

    #[test]
    fn long_split_lines_have_scroll_space_and_do_not_cross_their_column() {
        let ctx = egui::Context::default();
        let doc = long_doc();
        scroll_doc(&ctx, &doc, true);
        let (areas, _) = scroll_doc(&ctx, &doc, true);
        assert_eq!(areas.len(), 2);
        for area in &areas {
            assert!(area.content_size.x > 1000.0);
            let mut state = area.state;
            let mut offset = state.unclamped_offset();
            offset.x = area.content_size.x - area.inner_rect.width();
            super::super::write_scroll_offset(&mut state, offset);
            state.store(&ctx, area.id);
        }
        let (_, shapes) = scroll_doc(&ctx, &doc, true);
        let mut long: Vec<_> = shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Text(text) = &shape.shape
                    && text.galley.text().ends_with("END")
                {
                    Some((shape.clip_rect, text))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(long.len(), 2);
        long.sort_by(|a, b| a.0.left().total_cmp(&b.0.left()));
        assert!(long[0].0.right() <= long[1].0.left() + 1.0);
        for (clip, text) in long {
            assert!(
                text.pos.x + text.galley.size().x <= clip.right() + 1.0,
                "split line escaped its pane: {:?} vs {:?}",
                text.pos,
                clip
            );
        }
    }

    #[test]
    fn split_row_pitch_matches_the_virtualized_font_height() {
        let ctx = egui::Context::default();
        let doc = long_doc();
        scroll_doc(&ctx, &doc, true);
        let (_, shapes) = scroll_doc(&ctx, &doc, true);
        let painted = painted_text(&shapes);
        let a = require_text(&painted, "short-2");
        let b = require_text(&painted, "short-3");
        let font = ctx.global_style().text_styles[&egui::TextStyle::Monospace].clone();
        let height = ctx.fonts_mut(|fonts| fonts.row_height(&font));
        assert!((b.0.y - a.0.y - height).abs() <= 1.0);
    }

    #[test]
    fn horizontal_scroll_reaches_the_end_of_the_right_split_line() {
        let ctx = egui::Context::default();
        let doc = long_doc();
        scroll_doc(&ctx, &doc, true);
        let (areas, _) = scroll_doc(&ctx, &doc, true);
        for area in &areas {
            let mut state = area.state;
            let mut offset = state.unclamped_offset();
            offset.x = area.content_size.x - area.inner_rect.width();
            super::super::write_scroll_offset(&mut state, offset);
            state.store(&ctx, area.id);
        }
        let (areas, shapes) = scroll_doc(&ctx, &doc, true);
        let end = shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text().ends_with("END") => {
                    Some(text.pos.x + text.galley.size().x)
                }
                _ => None,
            })
            .fold(f32::NEG_INFINITY, f32::max);
        assert!((end - areas[1].inner_rect.right()).abs() <= 1.0);
    }

    #[test]
    fn split_panes_keep_independent_offsets_and_ids() {
        let ctx = egui::Context::default();
        let doc = long_doc();
        // Unified keeps its own offset while rows virtualize.
        scroll_doc(&ctx, &doc, false);
        let (before, _) = scroll_doc(&ctx, &doc, false);
        let width = before[0].content_size.x;
        let mut state = before[0].state;
        super::super::write_scroll_offset(&mut state, egui::vec2(250.0, 900.0));
        state.store(&ctx, before[0].id);
        let (after, _) = scroll_doc(&ctx, &doc, false);
        assert_eq!(after[0].content_size.x, width);
        assert_eq!(after[0].state.unclamped_offset(), egui::vec2(250.0, 900.0));
        // Each split pane scrolls horizontally on its own.
        scroll_doc(&ctx, &doc, true);
        let (areas, _) = scroll_doc(&ctx, &doc, true);
        assert_ne!(before[0].id, areas[0].id);
        assert_ne!(areas[0].id, areas[1].id);
        let mut left_state = areas[0].state;
        let mut offset = left_state.unclamped_offset();
        offset.x = 200.0;
        super::super::write_scroll_offset(&mut left_state, offset);
        left_state.store(&ctx, areas[0].id);
        let (areas, _) = scroll_doc(&ctx, &doc, true);
        assert_eq!(areas[0].state.unclamped_offset().x, 200.0);
        assert_ne!(areas[1].state.unclamped_offset().x, 200.0);
    }

    #[test]
    fn refreshing_a_diff_invalidates_its_cached_width() {
        let ctx = egui::Context::default();
        let doc = long_doc();
        let (long, _) = scroll_doc(&ctx, &doc, false);
        let (short, _) = scroll_doc(&ctx, &sample_doc(), false);
        assert!(long[0].content_size.x > short[0].content_size.x * 2.0);
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
                    paint_diff_document(ui, &doc, true, colors, "drag-test", ratio, &mut sync);
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
                paint_markdown_diff_preview(
                    ui,
                    &doc,
                    &mut previews,
                    std::path::Path::new("/repo/test.md"),
                    "test-preview",
                );
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
        let mut doc =
            diff::build_with_context(std::path::Path::new("file.rs"), "old\n", "new\n", false, 3);
        doc.read_only = true;
        doc.left_label = "Have revision".into();
        doc.right_label = "Workspace".into();
        let painted = paint_doc(doc, false);
        assert!(painted.iter().any(|(_, text)| text == "Read-only"));
        assert!(!painted.iter().any(|(_, text)| text == "Editable"));
    }
}
