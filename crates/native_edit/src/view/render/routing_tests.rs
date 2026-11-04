//! Key routing: printable chars arrive as text in every mode, and vim
//! owns its Ctrl chords on every platform.
use crate::doc::Doc;
use crate::highlight::{HighlightCache, Language};
use crate::view::{ShowSourceInput, SourceOptions, show_source};
use crate::vim::{ModalEngine, Mode, VimEngine};

fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

/// One frame of the real widget with `events`.
fn frame(doc: &mut Doc, engine: &mut VimEngine, events: Vec<egui::Event>) {
    let ctx = egui::Context::default();
    for events in [Vec::new(), events] {
        let mut output = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    show_source(ShowSourceInput {
                        ui,
                        doc: &mut *doc,
                        engine: &mut *engine,
                        options: &SourceOptions::default(),
                        highlight: &mut HighlightCache::default(),
                        language: Language::Plain,
                        id: "routing",
                    });
                });
            },
        );
        output.textures_delta.clear();
    }
}

/// A shifted symbol as a backend sends it: the physical key, then text.
fn typed(physical: egui::Key, text: &str) -> Vec<egui::Event> {
    vec![
        key(physical, egui::Modifiers::SHIFT),
        egui::Event::Text(text.into()),
    ]
}

#[test]
fn shifted_symbols_reach_vim_as_their_text() {
    let mut doc = Doc::new("a\n");
    let mut engine = VimEngine::new();
    // Shift+. is `>`, not dot-repeat; `>>` indents the line.
    frame(&mut doc, &mut engine, typed(egui::Key::Period, ">"));
    frame(&mut doc, &mut engine, typed(egui::Key::Period, ">"));
    assert_eq!(doc.text(), "    a\n");
    frame(&mut doc, &mut engine, typed(egui::Key::Comma, "<"));
    frame(&mut doc, &mut engine, typed(egui::Key::Comma, "<"));
    assert_eq!(doc.text(), "a\n");
    // `~` from a non-US layout arrives only as text.
    frame(&mut doc, &mut engine, vec![egui::Event::Text("~".into())]);
    assert_eq!(doc.text(), "A\n");
}

#[test]
fn register_prefix_and_underscore_type_through_text() {
    let mut doc = Doc::new("one\ntwo\n");
    let mut engine = VimEngine::new();
    for text in ["\"", "a", "y", "y", "j", "\"", "a", "p", "_"] {
        frame(&mut doc, &mut engine, vec![egui::Event::Text(text.into())]);
    }
    assert_eq!(doc.text(), "one\ntwo\none\n");
    assert_eq!(engine.mode(), Mode::Normal);
}

#[test]
fn ctrl_r_redoes_in_normal_mode_on_linux_and_macos() {
    for modifiers in [
        egui::Modifiers {
            ctrl: true,
            command: true,
            ..egui::Modifiers::NONE
        },
        egui::Modifiers::CTRL,
    ] {
        let mut doc = Doc::new("abc\n");
        let mut engine = VimEngine::new();
        frame(&mut doc, &mut engine, vec![egui::Event::Text("x".into())]);
        frame(&mut doc, &mut engine, vec![egui::Event::Text("u".into())]);
        assert_eq!(doc.text(), "abc\n");
        frame(&mut doc, &mut engine, vec![key(egui::Key::R, modifiers)]);
        assert_eq!(doc.text(), "bc\n");
        // Not the IDE replace prompt.
        assert_eq!(engine.mode(), Mode::Normal);
    }
}

#[test]
fn ctrl_a_increments_instead_of_selecting_all_in_normal_mode() {
    let mut doc = Doc::new("n = 41\n");
    let mut engine = VimEngine::new();
    let linux_ctrl = egui::Modifiers {
        ctrl: true,
        command: true,
        ..egui::Modifiers::NONE
    };
    frame(&mut doc, &mut engine, vec![key(egui::Key::A, linux_ctrl)]);
    assert_eq!(doc.text(), "n = 42\n");
    assert_eq!(engine.mode(), Mode::Normal);
}
