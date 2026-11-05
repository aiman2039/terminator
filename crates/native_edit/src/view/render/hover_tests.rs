//! What the source view gives a host hover popup: the text under a still
//! pointer, the Quick Documentation keys, and Copy for a popup that holds
//! it.
use super::{ShowSourceInput, SourceOptions, SourceOutcome, show_source, word_span};
use crate::doc::Doc;
use crate::highlight::{HighlightCache, Language};
use crate::vim::{Effect, VimEngine};

/// One view over several frames: egui hit-tests against last frame's
/// widgets, so the context lives across them.
struct Harness {
    ctx: egui::Context,
    doc: Doc,
    engine: VimEngine,
    highlight: HighlightCache,
    options: SourceOptions,
}

/// What one frame produced.
struct Frame {
    outcome: SourceOutcome,
    /// Width of one monospace cell.
    advance: f32,
    /// Text the frame put on the clipboard.
    copied: Option<String>,
    /// A Copy event was still there after the view ran.
    copy_left: bool,
}

impl Harness {
    fn new(text: &str) -> Self {
        Self {
            ctx: egui::Context::default(),
            doc: Doc::new(text),
            engine: VimEngine::new(),
            highlight: HighlightCache::default(),
            options: SourceOptions {
                hover_popup: true,
                show_line_numbers: false,
                show_status: false,
                sticky_headers: false,
                ..SourceOptions::default()
            },
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> Frame {
        let mut shown = None;
        let input = egui::RawInput {
            events,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 300.0),
            )),
            ..Default::default()
        };
        let mut output = self.ctx.run_ui(input, |ui| {
            // Measured as the view measures it (headless fonts may have no
            // glyphs, which the view treats as one point).
            let font = egui::FontId::monospace(self.options.font_size);
            let advance = ui.fonts_mut(|fonts| fonts.glyph_width(&font, 'm').max(1.0));
            let outcome = show_source(ShowSourceInput {
                ui,
                doc: &mut self.doc,
                engine: &mut self.engine,
                options: &self.options,
                highlight: &mut self.highlight,
                language: Language::Plain,
                id: "hover",
            });
            let copy_left = ui.input(|input| {
                input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::Copy))
            });
            shown = Some((outcome, advance, copy_left));
        });
        output.textures_delta.clear();
        let copied = output
            .platform_output
            .commands
            .iter()
            .find_map(|command| match command {
                egui::OutputCommand::CopyText(text) => Some(text.clone()),
                _ => None,
            });
        let (outcome, advance, copy_left) = shown.expect("frame ran");
        Frame {
            outcome,
            advance,
            copied,
            copy_left,
        }
    }

    /// Move the pointer to `pos`, then let it rest a frame.
    fn rest_at(&mut self, pos: egui::Pos2) -> Frame {
        self.frame(vec![egui::Event::PointerMoved(pos)]);
        self.frame(Vec::new())
    }
}

/// The middle of char `col` on painted row `row`.
#[expect(clippy::cast_precision_loss, reason = "Small test indices.")]
fn at(frame: &Frame, (row, col): (usize, usize)) -> egui::Pos2 {
    let origin = frame.outcome.content_rect.min;
    egui::pos2(
        origin.x + (col as f32 + 0.5) * frame.advance,
        origin.y + (row as f32 + 0.5) * frame.outcome.row_height,
    )
}

fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

#[test]
fn word_span_finds_the_identifier_that_holds_a_char() {
    let text = "let value_1 = größe;";
    assert_eq!(word_span(text, 6), Some((4, 11)));
    assert_eq!(word_span(text, 4), Some((4, 11)));
    assert_eq!(word_span(text, 16), Some((14, 19)));
    assert_eq!(word_span(text, 12), None, "punctuation");
    assert_eq!(word_span(text, 3), None, "a blank");
    assert_eq!(word_span(text, 99), None, "past the end");
}

