//! The app side of completion: answers that keep or close the popup,
//! auto-import edits that arrive after an accept, and the docs panel.
use super::*;

/// An app (isolated state under `dir`) with `text` loaded at `path` and
/// the caret at `(line, col)`.
fn app_with(dir: &Path, (path, text): (&Path, &str), (line, col): (usize, usize)) -> App {
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, terminator_core::Paths::at(dir.join("state")));
    let mut doc = NativeDoc::loaded(path.to_owned(), text);
    if let Some(buffer) = doc.doc.as_mut() {
        doc.engine.place_cursor(buffer, line, col);
    }
    app.native_docs.insert(path.to_owned(), doc);
    app
}

fn item(label: &str) -> native_lsp::CompletionItem {
    native_lsp::CompletionItem {
        label: label.into(),
        insert: label.into(),
        filter: label.into(),
        sort: label.into(),
        ..Default::default()
    }
}

fn results(
    path: &Path,
    generation: u64,
    items: Vec<native_lsp::CompletionItem>,
) -> CompletionResults {
    CompletionResults {
        path: path.to_owned(),
        generation,
        items,
        incomplete: true,
    }
}

fn text_of(app: &App, path: &Path) -> String {
    app.native_docs[path]
        .doc
        .as_ref()
        .map(Doc::text)
        .unwrap_or_default()
}

#[test]
fn late_auto_imports_land_as_their_own_undo_step_while_they_fit() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("main.rs");
    let text = "fn main() {\n    let s = HashSet\n}\n";
    let mut app = app_with(dir.path(), (&path, text), (1, 19));
    let start = app.native_docs[&path].engine.cursor();
    let buffer = app.native_docs[&path].doc.clone().expect("buffer");
    // Accepted at column 12 of line 1; the server answers later.
    let ResolveFor::Accepted { line, col, before } = complete::accepted_at(&buffer, (1, 12)) else {
        panic!("an accept target");
    };
    let mut set = item("HashSet");
    set.additional = vec![native_lsp::RenameTextEdit {
        range: native_lsp::Range {
            start: native_lsp::Position {
                line: 0,
                character: 0,
            },
            end: native_lsp::Position {
                line: 0,
                character: 0,
            },
        },
        new_text: "use std::collections::HashSet;\n".into(),
    }];
    app.native_completion_imports(&path, (line, col, before), set.clone());
    assert_eq!(
        text_of(&app, &path),
        "use std::collections::HashSet;\nfn main() {\n    let s = HashSet\n}\n"
    );
    let doc = app.native_docs.get_mut(&path).expect("doc");
    let caret = doc.engine.cursor();
    assert_eq!(
        (caret.line, caret.col),
        (start.line.saturating_add(1), start.col),
        "the caret follows its text"
    );
    let buffer = doc.doc.as_mut().expect("buffer");
    doc.engine.undo(buffer);
    assert_eq!(text_of(&app, &path), text, "one undo step");
    // Once the text before the item changed, the edits no longer fit.
    let doc = app.native_docs.get_mut(&path).expect("doc");
    doc.doc.as_mut().expect("buffer").insert(0, "// note\n");
    app.native_completion_imports(&path, (line, col, before), set);
    assert!(!text_of(&app, &path).contains("use std"));
    let note = app.info.clone().unwrap_or_default();
    assert!(note.contains("HashSet was not added"), "{note}");
}

#[test]
fn a_late_import_keeps_the_selected_snippet_stop_on_its_text() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("main.rs");
    let mut app = app_with(dir.path(), (&path, "fn main() {\n    ma\n}\n"), (1, 6));
    let mut call = item("max");
    call.insert = "max(${1:a}, ${2:b})$0".into();
    call.snippet = true;
    let doc = app.native_docs.get_mut(&path).expect("doc");
    doc.engine.set_modeless(true);
    let buffer = doc.doc.as_mut().expect("buffer");
    // Modeless: the caret may sit after the last char.
    let _ = doc.engine.place_cursor(buffer, 1, 6);
    complete::accept(complete::AcceptInput {
        buffer,
        engine: &mut doc.engine,
        path: &path,
        line: 1,
        start: 4,
        item: &call,
    });
    let ResolveFor::Accepted { line, col, before } = complete::accepted_at(buffer, (1, 4)) else {
        panic!("an accept target");
    };
    let top = native_lsp::Position {
        line: 0,
        character: 0,
    };
    call.additional = vec![native_lsp::RenameTextEdit {
        range: native_lsp::Range {
            start: top,
            end: top,
        },
        new_text: "use std::cmp::max;\n".into(),
    }];
    app.native_completion_imports(&path, (line, col, before), call);
    let doc = app.native_docs.get_mut(&path).expect("doc");
    let buffer = doc.doc.as_mut().expect("buffer");
    assert_eq!(
        buffer.text(),
        "use std::cmp::max;\nfn main() {\n    max(a, b)\n}\n"
    );
    assert_eq!(doc.engine.copy_text(buffer), "a", "still selected");
    assert!(doc.engine.snippet_jump(buffer, true));
    assert_eq!(doc.engine.copy_text(buffer), "b");
    let caret = doc.engine.cursor();
    assert_eq!((caret.line, caret.col), (2, 12));
}

