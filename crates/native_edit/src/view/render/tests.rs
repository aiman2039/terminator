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
            let outcome = show_source(
                ui,
                &mut doc,
                &mut engine,
                ShowSourceInput {
                    options: &SourceOptions {
                        soft_wrap: true,
                        ..Default::default()
                    },
                    highlight: &mut highlight,
                    language: Language::Markdown,
                    id: "wrapped.md",
                },
            );
            visual_count = outcome.visual_rows;
        },
    );
    output.textures_delta.clear();
    assert!(visual_count > doc.line_count());
}
