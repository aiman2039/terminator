use super::*;

fn body_of(framed: &[u8]) -> Value {
    let mut decoder = FrameDecoder::default();
    let bodies = decoder.feed(framed);
    assert_eq!(bodies.len(), 1);
    let body = bodies.into_iter().next().unwrap_or_default();
    serde_json::from_slice(&body).expect("valid json")
}

static NULL: Value = Value::Null;

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or(&NULL)
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    field(value, key).as_str().unwrap_or("")
}

fn number(value: &Value, key: &str) -> i64 {
    field(value, key).as_i64().unwrap_or(-1)
}

#[test]
fn framing_round_trips_one_byte_chunks() {
    let body = br#"{"jsonrpc":"2.0","method":"exit","params":{}}"#;
    let framed = frame(body);
    let mut decoder = FrameDecoder::default();
    let mut bodies = Vec::new();
    for chunk in framed.chunks(1) {
        bodies.extend(decoder.feed(chunk));
    }
    assert_eq!(bodies, vec![body.to_vec()]);
}

#[test]
fn decoder_waits_for_full_body() {
    let framed = frame(b"{}");
    let mut decoder = FrameDecoder::default();
    let split = framed.len().saturating_sub(1);
    let (head, tail) = framed.split_at(split);
    assert!(decoder.feed(head).is_empty());
    assert_eq!(decoder.feed(tail).len(), 1);
}

