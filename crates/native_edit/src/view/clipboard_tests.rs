//! Clipboard events reach the source view.
use super::super::highlight::{HighlightCache, Language};
use super::super::vim::VimEngine;
use super::render::{ShowSourceInput, SourceOptions, show_source};

#[test]
fn copy_and_cut_events_reach_the_clipboard() {
    for (event, copied, left) in [
        (egui::Event::Copy, "ab", "ab\ncd\n"),
        (egui::Event::Cut, "ab", "\ncd\n"),
    ] {
        let ctx = egui::Context::default();
        let mut doc = crate::doc::Doc::new("ab\ncd\n");
        let mut engine = VimEngine::new();
        engine.select_word_at(&mut doc, 0, 0);
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![event],
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    show_source(ShowSourceInput {
                        ui,
                        doc: &mut doc,
                        engine: &mut engine,
                        options: &SourceOptions::default(),
                        highlight: &mut HighlightCache::default(),
                        language: Language::Plain,
                        id: "headless",
                    });
                });
            },
        );
        output.textures_delta.clear();
        let texts: Vec<_> = output
            .platform_output
            .commands
            .iter()
            .filter_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, [copied]);
        assert_eq!(doc.text(), left);
    }
}
