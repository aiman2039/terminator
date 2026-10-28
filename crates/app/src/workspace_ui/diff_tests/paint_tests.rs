use super::*;

#[test]
fn reopened_diff_uses_unified_after_default_changes() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    app.selected = Some("project".into());
    let tab = Tab::Diff {
        remote: None,
        cwd: dir.path().into(),
        path: dir.path().join("file.md"),
        staged: false,
    };
    let Tab::Diff {
        cwd, path, staged, ..
    } = &tab
    else {
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
            app.file_action(FileActionInput {
                ui,
                action: if *staged {
                    FileAction::NativeStagedDiff
                } else {
                    FileAction::NativeWorkingDiff
                },
                path,
                line: None,
            });
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
            // Painted panes: the editable split derives its left pane
            // from the real texts, which this synthetic doc lacks.
            read_only: true,
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
