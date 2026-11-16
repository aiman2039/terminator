use super::*;

struct EditInput<'a> {
    line: u32,
    from: u32,
    to: u32,
    text: &'a str,
}

fn edit(input: EditInput<'_>) -> native_lsp::RenameTextEdit {
    let EditInput {
        line,
        from,
        to,
        text,
    } = input;
    native_lsp::RenameTextEdit {
        range: native_lsp::Range {
            start: native_lsp::Position {
                line,
                character: from,
            },
            end: native_lsp::Position {
                line,
                character: to,
            },
        },
        new_text: text.into(),
    }
}

#[test]
fn text_edits_apply_back_to_front_as_one_undo_step() {
    let mut buffer = Doc::new("fn a(){\nx\n}\n");
    let mut engine = VimEngine::new();
    assert!(apply_text_edits(
        &mut buffer,
        &[
            edit(EditInput {
                line: 0,
                from: 6,
                to: 6,
                text: " "
            }),
            edit(EditInput {
                line: 1,
                from: 0,
                to: 0,
                text: "    "
            })
        ],
    ));
    assert_eq!(buffer.text(), "fn a() {\n    x\n}\n");
    engine.undo(&mut buffer);
    assert_eq!(buffer.text(), "fn a(){\nx\n}\n");
    assert!(!apply_text_edits(&mut buffer, &[]));
}

#[test]
fn on_type_edits_keep_the_caret_on_the_typed_text() {
    let mut buffer = Doc::new("        }\n");
    let mut engine = VimEngine::new();
    let _ = engine.place_cursor(&mut buffer, 0, 8);
    apply_edits_keep_caret(
        &mut buffer,
        &mut engine,
        &[edit(EditInput {
            line: 0,
            from: 0,
            to: 8,
            text: "    ",
        })],
    );
    assert_eq!(buffer.text(), "    }\n");
    assert_eq!(engine.cursor().col, 4);
}

#[test]
fn inserts_at_one_position_keep_their_array_order() {
    let mut buffer = Doc::new("x");
    let at = |text| {
        edit(EditInput {
            line: 0,
            from: 0,
            to: 0,
            text,
        })
    };
    assert!(apply_text_edits(&mut buffer, &[at("A"), at("B")]));
    assert_eq!(buffer.text(), "ABx");
}

fn rename_a_to_b(path: &Path) -> native_lsp::RenameFileEdit {
    native_lsp::RenameFileEdit {
        uri: native_lsp::file_uri(path).unwrap().to_string(),
        version: None,
        edits: vec![edit(EditInput {
            line: 0,
            from: 4,
            to: 5,
            text: "b",
        })],
    }
}

#[test]
fn a_refused_disk_part_leaves_open_buffers_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    let open = dir.path().join("a.rs");
    let closed = dir.path().join("b.rs");
    std::fs::write(&closed, "let a = 1;\n").unwrap();
    let mut native = NativeDoc::new(open.clone(), false);
    native.doc = Some(Doc::new("let a = 1;\n"));
    app.native_docs.insert(open.clone(), native);
    // No project: the disk part is refused before any worker runs.
    app.apply_workspace_edit(vec![rename_a_to_b(&open), rename_a_to_b(&closed)]);
    assert_eq!(
        app.native_docs[&open].buffer().unwrap().text(),
        "let a = 1;\n"
    );
    // A worker refusal also leaves the open buffer as it was.
    let revision = app.native_docs[&open].buffer().unwrap().revision();
    let pending = |app: &mut App| {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        app.action_apply = Some(ActionApply {
            rx,
            open: vec![OpenEdit {
                path: open.clone(),
                revision,
                edits: rename_a_to_b(&open).edits,
            }],
        });
        tx
    };
    let tx = pending(&mut app);
    tx.send(Err("refused".into())).unwrap();
    app.drain_edit_answers();
    assert!(app.action_apply.is_none());
    assert_eq!(
        app.native_docs[&open].buffer().unwrap().text(),
        "let a = 1;\n"
    );
    // Success applies the open part.
    let tx = pending(&mut app);
    tx.send(Ok(vec![closed.clone()])).unwrap();
    app.drain_edit_answers();
    assert_eq!(
        app.native_docs[&open].buffer().unwrap().text(),
        "let b = 1;\n"
    );
}

#[test]
fn a_code_action_picked_after_an_edit_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    let path = dir.path().join("a.rs");
    let mut native = NativeDoc::new(path.clone(), false);
    native.doc = Some(Doc::new("let a = 1;\n"));
    let listed = native.doc.as_ref().map_or(0, Doc::revision);
    native.doc.as_mut().unwrap().insert(0, "// typed\n");
    app.native_docs.insert(path.clone(), native);
    let uri = native_lsp::file_uri(&path).unwrap().to_string();
    let item = native_lsp::CodeActionItem {
        title: "Rename".into(),
        kind: None,
        preferred: false,
        spell: false,
        raw: serde_json::json!({"edit": {"changes": {uri: [{
            "range": {"start": {"line": 0, "character": 4},
                      "end": {"line": 0, "character": 5}},
            "newText": "b"
        }]}}}),
    };
    app.run_code_action(&path, &item, listed);
    let text = app.native_docs[&path].buffer().unwrap().text();
    assert_eq!(text, "// typed\nlet a = 1;\n");
    // At the listed revision the same action applies.
    let current = app.native_docs[&path].buffer().unwrap().revision();
    app.run_code_action(&path, &item, current);
    let text = app.native_docs[&path].buffer().unwrap().text();
    assert_eq!(text, "// tbped\nlet a = 1;\n");
}