#[test]
fn decoder_drains_concatenated_frames() {
    let first = frame(br#"{"jsonrpc":"2.0","id":0,"result":{}}"#);
    let second = frame(br#"{"jsonrpc":"2.0","method":"m","params":{}}"#);
    let mut stream = first.clone();
    stream.extend_from_slice(&second);
    // One feed with both frames glued together …
    let mut decoder = FrameDecoder::default();
    let bodies = decoder.feed(&stream);
    assert_eq!(bodies.len(), 2);
    // … and split at every possible byte offset.
    for at in 0..stream.len() {
        let mut decoder = FrameDecoder::default();
        let (head, tail) = stream.split_at(at);
        let mut bodies = decoder.feed(head);
        bodies.extend(decoder.feed(tail));
        assert_eq!(bodies.len(), 2, "split at {at}");
    }
}

#[test]
fn decoder_skips_header_without_length() {
    let mut stream = b"X-Noise: yes\r\n\r\n".to_vec();
    stream.extend_from_slice(&frame(b"{}"));
    let mut decoder = FrameDecoder::default();
    assert_eq!(decoder.feed(&stream), vec![b"{}".to_vec()]);
}

#[test]
fn classifies_request_notification_response() {
    let request = parse_message(br#"{"jsonrpc":"2.0","id":1,"method":"m","params":{"a":1}}"#);
    assert!(matches!(
        request,
        Some(LspMessage::Request { method, .. }) if method == "m"
    ));
    let notification = parse_message(br#"{"jsonrpc":"2.0","method":"m","params":null}"#);
    assert!(matches!(
        notification,
        Some(LspMessage::Notification { method, .. }) if method == "m"
    ));
    let response = parse_message(br#"{"jsonrpc":"2.0","id":"x","result":1,"error":null}"#);
    assert!(matches!(
        response,
        Some(LspMessage::Response {
            id: RequestId::Text(_),
            ..
        })
    ));
    assert!(parse_message(b"{}").is_none());
    assert!(parse_message(b"not json").is_none());
}

#[test]
fn error_response_carries_code_and_message() {
    let message = parse_message(
        br#"{"jsonrpc":"2.0","id":2,"result":null,
            "error":{"code":-32601,"message":"gone"}}"#,
    );
    match message {
        Some(LspMessage::Response {
            error: Some(error), ..
        }) => {
            assert_eq!(error.code, -32601);
            assert_eq!(error.message, "gone");
        }
        other => panic!("expected error response, got {other:?}"),
    }
}

#[test]
fn initialize_carries_root_and_utf16() {
    let root = Url::parse("file:///repo").expect("root");
    let mut ids = IdGen::default();
    let value = body_of(&initialize(&ids.next_id(), 7, &root));
    assert_eq!(text(&value, "method"), "initialize");
    let params = field(&value, "params");
    assert_eq!(text(params, "rootUri"), "file:///repo");
    assert_eq!(number(params, "processId"), 7);
    let general = field(field(params, "capabilities"), "general");
    let encodings = field(general, "positionEncodings");
    let first = encodings.get(0).and_then(Value::as_str).unwrap_or("");
    assert_eq!(first, "utf-16");
}

#[test]
fn sync_notifications_carry_uri_version_text() {
    let uri = Url::parse("file:///repo/main.rs").expect("uri");
    let opened = body_of(&did_open(&uri, "rust", 3, "fn f() {}"));
    assert_eq!(text(&opened, "method"), "textDocument/didOpen");
    let document = field(field(&opened, "params"), "textDocument");
    assert_eq!(number(document, "version"), 3);
    assert_eq!(text(document, "text"), "fn f() {}");

    let changed = body_of(&did_change(&uri, 4, "fn g() {}"));
    assert_eq!(text(&changed, "method"), "textDocument/didChange");
    let changes = field(field(&changed, "params"), "contentChanges");
    let first = changes.get(0).unwrap_or(&NULL);
    assert_eq!(text(first, "text"), "fn g() {}");

    let closed = body_of(&did_close(&uri));
    assert_eq!(text(&closed, "method"), "textDocument/didClose");
}

#[test]
fn hover_and_definition_carry_utf16_position() {
    let uri = Url::parse("file:///repo/main.rs").expect("uri");
    let mut ids = IdGen::default();
    let at = Position {
        line: 4,
        character: 9,
    };
    let asking = body_of(&hover(&ids.next_id(), &uri, at));
    assert_eq!(text(&asking, "method"), "textDocument/hover");
    let position = field(field(&asking, "params"), "position");
    assert_eq!(number(position, "line"), 4);
    assert_eq!(number(position, "character"), 9);
    let jumping = body_of(&definition(&ids.next_id(), &uri, at));
    assert_eq!(text(&jumping, "method"), "textDocument/definition");
    assert_eq!(number(&jumping, "id"), 1);
}

#[test]
fn references_request_and_locations_keep_utf16_positions() {
    let uri = Url::parse("file:///repo/main.rs").expect("uri");
    let at = Position {
        line: 4,
        character: 9,
    };
    let request = body_of(&references(&RequestId::Number(7), &uri, at));
    assert_eq!(text(&request, "method"), "textDocument/references");
    let params = field(&request, "params");
    assert_eq!(number(field(params, "position"), "character"), 9);
    assert_eq!(
        field(field(params, "context"), "includeDeclaration"),
        &Value::Bool(true)
    );

    let locations = serde_json::json!([{
        "uri": "file:///repo/lib.rs",
        "range": {
            "start": { "line": 2, "character": 4 },
            "end": { "line": 2, "character": 8 }
        }
    }]);
    let parsed = parse_references(&locations);
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].uri, "file:///repo/lib.rs");
    assert_eq!(
        parsed[0].range.start,
        Position {
            line: 2,
            character: 4
        }
    );
    assert!(parse_references(&Value::Null).is_empty());
}

#[test]
fn rename_parses_multifile_text_edits_and_rejects_file_operations() {
    let uri = Url::parse("file:///repo/main.rs").unwrap();
    let request = body_of(&rename(
        &RequestId::Number(4),
        &uri,
        Position {
            line: 1,
            character: 5,
        },
        "new_name",
    ));
    assert_eq!(text(&request, "method"), "textDocument/rename");
    assert_eq!(text(field(&request, "params"), "newName"), "new_name");

    let edit = serde_json::json!({
        "documentChanges": [{
            "textDocument": { "uri": "file:///repo/main.rs", "version": 3 },
            "edits": [{
                "range": {
                    "start": { "line": 1, "character": 5 },
                    "end": { "line": 1, "character": 9 }
                },
                "newText": "new_name"
            }]
        }]
    });
    let files = parse_rename(&edit).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].version, Some(3));
    assert_eq!(files[0].edits[0].new_text, "new_name");

    let operation = serde_json::json!({
        "documentChanges": [{
            "kind": "rename",
            "oldUri": "file:///repo/a.rs",
            "newUri": "file:///repo/b.rs"
        }]
    });
    assert!(parse_rename(&operation).is_err());
}

