use super::super::*;
use super::diff_paint::{
    DiffColors, diff_gutter_digits, paint_diff_document, paint_markdown_diff_preview,
};
use super::header::{HeaderAction, header_action_view};
use super::tabs::cached_terminal_theme;
#[cfg(test)]
pub(crate) mod tests {

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
            right_text: String::new(),
            unified: vec![sample_line()],
            split: vec![],
        }
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
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
        let tab = Tab::Diff {
            cwd: "/repo".into(),
            path: "/repo/file.rs".into(),
            staged: false,
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
            .find(|(_, text)| text == needle)
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
                left_text: String::new(),
                right_text: String::new(),
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
                left_text: String::new(),
                right_text: String::new(),
                unified: vec![line(
                    diff::LineKind::Delete,
                    Some(100),
                    None,
                    "removed-line",
                )],
                split: vec![],
            },
            false,
        );
        let number = require_text(&painted, "100");
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
                left_text: String::new(),
                right_text: String::new(),
                unified: vec![line(diff::LineKind::Insert, None, Some(102), "added-line")],
                split: vec![],
            },
            false,
        );
        let number = require_text(&painted, "102");
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
                left_text: String::new(),
                right_text: String::new(),
                unified: vec![],
                split: vec![diff::SplitRow {
                    left: Some(line(diff::LineKind::Delete, Some(5), None, "left-only")),
                    right: Some(line(diff::LineKind::Insert, None, Some(6), "right-only")),
                }],
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
                left_text: String::new(),
                right_text: String::new(),
                unified: vec![
                    line(diff::LineKind::Equal, Some(1), Some(1), "row-a"),
                    line(diff::LineKind::Equal, Some(2), Some(2), "row-b"),
                ],
                split: vec![],
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
                left_text: String::new(),
                right_text: String::new(),
                unified: vec![line(diff::LineKind::Equal, Some(97), Some(97), "unchanged")],
                split: vec![],
            },
            false,
        );
        let numbers: Vec<_> = painted.iter().filter(|(_, text)| text == "97").collect();
        assert_eq!(
            numbers.len(),
            2,
            "unified equal lines paint old and new numbers separately, got {painted:?}"
        );
        let gap = (numbers[1].0.x - numbers[0].0.x).abs();
        assert!(
            gap > 8.0,
            "old and new 97 must be distinct columns, got {} and {}",
            numbers[0].0.x,
            numbers[1].0.x
        );
    }

    #[test]
    fn hunk_header_paints_no_line_numbers() {
        let painted = paint_doc(
            diff::DiffDocument {
                left_label: "Index".into(),
                right_label: "Working tree".into(),
                left_text: String::new(),
                right_text: String::new(),
                unified: vec![line(diff::LineKind::Hunk, None, None, "@@ -3,2 +3,2 @@")],
                split: vec![],
            },
            false,
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
                left_text: String::new(),
                right_text: String::new(),
                unified: vec![line(diff::LineKind::Insert, None, Some(10000), "wide-line")],
                split: vec![],
            },
            false,
        );
        let number = require_text(&painted, "10000");
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
                right_text: String::new(),
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
            state.offset.x = area.content_size.x - area.inner_rect.width();
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
            state.offset.x = area.content_size.x - area.inner_rect.width();
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
        state.offset = egui::vec2(250.0, 900.0);
        state.store(&ctx, before[0].id);
        let (after, _) = scroll_doc(&ctx, &doc, false);
        assert_eq!(after[0].content_size.x, width);
        assert_eq!(after[0].state.offset, egui::vec2(250.0, 900.0));
        // Each split pane scrolls horizontally on its own.
        scroll_doc(&ctx, &doc, true);
        let (areas, _) = scroll_doc(&ctx, &doc, true);
        assert_ne!(before[0].id, areas[0].id);
        assert_ne!(areas[0].id, areas[1].id);
        let mut left_state = areas[0].state;
        left_state.offset.x = 200.0;
        left_state.store(&ctx, areas[0].id);
        let (areas, _) = scroll_doc(&ctx, &doc, true);
        assert_eq!(areas[0].state.offset.x, 200.0);
        assert_ne!(areas[1].state.offset.x, 200.0);
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
                d.get_temp::<egui::Rect>(egui::Id::new(("fixture-target", "diff-split-handle")))
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
        };
        let ctx = egui::Context::default();
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
}
