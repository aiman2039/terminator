use super::{ShowSourceInput, SourceOptions, show_source};
use crate::doc::{Buffer, Doc};
use crate::highlight::{HighlightCache, Language};
use crate::vim::VimEngine;

#[test]
fn markdown_source_uses_wrapped_viewport_height() {
    let ctx = egui::Context::default();
    let mut doc = Doc::new(format!("{}\n", "long source words ".repeat(25)));
    let mut engine = VimEngine::new();
    let mut highlight = HighlightCache::default();
    let mut visual_count = 0;
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(200.0, 220.0),
            )),
            ..Default::default()
        },
        |ui| {
            ui.set_width(200.0);
            let outcome = show_source(ShowSourceInput {
                ui,
                doc: &mut doc,
                engine: &mut engine,
                options: &SourceOptions {
                    soft_wrap: true,
                    ..Default::default()
                },
                highlight: &mut highlight,
                language: Language::Markdown,
                id: "wrapped.md",
            });
            visual_count = outcome.visual_rows;
        },
    );
    output.textures_delta.clear();
    assert!(visual_count > doc.line_count());
}

#[test]
fn line_spacing_scales_rows_to_whole_pixels_and_centers_text() {
    use super::geometry::row_metrics;
    // 1.2 × 15.4 = 18.48 → 18.5 at 2x, text centered in the spare 3.1.
    let (row, dy) = row_metrics(15.4, 1.2, 2.0);
    assert_eq!(row, 18.5);
    assert_eq!(dy, 1.5);
    // Out-of-range factors clamp: never shorter than the glyph row.
    assert_eq!(row_metrics(16.0, 0.5, 1.0), (16.0, 0.0));
    assert_eq!(row_metrics(16.0, 3.0, 1.0), (32.0, 8.0));
    assert_eq!(row_metrics(16.0, f32::NAN, 1.0), (16.0, 0.0));
}

#[test]
fn indent_guides_stand_one_indent_level_apart() {
    use super::geometry::{Geometry, MeasureInput};
    use super::guides::{guide_stops, lead_cells};
    use crate::indent::IndentStyle;
    // Two tabs shown eight wide: guides at cells 0 and 8.
    assert_eq!(lead_cells("\t\tx", 8), Some(16));
    assert_eq!(guide_stops(16, 8).collect::<Vec<_>>(), [0, 8]);
    // Two-space levels: a guide every two cells.
    assert_eq!(guide_stops(4, 2).collect::<Vec<_>>(), [0, 2]);
    // A blank line keeps the guides of the line above.
    assert_eq!(lead_cells(" \t ", 4), None);
    // The view takes both widths from the document's indent style.
    let ctx = egui::Context::default();
    let mut doc = Doc::new("x\n");
    doc.set_indent_style(IndentStyle {
        tab_width: 8,
        ..IndentStyle::spaces(2)
    });
    let mut seen = None;
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        let geometry = Geometry::measure(MeasureInput {
            ui,
            doc: &doc,
            options: &SourceOptions::default(),
            id: "guides",
        });
        seen = Some((geometry.tab, geometry.indent));
    });
    output.textures_delta.clear();
    assert_eq!(seen, Some((8, 2)));
}

#[test]
fn snapping_rounds_to_physical_pixels() {
    use super::geometry::snap;
    assert_eq!(snap(10.3, 1.0), 10.0);
    assert_eq!(snap(10.3, 2.0), 10.5);
    assert_eq!(snap(10.2, 2.0), 10.0);
    assert_eq!(snap(7.77, 0.0), 7.77);
}