#[test]
fn the_word_under_a_still_pointer_is_reported_with_its_rect() {
    let mut view = Harness::new("let value_1 = other;\n");
    let first = view.frame(Vec::new());
    let spot = view
        .rest_at(at(&first, (0, 6)))
        .outcome
        .hover
        .expect("a spot");
    assert_eq!(
        (spot.line, spot.col, spot.start, spot.end, spot.word),
        (0, 6, 4, 11, true)
    );
    let rect = first.outcome.content_rect;
    assert!(
        (spot.rect.left() - (rect.left() + 4.0 * first.advance)).abs() < 1.0,
        "the popup goes under the word: {spot:?}"
    );
    assert!((spot.rect.bottom() - (rect.top() + first.outcome.row_height)).abs() < 1.0);
    // Punctuation is a one-char spot (it may carry a problem).
    let eq = view
        .rest_at(at(&first, (0, 12)))
        .outcome
        .hover
        .expect("a spot");
    assert_eq!((eq.start, eq.end, eq.word), (12, 13, false));
}

#[test]
fn no_spot_past_a_line_while_a_button_is_down_or_when_the_host_has_no_popup() {
    let mut view = Harness::new("let x = 1;\n\nfn f() {}\n");
    let first = view.frame(Vec::new());
    assert_eq!(view.rest_at(at(&first, (0, 30))).outcome.hover, None);
    // An empty line keeps its first cell, where a problem is underlined.
    let blank = view
        .rest_at(at(&first, (1, 0)))
        .outcome
        .hover
        .expect("first cell");
    assert_eq!((blank.line, blank.start, blank.end), (1, 0, 1));
    assert_eq!(view.rest_at(at(&first, (1, 3))).outcome.hover, None);
    let word = at(&first, (2, 4));
    assert!(view.rest_at(word).outcome.hover.is_some());
    let press = egui::Event::PointerButton {
        pos: word,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    };
    assert_eq!(view.frame(vec![press]).outcome.hover, None, "a drag starts");
    let release = egui::Event::PointerButton {
        pos: word,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::NONE,
    };
    view.frame(vec![release]);
    assert!(view.frame(Vec::new()).outcome.hover.is_some());
    view.options.hover_popup = false;
    assert_eq!(view.frame(Vec::new()).outcome.hover, None);
}

#[test]
fn f1_and_ctrl_j_ask_for_quick_documentation() {
    let linux_ctrl = egui::Modifiers {
        ctrl: true,
        command: true,
        ..egui::Modifiers::NONE
    };
    let mac_cmd = egui::Modifiers {
        mac_cmd: true,
        command: true,
        ..egui::Modifiers::NONE
    };
    let cases = [
        (key(egui::Key::F1, egui::Modifiers::NONE), true),
        (key(egui::Key::J, egui::Modifiers::CTRL), true),
        (key(egui::Key::J, linux_ctrl), true),
        (key(egui::Key::F1, egui::Modifiers::SHIFT), false),
        (key(egui::Key::J, mac_cmd), false),
        // Ctrl+Shift+J joins lines.
        (
            key(egui::Key::J, egui::Modifiers::CTRL | egui::Modifiers::SHIFT),
            false,
        ),
    ];
    for (event, hover) in cases {
        let mut view = Harness::new("one\ntwo\n");
        view.frame(Vec::new());
        let effects = view.frame(vec![event.clone()]).outcome.effects;
        assert_eq!(effects.contains(&Effect::Hover), hover, "{event:?}");
    }
}

#[test]
fn copy_is_left_to_a_host_popup_that_holds_it() {
    for yield_copy in [false, true] {
        let mut view = Harness::new("alpha beta\n");
        view.options.yield_copy = yield_copy;
        view.frame(Vec::new());
        view.engine.select_all(&view.doc);
        let copy = view.frame(vec![egui::Event::Copy]);
        if yield_copy {
            assert_eq!(copy.copied, None, "the view copies nothing");
            assert!(copy.copy_left, "the popup still sees Copy");
        } else {
            assert!(copy.copied.is_some_and(|text| text.contains("alpha beta")));
            assert!(!copy.copy_left);
        }
    }
}
