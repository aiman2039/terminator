//! Completion docs, label details, `isIncomplete`, trigger context, and
//! `completionItem/resolve`.
use super::*;
use serde_json::json;

fn body_of(framed: &[u8]) -> Value {
    let mut decoder = FrameDecoder::default();
    let bodies = decoder.feed(framed);
    assert_eq!(bodies.len(), 1);
    serde_json::from_slice(&bodies[0]).expect("valid json")
}

#[test]
fn completion_items_keep_docs_label_details_and_the_raw_item() {
    let list = json!({
        "isIncomplete": true,
        "items": [
            {
                "label": "len",
                "labelDetails": {"detail": "()", "description": "fn(&self) -> usize"},
                "kind": 2,
                "documentation": {
                    "kind": "markdown",
                    "value": "Returns the number of elements.\n\n```rust\nlet n = v.len();\n```",
                },
                "data": {"position": 7},
            },
            {"label": "push", "documentation": "Appends an element."},
            {
                "label": "plain",
                "documentation": {"kind": "plaintext", "value": "Plain words."},
                "labelDetails": {"detail": " "},
            },
            {"label": "blank", "documentation": "  "},
        ],
    });
    let parsed = parse_completion_list(&list);
    assert!(parsed.incomplete);
    let [len, push, plain, blank] = parsed.items.as_slice() else {
        panic!("four items: {:?}", parsed.items);
    };
    assert_eq!(len.label_detail.as_deref(), Some("()"));
    assert_eq!(len.label_description.as_deref(), Some("fn(&self) -> usize"));
    let docs = len.documentation.as_ref().expect("docs");
    assert!(docs.markdown);
    assert!(docs.text.starts_with("Returns the number"));
    assert_eq!(len.raw, list["items"][0], "the item goes back as sent");
    assert!(!len.resolved);
    // A plain string is plain text, as the protocol says.
    assert_eq!(
        push.documentation,
        Some(Hover {
            text: "Appends an element.".into(),
            markdown: false,
        })
    );
    assert_eq!(
        plain.documentation.as_ref().map(|doc| doc.markdown),
        Some(false)
    );
    assert_eq!(plain.label_detail, None, "blank details draw nothing");
    assert_eq!(blank.documentation, None);
}

#[test]
fn the_incomplete_flag_follows_the_answer() {
    let complete = json!({"isIncomplete": false, "items": [{"label": "a"}]});
    assert!(!parse_completion_list(&complete).incomplete);
    let array = parse_completion_list(&json!([{"label": "a"}]));
    assert_eq!((array.items.len(), array.incomplete), (1, false));
    // A list without the flag still lists its items.
    let bare = parse_completion_list(&json!({"items": [{"label": "a"}]}));
    assert_eq!((bare.items.len(), bare.incomplete), (1, false));
    assert_eq!(
        parse_completion_list(&Value::Null),
        CompletionList::default()
    );
    // One bad item does not drop the rest.
    let mixed = json!({"isIncomplete": true, "items": [{"label": 5}, {"label": "ok"}]});
    let mixed = parse_completion_list(&mixed);
    assert!(mixed.incomplete);
    let labels: Vec<&str> = mixed.items.iter().map(|item| item.label.as_str()).collect();
    assert_eq!(labels, ["ok"]);
}