#[test]
fn workspace_symbol_request_and_responses_keep_file_targets() {
    let request = body_of(&workspace_symbol(&RequestId::Number(9), "helper"));
    assert_eq!(text(&request, "method"), "workspace/symbol");
    assert_eq!(text(field(&request, "params"), "query"), "helper");
    let flat = serde_json::json!([{
        "name": "helper",
        "kind": 12,
        "containerName": "module",
        "location": {
            "uri": "file:///repo/src/lib.rs",
            "range": {
                "start": { "line": 3, "character": 4 },
                "end": { "line": 3, "character": 10 }
            }
        }
    }]);
    let hits = parse_workspace_symbols(&flat);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].name, "helper");
    assert_eq!(hits[0].container.as_deref(), Some("module"));
    assert_eq!(hits[0].range.start.line, 3);
    assert_eq!(hits[0].uri, "file:///repo/src/lib.rs");

    let unresolved = serde_json::json!([{
        "name": "unresolved",
        "kind": 12,
        "location": { "uri": "file:///repo/src/lib.rs" }
    }]);
    assert!(parse_workspace_symbols(&unresolved).is_empty());
}

#[test]
fn publishes_diagnostics_with_severity() {
    let params = serde_json::json!({
        "uri": "file:///repo/main.rs",
        "diagnostics": [
            {
                "range": {"start": {"line": 1, "character": 2},
                          "end": {"line": 1, "character": 9}},
                "severity": 1,
                "message": "cannot find value `x`",
            },
            {
                "range": {"start": {"line": 4, "character": 0},
                          "end": {"line": 4, "character": 1}},
                "message": "unused, no severity",
            },
        ],
    });
    let (uri, items) = parse_diagnostics(&params).expect("diagnostics");
    assert_eq!(uri, "file:///repo/main.rs");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].severity, Some(Severity::Error));
    assert_eq!(
        items[0].range.start,
        Position {
            line: 1,
            character: 2
        }
    );
    assert_eq!(items[1].severity, None);
}

#[test]
fn hover_shapes_flatten_to_text() {
    assert!(parse_hover(&Value::Null).is_none());
    let markup = serde_json::json!({
        "contents": {"kind": "markdown", "value": "# f\n```rust\nfn f()\n```"}
    });
    assert_eq!(
        parse_hover(&markup).expect("markup").text,
        "# f\n```rust\nfn f()\n```"
    );
    assert!(parse_hover(&markup).expect("markup").markdown);
    let plaintext = serde_json::json!({
        "contents": {"kind": "plaintext", "value": "# f"}
    });
    assert!(!parse_hover(&plaintext).expect("plaintext").markdown);
    let scalar = serde_json::json!({"contents": "plain hover"});
    assert_eq!(parse_hover(&scalar).expect("scalar").text, "plain hover");
    let array = serde_json::json!({
        "contents": [
            {"language": "rust", "value": "fn f()"},
            "docs here",
        ]
    });
    assert_eq!(
        parse_hover(&array).expect("array").text,
        "```rust\nfn f()\n```\n\ndocs here"
    );
}

#[test]
fn definition_shapes_land_on_targets() {
    assert!(parse_definition(&Value::Null).is_empty());
    let single = serde_json::json!({
        "uri": "file:///repo/a.rs",
        "range": {"start": {"line": 0, "character": 0},
                  "end": {"line": 0, "character": 5}},
    });
    let targets = parse_definition(&single);
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].uri, "file:///repo/a.rs");

    let link = serde_json::json!([{
        "originSelectionRange": {
            "start": {"line": 3, "character": 1},
            "end": {"line": 3, "character": 2}},
        "targetUri": "file:///repo/b.rs",
        "targetRange": {"start": {"line": 9, "character": 0},
                        "end": {"line": 9, "character": 4}},
        "targetSelectionRange": {
            "start": {"line": 9, "character": 0},
            "end": {"line": 9, "character": 4}},
    }]);
    let targets = parse_definition(&link);
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].uri, "file:///repo/b.rs");
    assert_eq!(targets[0].range.start.line, 9);
}