#[test]
fn caret_blinks_every_530ms_and_input_shows_it_again() {
    use super::blink::{BLINK_HALF, BlinkState, blink_phase};
    use crate::vim::Cursor;
    assert!(blink_phase(0.0).0);
    assert!(blink_phase(BLINK_HALF - 0.01).0);
    assert!(!blink_phase(BLINK_HALF + 0.01).0);
    assert!(blink_phase(BLINK_HALF * 2.0 + 0.01).0);
    let (_, until) = blink_phase(0.2);
    assert!((until - (BLINK_HALF - 0.2)).abs() < 1e-9);
    let at = Cursor { line: 1, col: 2 };
    let started = BlinkState::next(None, 5.0, at, false);
    assert_eq!(started.since, 5.0);
    // Idle frames keep the cycle; a key or a caret move restarts it.
    assert_eq!(BlinkState::next(Some(started), 6.0, at, false), started);
    assert_eq!(BlinkState::next(Some(started), 6.0, at, true).since, 6.0);
    let moved = Cursor { line: 1, col: 3 };
    assert_eq!(
        BlinkState::next(Some(started), 7.0, moved, false).since,
        7.0
    );
}

#[test]
fn host_spacers_add_rows_and_scroll_offset_positions_the_view() {
    use super::{LineFill, RangeFill, source_row_height};
    let ctx = egui::Context::default();
    let mut doc = Doc::new("line\n".repeat(80));
    let mut engine = VimEngine::new();
    let mut highlight = HighlightCache::default();
    let options = SourceOptions {
        spacers: std::sync::Arc::new(vec![(1, 2), (doc.line_count(), 1)]),
        spacer_fill: Some(egui::Color32::GRAY),
        line_fills: std::sync::Arc::new(vec![LineFill {
            line: 2,
            color: egui::Color32::RED,
        }]),
        range_fills: std::sync::Arc::new(vec![RangeFill {
            line: 2,
            start: 0,
            end: 4,
            color: egui::Color32::BLUE,
        }]),
        scroll_offset: Some(60.0),
        sticky_headers: false,
        show_status: false,
        ..Default::default()
    };
    let mut seen = None;
    for _ in 0..2 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(300.0, 200.0),
                )),
                ..Default::default()
            },
            |ui| {
                let expected = source_row_height(ui, &options);
                let outcome = show_source(ShowSourceInput {
                    ui,
                    doc: &mut doc,
                    engine: &mut engine,
                    options: &options,
                    highlight: &mut highlight,
                    language: Language::Plain,
                    id: "spaced",
                });
                seen = Some((outcome.visual_rows, outcome.row_height, expected, {
                    outcome.scroll.state.unclamped_offset().y
                }));
            },
        );
        output.textures_delta.clear();
    }
    let (rows, row_height, expected, offset) = seen.expect("frame ran");
    assert_eq!(rows, doc.line_count() + 3);
    assert!((row_height - expected).abs() < f32::EPSILON);
    assert!(
        (offset - 60.0).abs() < 0.5,
        "the host offset positions the view: {offset}"
    );
}

#[test]
fn multi_line_diagnostics_underline_every_line_they_cover() {
    use crate::lsp::{Diagnostic, Position, Range, Severity};
    let doc = Doc::new("call(\n    a,\n    b)\nnext\n");
    let diagnostic = Diagnostic {
        range: Range {
            start: Position {
                line: 0,
                character: 0,
            },
            end: Position {
                line: 2,
                character: 6,
            },
        },
        severity: Some(Severity::Error),
        message: "bad call".into(),
    };
    let heads = super::squiggles(&doc, std::slice::from_ref(&diagnostic));
    assert_eq!(heads.len(), 1, "F8 stops once per problem");
    let tails = super::squiggle_tails(&doc, &[diagnostic]);
    let spans: Vec<_> = tails.iter().map(|s| (s.line, s.start, s.end)).collect();
    assert_eq!(spans, [(1, 4, 6), (2, 4, 6)]);
}