#[test]
fn resolve_fills_docs_detail_and_imports_but_keeps_the_insert() {
    let listed = parse_completion_item(&json!({
        "label": "HashSet",
        "labelDetails": {"description": "std::collections::HashSet"},
        "textEdit": {
            "range": {"start": {"line": 3, "character": 4}, "end": {"line": 3, "character": 8}},
            "newText": "HashSet",
        },
        "data": {"imports": [{"full_import_path": "std::collections::HashSet"}]},
    }))
    .expect("listed item");
    let answer = parse_completion_item(&json!({
        "label": "HashSet",
        "detail": "struct HashSet<T, S = RandomState>",
        "documentation": {"kind": "markdown", "value": "A hash set."},
        "insertText": "SomethingElse",
        "additionalTextEdits": [{
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
            "newText": "use std::collections::HashSet;\n",
        }],
    }))
    .expect("resolved item");
    let mut item = listed.clone();
    item.merge_resolved(answer);
    assert!(item.resolved);
    assert_eq!(
        item.detail.as_deref(),
        Some("struct HashSet<T, S = RandomState>")
    );
    assert_eq!(
        item.documentation.as_ref().map(|doc| doc.text.as_str()),
        Some("A hash set.")
    );
    assert_eq!(item.additional.len(), 1);
    assert!(item.additional[0].new_text.starts_with("use std"));
    // What the list showed and inserts stays.
    assert_eq!(item.insert, listed.insert);
    assert_eq!(item.edit, listed.edit);
    assert_eq!(item.label_description, listed.label_description);
    assert_eq!(item.raw, listed.raw);
    // An answer without the lazy parts keeps what the list had.
    let mut kept = listed;
    kept.detail = Some("listed".into());
    kept.merge_resolved(parse_completion_item(&json!({"label": "HashSet"})).expect("bare"));
    assert_eq!(kept.detail.as_deref(), Some("listed"));
    assert!(kept.resolved, "asked once, never again");
    assert!(parse_completion_item(&Value::Null).is_none());
}

#[test]
fn resolve_sends_the_listed_item_back() {
    let raw = json!({"label": "len", "data": {"hash": "abc"}});
    let body = body_of(&completion_resolve(&RequestId::Number(4), &raw));
    assert_eq!(body["method"], "completionItem/resolve");
    assert_eq!(body["id"], 4);
    assert_eq!(body["params"], raw);
}

#[test]
fn completion_context_names_why_it_was_asked() {
    let uri = Url::parse("file:///repo/main.rs").expect("uri");
    let at = Position {
        line: 2,
        character: 6,
    };
    let params =
        |trigger| body_of(&completion(&RequestId::Number(1), &uri, at, trigger))["params"].clone();
    let invoked = params(CompletionTrigger::Invoked);
    assert_eq!(invoked["position"], json!({"line": 2, "character": 6}));
    assert_eq!(invoked["context"], json!({"triggerKind": 1}));
    assert_eq!(
        params(CompletionTrigger::Character('.'))["context"],
        json!({"triggerKind": 2, "triggerCharacter": "."})
    );
    assert_eq!(
        params(CompletionTrigger::Incomplete)["context"],
        json!({"triggerKind": 3})
    );
}

#[test]
fn server_trigger_characters_and_resolve_support() {
    // rust-analyzer's provider, plus entries that are not one character.
    let caps = json!({"completionProvider": {
        "triggerCharacters": [":", ".", "'", "(", "::", ""],
        "resolveProvider": true,
    }});
    assert_eq!(completion_triggers(&caps), [':', '.', '\'', '(']);
    assert!(completion_resolves(&caps));
    assert!(completion_triggers(&json!({})).is_empty());
    assert!(!completion_resolves(&json!({"completionProvider": {}})));
    assert!(!completion_resolves(&Value::Null));
}

#[test]
fn initialize_asks_for_lazy_completion_details() {
    let root = Url::parse("file:///repo").expect("root");
    let body = body_of(&initialize(&RequestId::Number(0), 7, &root));
    let completion = &body["params"]["capabilities"]["textDocument"]["completion"];
    let item = &completion["completionItem"];
    assert_eq!(
        item["resolveSupport"]["properties"],
        json!(["documentation", "detail", "additionalTextEdits"])
    );
    assert_eq!(item["labelDetailsSupport"], true);
    assert_eq!(
        item["documentationFormat"],
        json!(["markdown", "plaintext"])
    );
    assert_eq!(completion["contextSupport"], true);
}