#[test]
fn document_symbols_flatten_nested_and_flat_shapes() {
    assert!(parse_document_symbols(&Value::Null).is_empty());
    let nested = serde_json::json!([{
        "name": "main",
        "kind": 12,
        "range": {"start": {"line": 0, "character": 0},
                  "end": {"line": 9, "character": 1}},
        "selectionRange": {"start": {"line": 0, "character": 3},
                           "end": {"line": 0, "character": 7}},
        "children": [{
            "name": "helper",
            "kind": 12,
            "range": {"start": {"line": 2, "character": 1},
                      "end": {"line": 4, "character": 2}},
            "selectionRange": {"start": {"line": 2, "character": 5},
                               "end": {"line": 2, "character": 11}},
        }],
    }]);
    let symbols = parse_document_symbols(&nested);
    assert_eq!(symbols.len(), 2);
    assert_eq!(symbols[0].name, "main");
    assert_eq!(symbols[0].depth, 0);
    assert_eq!(symbols[1].name, "helper");
    assert_eq!(symbols[1].depth, 1);
    assert_eq!(symbols[1].range.start.line, 2);
    // Jumps land on the name, not the item's first line.
    assert_eq!(symbols[1].selection.start.character, 5);

    let flat = serde_json::json!([{
        "name": "Thing",
        "kind": 5,
        "location": {
            "uri": "file:///repo/a.rs",
            "range": {"start": {"line": 3, "character": 0},
                      "end": {"line": 5, "character": 1}},
        },
    }]);
    let symbols = parse_document_symbols(&flat);
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].kind, "Class");
    assert_eq!(symbols[0].depth, 0);
}

#[test]
fn code_actions_keep_editable_ones_preferred_first() {
    let actions = serde_json::json!([
        { "title": "Run command", "command": { "title": "x", "command": "x" } },
        { "title": "Extract variable", "kind": "refactor.extract", "data": 1 },
        { "title": "Add missing import", "kind": "quickfix", "isPreferred": true,
          "edit": { "changes": { "file:///a.rs": [
              { "range": { "start": { "line": 0, "character": 0 },
                           "end": { "line": 0, "character": 0 } },
                "newText": "use x;\n" } ] } } },
    ]);
    let items = parse_code_actions(&actions);
    assert_eq!(
        items
            .iter()
            .map(|item| item.title.as_str())
            .collect::<Vec<_>>(),
        ["Add missing import", "Extract variable"]
    );
    let edits = code_action_edits(&items[0].raw).unwrap();
    assert_eq!(edits[0].edits[0].new_text, "use x;\n");
    assert!(code_action_edits(&items[1].raw).is_err());
    let formatted = parse_text_edits(&serde_json::json!([
        { "range": { "start": { "line": 1, "character": 0 },
                     "end": { "line": 1, "character": 2 } }, "newText": "    " }
    ]));
    assert_eq!(formatted.len(), 1);
    assert!(parse_text_edits(&Value::Null).is_empty());
}

#[test]
fn inlay_hints_parse_string_and_part_labels_in_position_order() {
    let request = body_of(&inlay_hint(
        &RequestId::Number(4),
        &Url::parse("file:///tmp/a.rs").unwrap(),
        12,
    ));
    assert_eq!(text(&request, "method"), "textDocument/inlayHint");
    assert_eq!(request["params"]["range"]["end"]["line"], 12);
    let result = serde_json::json!([
        {
            "position": { "line": 3, "character": 9 },
            "label": [{ "value": "a" }, { "value": ":" }],
            "paddingRight": true,
        },
        { "position": { "line": 1, "character": 5 }, "label": ": i32", "kind": 1 },
        { "position": { "line": 2, "character": 0 }, "label": "  " },
        { "position": { "line": 2 }, "label": "broken" },
    ]);
    assert_eq!(
        parse_inlay_hints(&result),
        [
            InlayHint {
                position: Position {
                    line: 1,
                    character: 5
                },
                label: ": i32".into(),
            },
            InlayHint {
                position: Position {
                    line: 3,
                    character: 9
                },
                label: "a: ".into(),
            },
        ]
    );
    assert!(parse_inlay_hints(&Value::Null).is_empty());
}