#[test]
fn error_lens_and_overview_marks_paint_without_panicking() {
    let ctx = egui::Context::default();
    let mut doc = Doc::new("let x = 1;\nlet y = 2;\n");
    let mut engine = VimEngine::new();
    let mut highlight = HighlightCache::default();
    let squiggle = super::Squiggle {
        line: 0,
        start: 4,
        end: 5,
        severity: crate::lsp::Severity::Warning,
        message: "unused\nmore".into(),
    };
    let options = SourceOptions {
        squiggles: std::sync::Arc::new(vec![squiggle]),
        error_lens: true,
        find_matches: std::sync::Arc::new(vec![(1, 4, 5)]),
        change_marks: std::sync::Arc::new(vec![super::ChangeMark {
            start: 1,
            end: 2,
            kind: super::ChangeKind::Added,
        }]),
        ..SourceOptions::default()
    };
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        let outcome = show_source(ShowSourceInput {
            ui,
            doc: &mut doc,
            engine: &mut engine,
            options: &options,
            highlight: &mut highlight,
            language: Language::Rust,
            id: "lens",
        });
        assert!(outcome.effects.is_empty());
    });
    output.textures_delta.clear();
}

/// Draw one frame of a 500 × 300 source view; returns its footer band and
/// the reported host cells.
fn footer_cells(options: &SourceOptions) -> (Option<egui::Rect>, Vec<egui::Rect>) {
    let ctx = egui::Context::default();
    let mut doc = Doc::new("text\n");
    let mut engine = VimEngine::new();
    let mut highlight = HighlightCache::default();
    let mut seen = (None, Vec::new());
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(500.0, 300.0),
            )),
            ..Default::default()
        },
        |ui| {
            let outcome = show_source(ShowSourceInput {
                ui,
                doc: &mut doc,
                engine: &mut engine,
                options,
                highlight: &mut highlight,
                language: Language::Plain,
                id: "cells",
            });
            seen = (outcome.footer_rect, outcome.status_rects);
        },
    );
    output.textures_delta.clear();
    seen
}

#[test]
fn footer_reports_each_host_status_cell_in_order() {
    let items = std::sync::Arc::new(vec!["LF".into(), "UTF-8".into(), "4 spaces".into()]);
    let (footer, cells) = footer_cells(&SourceOptions {
        status_items: std::sync::Arc::clone(&items),
        ..SourceOptions::default()
    });
    let footer = footer.expect("the footer is shown");
    assert_eq!(cells.len(), 3, "{cells:?}");
    for pair in cells.windows(2) {
        assert!(pair[0].right() < pair[1].left(), "{cells:?}");
    }
    for cell in &cells {
        assert!(cell.width() > 0.0, "{cell:?}");
        assert!(footer.contains_rect(*cell), "{cell:?} in {footer:?}");
        assert_eq!(cell.y_range(), footer.y_range());
    }
    let (hidden, cells) = footer_cells(&SourceOptions {
        status_items: items,
        show_status: false,
        ..SourceOptions::default()
    });
    assert!(hidden.is_none() && cells.is_empty());
}

/// One unfocused 300×200 frame of `doc` at `time` seconds (scroll
/// animations finish once time moves on); returns the scroll offset and
/// the row pitch.
fn source_frame(
    (ctx, time): (&egui::Context, f64),
    view: (&mut Doc, &mut VimEngine, &mut HighlightCache),
    options: &SourceOptions,
    language: Language,
) -> (f32, f32) {
    let (doc, engine, highlight) = view;
    let mut seen = (0.0, 0.0);
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(300.0, 200.0),
            )),
            time: Some(time),
            ..Default::default()
        },
        |ui| {
            let outcome = show_source(ShowSourceInput {
                ui,
                doc,
                engine,
                options,
                highlight,
                language,
                id: "reopened",
            });
            seen = (
                outcome.scroll.state.unclamped_offset().y,
                outcome.row_height,
            );
        },
    );
    output.textures_delta.clear();
    seen
}

