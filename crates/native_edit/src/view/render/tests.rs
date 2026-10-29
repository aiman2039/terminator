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
fn line_spacing_adds_row_height_and_centers_the_text() {
    use super::geometry::{Geometry, MeasureInput};
    let ctx = egui::Context::default();
    let doc = Doc::new("one\ntwo\n");
    let mut rows = Vec::new();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        for line_spacing in [1.0, 1.5] {
            let geometry = Geometry::measure(MeasureInput {
                ui,
                doc: &doc,
                options: &SourceOptions {
                    line_spacing,
                    ..Default::default()
                },
                id: "spacing.md",
            });
            rows.push((geometry.row_height, geometry.text_top));
        }
    });
    output.textures_delta.clear();
    let [(plain, plain_top), (spaced, spaced_top)] = rows[..] else {
        panic!("two measurements");
    };
    assert!(plain_top.abs() < f32::EPSILON);
    assert!(spaced > plain * 1.4);
    assert!((spaced_top - ((spaced - plain) * 0.5).floor()).abs() < 1.0);
}