#[test]
fn hierarchy_requests_and_answers_round_trip_items() {
    let uri = Url::parse("file:///tmp/a.rs").unwrap();
    let at = Position {
        line: 3,
        character: 4,
    };
    let prepare = body_of(&prepare_hierarchy(
        &RequestId::Number(1),
        &uri,
        at,
        HierarchyKind::Call,
    ));
    assert_eq!(
        text(&prepare, "method"),
        "textDocument/prepareCallHierarchy"
    );
    let item = serde_json::json!({
        "name": "helper",
        "kind": 12,
        "detail": "fn helper()",
        "uri": "file:///tmp/a.rs",
        "range": { "start": { "line": 1, "character": 0 }, "end": { "line": 3, "character": 1 } },
        "selectionRange": { "start": { "line": 1, "character": 3 }, "end": { "line": 1, "character": 9 } },
        "data": 7,
    });
    let prepared = parse_hierarchy_items(&serde_json::json!([item.clone()]));
    assert_eq!(prepared.len(), 1);
    assert_eq!(prepared[0].name, "helper");
    assert_eq!(prepared[0].kind, "Function");
    assert_eq!(prepared[0].selection.start.character, 3);
    // Children send the server's own item back.
    for (kind, direction, method) in [
        (
            HierarchyKind::Call,
            HierarchyDirection::Up,
            "callHierarchy/incomingCalls",
        ),
        (
            HierarchyKind::Call,
            HierarchyDirection::Down,
            "callHierarchy/outgoingCalls",
        ),
        (
            HierarchyKind::Type,
            HierarchyDirection::Up,
            "typeHierarchy/supertypes",
        ),
        (
            HierarchyKind::Type,
            HierarchyDirection::Down,
            "typeHierarchy/subtypes",
        ),
    ] {
        let body = body_of(&hierarchy_children(
            &RequestId::Number(2),
            &prepared[0].raw,
            kind,
            direction,
        ));
        assert_eq!(text(&body, "method"), method);
        assert_eq!(body["params"]["item"]["data"], 7);
    }
    // Calls wrap items in `from` / `to`.
    let incoming = serde_json::json!([{ "from": item, "fromRanges": [] }]);
    assert_eq!(parse_hierarchy_items(&incoming)[0].name, "helper");
    assert!(parse_hierarchy_items(&Value::Null).is_empty());
    let caps = serde_json::json!({ "callHierarchyProvider": true, "typeHierarchyProvider": {} });
    assert!(supports_hierarchy(&caps, HierarchyKind::Call));
    assert!(supports_hierarchy(&caps, HierarchyKind::Type));
    assert!(!supports_hierarchy(
        &serde_json::json!({}),
        HierarchyKind::Call
    ));
}

#[test]
fn signature_help_marks_the_active_parameter() {
    let help = serde_json::json!({
        "signatures": [{
            "label": "fn push(&mut self, value: T)",
            "parameters": [{"label": "&mut self"}, {"label": [19, 27]}],
        }],
        "activeSignature": 0,
        "activeParameter": 1,
    });
    let parsed = parse_signature_help(&help).expect("signature");
    assert_eq!(parsed.label.get(19..27), Some("value: T"));
    assert_eq!(parsed.active, Some(19..27));
    let simple = serde_json::json!({
        "signatures": [{"label": "f(a, b)", "parameters": [{"label": "a"}, {"label": "b"}]}],
        "activeParameter": 1,
    });
    assert_eq!(parse_signature_help(&simple).unwrap().active, Some(5..6));
    assert!(parse_signature_help(&serde_json::json!({"signatures": []})).is_none());
}

