//! Key routing: printable chars arrive as text in every mode, vim owns
//! its Ctrl chords on every platform, and Start New Line keys reach the
//! editor in modeless editing and vim Insert mode.
use crate::doc::Doc;
use crate::highlight::{HighlightCache, Language};
use crate::view::{
    IdeEdit, IdeEditKeyInput, ShowSourceInput, SourceOptions, ide_edit_in_vim, ide_edit_key,
    show_source,
};
use crate::vim::{Cursor, Effect, ModalEngine, Mode, VimEngine};

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
    frame_with(doc, engine, &SourceOptions::default(), events);
}

/// One frame of the real widget with `options` and `events`.
fn frame_with(
    doc: &mut Doc,
    engine: &mut VimEngine,
    options: &SourceOptions,
    events: Vec<egui::Event>,
) {
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
                        options,
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

/// `snippet` inserted at char `at` with its session started, as an
/// accepted completion leaves it.
fn with_snippet(doc: &mut Doc, engine: &mut VimEngine, at: usize, snippet: &str) {
    use crate::doc::Buffer as _;
    let snippet = crate::lsp::parse_snippet(snippet, &crate::lsp::SnippetVars::default());
    doc.insert(at, &snippet.text);
    engine.start_snippet(doc, at, &snippet);
}

#[test]
fn tab_and_shift_tab_move_between_snippet_stops_and_enter_finishes() {
    let mut doc = Doc::new("let v = ;\n");
    let mut engine = VimEngine::new();
    engine.set_modeless(true);
    with_snippet(&mut doc, &mut engine, 8, "foo(${1:a}, ${2:b})$0");
    let modeless = SourceOptions {
        vim: false,
        ..SourceOptions::default()
    };
    let none = egui::Modifiers::NONE;
    // Text and Tab in one frame keep their order.
    let typed = vec![egui::Event::Text("x".into()), key(egui::Key::Tab, none)];
    frame_with(&mut doc, &mut engine, &modeless, typed);
    assert_eq!(engine.copy_text(&doc), "b");
    let back = vec![key(egui::Key::Tab, egui::Modifiers::SHIFT)];
    frame_with(&mut doc, &mut engine, &modeless, back);
    assert_eq!(engine.copy_text(&doc), "x");
    frame_with(
        &mut doc,
        &mut engine,
        &modeless,
        vec![key(egui::Key::Enter, none)],
    );
    assert_eq!(doc.text(), "let v = foo(x, b);\n", "Enter adds no line");
    assert_eq!(engine.cursor().col, 17, "on `$0`");
    assert!(!engine.snippet_active());
    // Tab is Tab again: spaces up to the next indent stop.
    frame_with(
        &mut doc,
        &mut engine,
        &modeless,
        vec![key(egui::Key::Tab, none)],
    );
    assert_eq!(doc.text(), "let v = foo(x, b)   ;\n");
}

#[test]
fn vim_insert_typing_replaces_a_stop_and_escape_ends_the_session() {
    let mut doc = Doc::new("x = ;\n");
    let mut engine = VimEngine::new();
    frame(&mut doc, &mut engine, vec![egui::Event::Text("i".into())]);
    let _ = engine.place_cursor(&mut doc, 0, 4);
    with_snippet(&mut doc, &mut engine, 4, "max(${1:a}, ${2:b})$0");
    let none = egui::Modifiers::NONE;
    let typed = vec![egui::Event::Text("1".into()), key(egui::Key::Tab, none)];
    frame(&mut doc, &mut engine, typed);
    assert_eq!(doc.text(), "x = max(1, b);\n");
    assert_eq!(engine.cursor().col, 12, "after `b`");
    frame(&mut doc, &mut engine, vec![key(egui::Key::Backspace, none)]);
    assert_eq!(doc.text(), "x = max(1, );\n", "the whole stop goes");
    frame(&mut doc, &mut engine, vec![key(egui::Key::Escape, none)]);
    assert!(!engine.snippet_active());
    assert_eq!(engine.mode(), Mode::Normal);
}

#[test]
fn an_open_popup_keeps_tab_for_itself() {
    let mut doc = Doc::new("\n");
    let mut engine = VimEngine::new();
    engine.set_modeless(true);
    with_snippet(&mut doc, &mut engine, 0, "f(${1:a}, ${2:b})");
    let popup = SourceOptions {
        vim: false,
        popup_keys: true,
        ..SourceOptions::default()
    };
    let tab = vec![key(egui::Key::Tab, egui::Modifiers::NONE)];
    frame_with(&mut doc, &mut engine, &popup, tab);
    assert_eq!(engine.copy_text(&doc), "a");
    assert!(engine.snippet_active());
}

/// Cmd+Alt (macOS) and Ctrl+Alt (elsewhere, where Ctrl is Command).
fn command_alt() -> [egui::Modifiers; 2] {
    let alt = egui::Modifiers::ALT;
    [
        egui::Modifiers {
            mac_cmd: true,
            command: true,
            ..alt
        },
        egui::Modifiers {
            ctrl: true,
            command: true,
            ..alt
        },
    ]
}

#[test]
fn start_new_line_chords_follow_the_jetbrains_keymap() {
    let edit = |key, modifiers, mac| {
        ide_edit_key(IdeEditKeyInput {
            key,
            modifiers,
            mac,
            page_rows: 1,
        })
    };
    let below = Some(IdeEdit::StartNewLine { above: false });
    let above = Some(IdeEdit::StartNewLine { above: true });
    for mac in [true, false] {
        assert_eq!(edit(egui::Key::Enter, egui::Modifiers::SHIFT, mac), below);
        assert_eq!(edit(egui::Key::Enter, egui::Modifiers::NONE, mac), None);
    }
    let [cmd_alt, ctrl_alt] = command_alt();
    assert_eq!(edit(egui::Key::Enter, cmd_alt, true), above);
    assert_eq!(edit(egui::Key::Enter, ctrl_alt, false), above);
    // macOS Ctrl+Alt+Enter stays Alt+Enter (context actions).
    let mac_ctrl_alt = egui::Modifiers {
        ctrl: true,
        ..egui::Modifiers::ALT
    };
    assert_eq!(edit(egui::Key::Enter, mac_ctrl_alt, true), None);
    // Vim takes them in Insert mode only.
    for (mode, owned) in [(Mode::Insert, true), (Mode::Normal, false)] {
        for start in [below, above].into_iter().flatten() {
            assert_eq!(ide_edit_in_vim(start, egui::Modifiers::SHIFT, mode), owned);
        }
    }
}

#[test]
fn modeless_start_new_line_keys_open_lines_through_the_view() {
    let modeless = SourceOptions {
        vim: false,
        ..SourceOptions::default()
    };
    for chord in command_alt() {
        let mut doc = Doc::new("fn a() {\n    call(x);\n}\n");
        let mut engine = VimEngine::new();
        let _ = engine.place_cursor(&mut doc, 1, 9);
        let shift_enter = key(egui::Key::Enter, egui::Modifiers::SHIFT);
        frame_with(&mut doc, &mut engine, &modeless, vec![shift_enter]);
        assert_eq!(doc.text(), "fn a() {\n    call(x);\n    \n}\n");
        assert_eq!(engine.cursor(), Cursor { line: 2, col: 4 });
        let above = key(egui::Key::Enter, chord);
        frame_with(&mut doc, &mut engine, &modeless, vec![above]);
        assert_eq!(doc.text(), "fn a() {\n    call(x);\n    \n    \n}\n");
        assert_eq!(engine.cursor(), Cursor { line: 2, col: 4 });
    }
}

#[test]
fn vim_shift_enter_opens_a_line_in_insert_and_moves_down_in_normal() {
    let mut doc = Doc::new("  ab\ncd\n");
    let mut engine = VimEngine::new();
    let shift_enter = || vec![key(egui::Key::Enter, egui::Modifiers::SHIFT)];
    frame(&mut doc, &mut engine, shift_enter());
    assert_eq!(doc.text(), "  ab\ncd\n");
    assert_eq!(engine.cursor(), Cursor { line: 1, col: 0 });
    for text in ["k", "i"] {
        frame(&mut doc, &mut engine, vec![egui::Event::Text(text.into())]);
    }
    frame(&mut doc, &mut engine, shift_enter());
    assert_eq!(doc.text(), "  ab\n  \ncd\n");
    assert_eq!(engine.mode(), Mode::Insert);
    let [cmd_alt, _] = command_alt();
    frame(&mut doc, &mut engine, vec![key(egui::Key::Enter, cmd_alt)]);
    assert_eq!(doc.text(), "  ab\n  \n  \ncd\n");
    assert_eq!(engine.cursor(), Cursor { line: 1, col: 2 });
}

#[test]
fn command_z_puts_the_caret_back_on_the_undone_typing() {
    let modeless = SourceOptions {
        vim: false,
        ..SourceOptions::default()
    };
    let mut doc = Doc::new("one\ntwo\nthree\n");
    let mut engine = VimEngine::new();
    engine.set_modeless(true);
    let _ = engine.place_cursor(&mut doc, 1, 3);
    let typed = vec![egui::Event::Text("!!".into())];
    frame_with(&mut doc, &mut engine, &modeless, typed);
    assert_eq!(doc.text(), "one\ntwo!!\nthree\n");
    let down = key(egui::Key::ArrowDown, egui::Modifiers::NONE);
    frame_with(&mut doc, &mut engine, &modeless, vec![down]);
    let undo = key(egui::Key::Z, egui::Modifiers::COMMAND);
    frame_with(&mut doc, &mut engine, &modeless, vec![undo]);
    assert_eq!(doc.text(), "one\ntwo\nthree\n");
    assert_eq!(engine.cursor(), Cursor { line: 1, col: 3 });
}

/// The effects the widget reports for one press of `chord` (`vim` on or
/// off), after a first frame that gives it focus.
fn chord_effects(vim: bool, chord: (egui::Key, egui::Modifiers)) -> Vec<Effect> {
    let mut doc = Doc::new("needle\n");
    let mut engine = VimEngine::new();
    let options = SourceOptions {
        vim,
        ..SourceOptions::default()
    };
    let ctx = egui::Context::default();
    let mut effects = Vec::new();
    for events in [Vec::new(), vec![key(chord.0, chord.1)]] {
        let mut output = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    effects = show_source(ShowSourceInput {
                        ui,
                        doc: &mut doc,
                        engine: &mut engine,
                        options: &options,
                        highlight: &mut HighlightCache::default(),
                        language: Language::Plain,
                        id: "routing",
                    })
                    .effects;
                });
            },
        );
        output.textures_delta.clear();
    }
    effects
}

#[test]
fn cmd_shift_r_asks_for_replace_in_files_and_cmd_r_keeps_the_replace_bar() {
    let command = egui::Modifiers::COMMAND;
    let shifted = command | egui::Modifiers::SHIFT;
    for vim in [false, true] {
        assert_eq!(
            chord_effects(vim, (egui::Key::R, shifted)),
            [Effect::ReplaceInFiles],
            "vim {vim}"
        );
    }
    assert_eq!(
        chord_effects(false, (egui::Key::R, command)),
        [Effect::FindBar { replace: true }]
    );
}
