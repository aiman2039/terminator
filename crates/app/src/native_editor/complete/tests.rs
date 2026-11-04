use super::signature::open_paren;
use super::*;

fn item(label: &str, sort: &str) -> native_lsp::CompletionItem {
    native_lsp::CompletionItem {
        label: label.into(),
        kind: Some("Function".into()),
        detail: None,
        insert: label.into(),
        snippet: false,
        filter: label.into(),
        sort: sort.into(),
        edit: None,
        replace: None,
        additional: Vec::new(),
    }
}

#[test]
fn signature_opens_inside_calls_and_closes_outside() {
    assert_eq!(open_paren("f(a, g(b), ", 11), Some(2));
    assert_eq!(open_paren("f(a)", 4), None);
    let mut buffer = Doc::new("f(\n");
    let mut engine = VimEngine::new();
    engine.set_modeless(true);
    let _ = engine.place_cursor(&mut buffer, 0, 2);
    engine.modeless_key(&mut buffer, terminator_native_edit::vim::Key::Char('x'));
    let before = buffer.revision();
    engine.type_text(&mut buffer, ",");
    let mut popup = None;
    let asked = signature_after_frame(SignatureAfterFrameInput {
        popup: &mut popup,
        buffer: &buffer,
        engine: &engine,
        revision_before: before,
        server: true,
    });
    assert_eq!(asked.map(|(_, line, open)| (line, open)), Some((0, 2)));
    popup = Some(SignaturePopup {
        path: PathBuf::from("/f.rs"),
        generation: 1,
        help: None,
        line: 0,
        open: 2,
        anchor: None,
    });
    // Leaving the call (past its `)`) closes the popup.
    let _ = engine.place_cursor(&mut buffer, 0, 0);
    let rev = buffer.revision();
    assert!(
        signature_after_frame(SignatureAfterFrameInput {
            popup: &mut popup,
            buffer: &buffer,
            engine: &engine,
            revision_before: rev,
            server: true
        })
        .is_none()
    );
    assert!(popup.is_none());
}

#[test]
fn prefix_and_trigger_rules() {
    assert_eq!(prefix_start("let foo_ba", 10), 4);
    assert!(triggers("x.", 2));
    assert!(triggers("std::", 5));
    assert!(!triggers("a :", 3));
    assert!(!triggers("a ", 2));
}

#[test]
fn ranking_prefers_prefix_matches_then_server_order() {
    let items = [
        item("to_string", "2"),
        item("Tokio", "1"),
        item("into_iter", "0"),
        item("as_ref", "0"),
    ];
    // `to`: exact-case prefix, then any-case prefix, then subsequence.
    assert_eq!(rank(&items, "to"), [0, 1, 2]);
}

#[test]
fn accepting_replaces_the_prefix_and_lands_on_the_snippet_stop() {
    let mut buffer = Doc::new("let v = pri\n");
    let mut engine = VimEngine::new();
    engine.set_modeless(true);
    let _ = engine.place_cursor(&mut buffer, 0, 11);
    let mut chosen = item("println!", "0");
    chosen.insert = "println!(\"$1\")$0".into();
    chosen.snippet = true;
    accept(AcceptInput {
        buffer: &mut buffer,
        engine: &mut engine,
        line: 0,
        start: 8,
        item: &chosen,
    });
    assert_eq!(buffer.text(), "let v = println!(\"\")\n");
    assert_eq!(engine.cursor().col, 18);
    engine.undo(&mut buffer);
    assert_eq!(buffer.text(), "let v = pri\n");
}

#[test]
fn completion_badges_match_the_structure_view() {
    use super::popup::kind_badge;
    assert_eq!(kind_badge(Some("Struct")).map(|(c, _)| c), Some('S'));
    assert_eq!(kind_badge(Some("Method")).map(|(c, _)| c), Some('m'));
    assert_eq!(kind_badge(Some("Field")).map(|(c, _)| c), Some('p'));
    // Keywords show no badge, as in `IntelliJ`.
    assert!(kind_badge(Some("Keyword")).is_none());
    assert!(kind_badge(None).is_none());
    // Functions and fields read as different groups.
    assert_ne!(
        kind_badge(Some("Function")).map(|(_, tint)| tint),
        kind_badge(Some("Field")).map(|(_, tint)| tint)
    );
}

#[test]
fn accepting_applies_auto_imports_and_the_replace_range() {
    let position = |line, character| native_lsp::Position { line, character };
    let mut buffer = Doc::new("fn main() {\n    HashSe\n}\n");
    let mut engine = VimEngine::new();
    engine.set_modeless(true);
    let _ = engine.place_cursor(&mut buffer, 1, 8);
    let mut chosen = item("HashSet", "0");
    // Caret inside the word: the replace range covers its tail too.
    chosen.replace = Some(native_lsp::Range {
        start: position(1, 4),
        end: position(1, 10),
    });
    chosen.additional = vec![native_lsp::RenameTextEdit {
        range: native_lsp::Range {
            start: position(0, 0),
            end: position(0, 0),
        },
        new_text: "use std::collections::HashSet;\n".into(),
    }];
    accept(AcceptInput {
        buffer: &mut buffer,
        engine: &mut engine,
        line: 1,
        start: 4,
        item: &chosen,
    });
    assert_eq!(
        buffer.text(),
        "use std::collections::HashSet;\nfn main() {\n    HashSet\n}\n"
    );
    assert_eq!(
        (engine.cursor().line, engine.cursor().col),
        (2, 11),
        "the caret follows the inserted item"
    );
    engine.undo(&mut buffer);
    assert_eq!(buffer.text(), "fn main() {\n    HashSe\n}\n");
}