#[test]
fn completion_lists_keep_edits_snippets_and_sort_keys() {
    let list = serde_json::json!({
        "isIncomplete": false,
        "items": [
            {
                "label": "push",
                "kind": 2,
                "detail": "fn(&mut self, T)",
                "insertTextFormat": 2,
                "sortText": "0001",
                "textEdit": {
                    "range": {"start": {"line": 3, "character": 6},
                              "end": {"line": 3, "character": 8}},
                    "newText": "push(${1:value})$0",
                },
            },
            { "label": "len" },
        ],
    });
    let items = parse_completion(&list);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].kind.as_deref(), Some("Method"));
    assert!(items[0].snippet);
    assert_eq!(items[0].insert, "push(${1:value})$0");
    assert_eq!(items[0].edit.map(|range| range.start.character), Some(6));
    assert_eq!(items[0].sort, "0001");
    // Bare items fall back to their label.
    assert_eq!(items[1].insert, "len");
    assert_eq!(items[1].filter, "len");
    assert!(parse_completion(&Value::Null).is_empty());
}

#[test]
fn snippets_expand_to_text_with_the_caret_on_the_first_stop() {
    assert_eq!(
        expand_snippet("push(${1:value})$0"),
        ("push(value)".into(), 5)
    );
    assert_eq!(expand_snippet("foo($0)"), ("foo()".into(), 4));
    assert_eq!(expand_snippet("plain"), ("plain".into(), 5));
    assert_eq!(expand_snippet("\\$x ${1|a,b|}"), ("$x a".into(), 4));
}

#[test]
fn document_symbols_keep_detail_and_deprecation() {
    let nested = serde_json::json!([{
        "name": "old",
        "detail": "fn() -> u8",
        "kind": 12,
        "tags": [1],
        "range": {"start": {"line": 0, "character": 0},
                  "end": {"line": 0, "character": 9}},
        "selectionRange": {"start": {"line": 0, "character": 3},
                           "end": {"line": 0, "character": 6}},
    }]);
    let symbols = parse_document_symbols(&nested);
    assert_eq!(symbols[0].detail.as_deref(), Some("fn() -> u8"));
    assert!(symbols[0].deprecated);
}

#[test]
fn markdown_outline_nests_headings_and_spans_sections() {
    let doc = crate::doc::Doc::new("# A\ntext\n## B\nmore\n### C\n## D\n# E\n");
    let outline = markdown_outline(&doc);
    let shape: Vec<(&str, usize, u32, u32)> = outline
        .iter()
        .map(|s| {
            (
                s.name.as_str(),
                s.depth,
                s.range.start.line,
                s.range.end.line,
            )
        })
        .collect();
    assert_eq!(
        shape,
        [
            ("A", 0, 0, 5),
            ("B", 1, 2, 4),
            ("C", 2, 4, 4),
            ("D", 1, 5, 5),
            ("E", 0, 6, 7),
        ]
    );
    // The caret in B's body is inside B, not A.
    assert_eq!(innermost_symbol(&outline, 3), Some(1));
    assert_eq!(innermost_symbol(&outline, 4), Some(2));
}

#[test]
fn server_table_covers_rust_python_and_typescript() {
    assert_eq!(server_candidates(Language::Rust), &["rust-analyzer"]);
    assert_eq!(server_candidates(Language::Python), &["pylsp"]);
    assert_eq!(
        server_candidates(Language::TypeScript),
        &["typescript-language-server"]
    );
    assert_eq!(
        server_candidates(Language::JavaScript),
        &["typescript-language-server"]
    );
    assert!(server_candidates(Language::Go).is_empty());
    assert_eq!(server_language(Language::Rust), Some("rust"));
    assert_eq!(server_language(Language::TypeScript), Some("typescript"));
    assert_eq!(server_language(Language::JavaScript), Some("typescript"));
    assert_eq!(server_language(Language::Markdown), None);
    assert_eq!(server_args(Language::TypeScript), &["--stdio"]);
    assert!(server_args(Language::Rust).is_empty());
}