#[test]
fn a_late_import_moves_the_open_popup_with_its_line() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("main.rs");
    let text = "fn main() {\n    let m = HashMap::\n}\n";
    let mut app = app_with(dir.path(), (&path, text), (1, 21));
    let doc = app.native_docs.get_mut(&path).expect("doc");
    doc.engine.set_modeless(true);
    let buffer = doc.doc.as_mut().expect("buffer");
    let _ = doc.engine.place_cursor(buffer, 1, 21);
    let buffer = app.native_docs[&path].doc.clone().expect("buffer");
    let ResolveFor::Accepted { line, col, before } = complete::accepted_at(&buffer, (1, 12)) else {
        panic!("an accept target");
    };
    // `::` opened a member list after the accepted item.
    app.completion = Some(CompletionPopup::new(&path, 5, (1, 21), None));
    let top = native_lsp::Position {
        line: 0,
        character: 0,
    };
    let mut map = item("HashMap");
    map.additional = vec![native_lsp::RenameTextEdit {
        range: native_lsp::Range {
            start: top,
            end: top,
        },
        new_text: "use std::collections::HashMap;\n".into(),
    }];
    app.native_completion_imports(&path, (line, col, before), map);
    let caret = app.native_docs[&path].engine.cursor();
    assert_eq!((caret.line, caret.col), (2, 21));
    let popup = app.completion.as_ref().expect("still open");
    assert_eq!((popup.line, popup.start), (2, 21), "on the caret's line");
}

#[test]
fn an_empty_complete_answer_closes_the_popup_so_typing_asks_again() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("main.rs");
    let mut app = app_with(dir.path(), (&path, "x\n"), (0, 1));
    app.completion = Some(CompletionPopup::new(&path, 3, (0, 0), None));
    let mut empty = results(&path, 3, Vec::new());
    empty.incomplete = false;
    app.native_completion_answered(empty);
    assert!(app.completion.is_none());
    // An incomplete answer stays open, even empty: typing asks again.
    app.completion = Some(CompletionPopup::new(&path, 4, (0, 0), None));
    app.native_completion_answered(results(&path, 4, Vec::new()));
    assert!(
        app.completion
            .as_ref()
            .is_some_and(|popup| popup.incomplete)
    );
    // An answer to an older request changes nothing.
    app.native_completion_answered(results(&path, 2, vec![item("stale")]));
    assert!(
        app.completion
            .as_ref()
            .is_some_and(|popup| popup.items.is_empty())
    );
}

#[test]
fn an_incomplete_list_stays_on_screen_while_it_is_asked_again() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("main.rs");
    let mut app = app_with(dir.path(), (&path, "le\n"), (0, 2));
    let mut popup = CompletionPopup::new(&path, 1, (0, 0), None);
    popup.adopt(results(&path, 1, vec![item("len")]));
    app.completion = Some(popup);
    let ask = complete::CompletionAsk {
        position: native_lsp::Position {
            line: 0,
            character: 2,
        },
        line: 0,
        start: 0,
        trigger: native_lsp::CompletionTrigger::Incomplete,
    };
    app.native_completion_asked(&path, (ask, 2), None);
    let shown = app.completion.as_ref().expect("open");
    assert_eq!((shown.generation, shown.list), (Some(2), 1));
    assert_eq!(
        shown.items.len(),
        1,
        "the old list stays until the new one lands"
    );
    // Any other request starts a fresh popup.
    let typed = complete::CompletionAsk {
        trigger: native_lsp::CompletionTrigger::Character('.'),
        ..ask
    };
    app.native_completion_asked(&path, (typed, 3), None);
    assert!(
        app.completion
            .as_ref()
            .is_some_and(|popup| popup.items.is_empty())
    );
}

#[test]
fn the_docs_panel_draws_the_selected_docs_beside_the_list() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("main.rs");
    let mut app = app_with(dir.path(), (&path, "le\n"), (0, 2));
    let mut len = item("len");
    len.detail = Some("fn(&self) -> usize".into());
    len.documentation = Some(native_lsp::Hover {
        text: "Returns the **length**.\n\n```rust\nlet n = v.len();\n```".into(),
        markdown: true,
    });
    let mut popup = CompletionPopup::new(&path, 1, (0, 0), None);
    popup.adopt(results(&path, 1, vec![len]));
    let list = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(520.0, 240.0));
    popup.rect = Some(list);
    app.completion = Some(popup);
    let ctx = egui::Context::default();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1400.0, 900.0),
        )),
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| app.native_completion_docs(ui, &path));
    output.textures_delta.clear();
    let panel = ctx
        .memory(|memory| memory.area_rect(egui::Id::unique(("native-completion-docs", &path))))
        .expect("the docs panel");
    assert!(panel.left() > list.right(), "{panel:?} right of {list:?}");
    assert!((panel.top() - list.top()).abs() < 1.0);
}
