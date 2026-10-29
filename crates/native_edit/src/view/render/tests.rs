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