#[test]
fn typescript_file_ids_follow_the_extension() {
    let id = |name: &str| language_id_for_path(Path::new(name), Language::TypeScript);
    assert_eq!(id("app.ts"), Some("typescript"));
    assert_eq!(id("view.tsx"), Some("typescriptreact"));
    assert_eq!(id("config.mts"), Some("typescript"));
    assert_eq!(
        language_id_for_path(Path::new("main.js"), Language::JavaScript),
        Some("javascript")
    );
    assert_eq!(
        language_id_for_path(Path::new("view.jsx"), Language::JavaScript),
        Some("javascriptreact")
    );
    assert_eq!(
        language_id_for_path(Path::new("notes.md"), Language::Markdown),
        None
    );
}

#[test]
fn path_probe_finds_fake_server() {
    let dir = tempfile::tempdir().expect("tempdir");
    let fake = dir
        .path()
        .join(format!("rust-analyzer{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(&fake, "#!/bin/sh\n").expect("write fake");
    let found = find_server(&["rust-analyzer"], &dir.path().to_string_lossy());
    // PATHEXT can spell .EXE differently from the file's .exe suffix.
    assert_eq!(
        found
            .expect("fake server")
            .canonicalize()
            .expect("found path"),
        fake.canonicalize().expect("fixture path")
    );
    assert!(find_server(&["rust-analyzer"], "/nonexistent-dir-xyz").is_none());
    assert!(find_server(&["rust-analyzer"], "").is_none());
}

#[test]
fn exe_stems_resolve_through_extension_probing() {
    // Windows `PATH` entries hold `rust-analyzer.exe`, never the bare
    // stem: prove the probing finds it using explicit extensions so
    // the test holds on every platform.
    let extensions = [".EXE", ".BAT"].map(str::to_owned);
    let dir = tempfile::tempdir().expect("tempdir");
    let exe = dir.path().join("rust-analyzer.EXE");
    std::fs::write(&exe, "fake").expect("write fake");
    let path_env = dir.path().to_string_lossy().into_owned();
    assert_eq!(
        find_server_with(&["rust-analyzer"], &path_env, &extensions),
        Some(exe)
    );
    // A stem that already carries an extension is used as-is: no
    // doubled suffix, no probing.
    assert_eq!(
        candidate_file_names("rust-analyzer.EXE", &extensions),
        vec!["rust-analyzer.EXE".to_owned()]
    );
    // Unknown stems still miss, even with extensions configured.
    assert!(find_server_with(&["pylsp"], &path_env, &extensions).is_none());
    assert!(find_server_with(&["rust-analyzer"], "", &extensions).is_none());
}

#[test]
fn utf16_math_round_trips_astral_chars() {
    // `a` + crab (2 units) + `b`: char cols 0..3, unit cols 0..4.
    let line = "a🦀b";
    assert_eq!(char_col_to_utf16(line, 0), 0);
    assert_eq!(char_col_to_utf16(line, 1), 1);
    assert_eq!(char_col_to_utf16(line, 2), 3);
    assert_eq!(char_col_to_utf16(line, 3), 4);
    assert_eq!(char_col_to_utf16(line, 99), 4);
    // Splitting inside the surrogate pair snaps to the pair start.
    assert_eq!(utf16_to_char_col(line, 0), 0);
    assert_eq!(utf16_to_char_col(line, 1), 1);
    assert_eq!(utf16_to_char_col(line, 2), 1);
    assert_eq!(utf16_to_char_col(line, 3), 2);
    assert_eq!(utf16_to_char_col(line, 99), 3);
}

#[test]
fn file_uri_round_trips_special_chars() {
    let path = std::env::temp_dir().join("some dir/f Gö.rs");
    let uri = file_uri(&path).expect("uri");
    assert_eq!(uri.scheme(), "file");
    assert!(uri.as_str().ends_with("/some%20dir/f%20G%C3%B6.rs"));
    assert_eq!(uri_to_path(uri.as_str()), Some(path));
    assert!(file_uri(Path::new("relative/file.rs")).is_none());
    assert!(uri_to_path("https://example.com/x").is_none());
    assert!(uri_to_path(":::").is_none());
}