#[test]
fn top_line_puts_that_line_on_the_first_row_past_folds() {
    let ctx = egui::Context::default();
    let text = format!(
        "fn a() {{\n{}}}\n{}",
        "    x();\n".repeat(49),
        "line\n".repeat(300)
    );
    let mut doc = Doc::new(text);
    let mut engine = VimEngine::new();
    let mut highlight = HighlightCache::default();
    highlight.restore_folds(&doc, Language::Rust, &[0, 7]);
    assert_eq!(
        highlight.folds().folded_starts(),
        [0],
        "line 7 starts no region"
    );
    let restore = SourceOptions {
        top_line: Some(100),
        sticky_headers: false,
        ..Default::default()
    };
    let view = (&mut doc, &mut engine, &mut highlight);
    let (offset, row) = source_frame((&ctx, 0.0), view, &restore, Language::Rust);
    // Lines 1..=50 are folded away, so line 100 is row 50.
    assert!(
        (offset - 50.0 * row).abs() < 0.5,
        "offset {offset}, row {row}"
    );
    // Later frames without the option keep the position.
    let later = SourceOptions {
        sticky_headers: false,
        ..Default::default()
    };
    let view = (&mut doc, &mut engine, &mut highlight);
    let (offset, _) = source_frame((&ctx, 1.0), view, &later, Language::Rust);
    assert!((offset - 50.0 * row).abs() < 0.5, "offset {offset}");
    assert_eq!(
        engine.scroll_line(),
        100,
        "an unfocused view reports its top line"
    );
}

#[test]
fn a_scroll_request_before_the_first_frame_reveals_the_caret() {
    let ctx = egui::Context::default();
    let mut doc = Doc::new("line\n".repeat(400));
    let mut engine = VimEngine::new();
    let _ = engine.place_cursor(&mut doc, 300, 0);
    engine.request_scroll();
    let mut highlight = HighlightCache::default();
    let options = SourceOptions {
        sticky_headers: false,
        ..Default::default()
    };
    for second in 0..4 {
        let view = (&mut doc, &mut engine, &mut highlight);
        source_frame((&ctx, f64::from(second)), view, &options, Language::Plain);
    }
    let top = engine.scroll_line();
    assert!(top > 280 && top <= 300, "line 300 is on screen: top {top}");
}

/// Menu row renderer that remembers each row's rect by label.
fn recording_item(ui: &mut egui::Ui, label: &str, _icon: &str, _keys: &str) -> egui::Response {
    let response = ui.button(label);
    let rect = response.rect;
    ui.ctx()
        .data_mut(|data| data.insert_temp(egui::Id::unique(("menu-row", label)), rect));
    response
}

#[test]
fn context_menu_git_items_report_the_compare_pick() {
    use super::GitCompare;
    let ctx = egui::Context::default();
    let mut doc = Doc::new("short file");
    let mut engine = VimEngine::new();
    let mut highlight = HighlightCache::default();
    let options = SourceOptions {
        blame: Some(false),
        menu_item: Some(recording_item),
        ..Default::default()
    };
    let mut draw = |events: Vec<egui::Event>| {
        let mut seen = None;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(600.0, 700.0),
            )),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            let outcome = show_source(ShowSourceInput {
                ui,
                doc: &mut doc,
                engine: &mut engine,
                options: &options,
                highlight: &mut highlight,
                language: Language::Plain,
                id: "git-menu",
            });
            seen = Some((outcome.content_rect, outcome.git_compare));
        });
        output.textures_delta.clear();
        seen.expect("source outcome")
    };
    let button = |pos, button, pressed| egui::Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let (content, _) = draw(vec![]);
    let at = content.left_top() + egui::vec2(60.0, 20.0);
    let secondary = egui::PointerButton::Secondary;
    draw(vec![egui::Event::PointerMoved(at)]);
    draw(vec![button(at, secondary, true)]);
    draw(vec![button(at, secondary, false)]);
    let row_rect = |label: &str| {
        ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("menu-row", label))))
    };
    for _ in 0..3 {
        draw(vec![]);
    }
    assert!(
        row_rect("Compare with…").is_some(),
        "both compare rows show"
    );
    let item = row_rect("Compare with Revision…")
        .expect("menu row painted")
        .center();
    let primary = egui::PointerButton::Primary;
    draw(vec![egui::Event::PointerMoved(item)]);
    draw(vec![button(item, primary, true)]);
    let (_, picked) = draw(vec![button(item, primary, false)]);
    assert_eq!(picked, Some(GitCompare::History));
}
