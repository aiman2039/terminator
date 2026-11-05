//! Quick Documentation requests: one request per spot, answers kept by
//! the spot they describe, and none shown for another spot or newer text.
use super::*;
use serde_json::json;

type Outbox = tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>;

/// Every message the server was sent, as JSON, in order.
fn sent(outbox: &mut Outbox) -> Vec<serde_json::Value> {
    let mut decoder = L::FrameDecoder::default();
    let mut out = Vec::new();
    while let Ok(framed) = outbox.try_recv() {
        for body in decoder.feed(&framed) {
            out.push(serde_json::from_slice(&body).expect("json"));
        }
    }
    out
}

/// Chars `start..end` of line 0 of `path` at buffer `revision`.
fn spot(path: &Path, revision: u64, (start, end): (usize, usize)) -> HoverKey {
    HoverKey {
        path: path.to_owned(),
        revision,
        line: 0,
        start,
        end,
    }
}

fn at(character: u32) -> L::Position {
    L::Position { line: 0, character }
}

/// The server that tracks `path` answers request `id` with `reply`
/// (`result` or `error`).
fn reply(manager: &mut LspManager, path: &Path, id: &serde_json::Value, reply: serde_json::Value) {
    let key = manager.docs[path].server.clone();
    let mut body = json!({"jsonrpc": "2.0", "id": id});
    if let (Some(body), Some(reply)) = (body.as_object_mut(), reply.as_object()) {
        body.extend(reply.clone());
    }
    manager.on_body(&key, body.to_string().as_bytes());
}

/// A manager with `path` open on a ready server with hover.
fn hovering(path: &Path) -> (LspManager, Outbox) {
    let mut manager = LspManager::trusting();
    let outbox = manager.open_on_fake_server(path, json!({"hoverProvider": true}));
    (manager, outbox)
}

#[test]
fn a_spot_is_asked_once_and_its_answer_kept() {
    let path = crate::test_path("/tmp/hover.rs");
    let (mut manager, mut outbox) = hovering(&path);
    let word = spot(&path, 1, (4, 9));
    assert!(!manager.hover_asked(&word));
    assert_eq!(manager.request_hover(&word, at(6)), Ok(true));
    let request = sent(&mut outbox).pop().expect("hover");
    assert_eq!(request["method"], "textDocument/hover");
    assert_eq!(
        request["params"]["position"],
        json!({"line": 0, "character": 6})
    );
    assert_eq!(manager.hover_docs(&word), None, "on its way");
    assert_eq!(manager.request_hover(&word, at(5)), Ok(true));
    assert!(sent(&mut outbox).is_empty(), "one request per spot");
    let docs = json!({"contents": {"kind": "markdown", "value": "fn value()"}});
    reply(
        &mut manager,
        &path,
        &request["id"],
        json!({ "result": docs }),
    );
    let shown = manager.hover_docs(&word).flatten().expect("docs");
    assert_eq!(shown.text, "fn value()");
    assert!(shown.markdown);
    // Back on the word later: the kept answer, nothing sent.
    assert_eq!(manager.request_hover(&word, at(8)), Ok(true));
    assert!(sent(&mut outbox).is_empty());
    // A closed file forgets its answers.
    manager.prune(&std::collections::HashSet::new());
    assert_eq!(manager.hover_docs(&word), None);
}

#[test]
fn an_answer_never_shows_for_another_spot_or_newer_text() {
    let path = crate::test_path("/tmp/spots.rs");
    let (mut manager, mut outbox) = hovering(&path);
    let (first, second) = (spot(&path, 1, (0, 3)), spot(&path, 1, (8, 13)));
    assert_eq!(manager.request_hover(&first, at(1)), Ok(true));
    assert_eq!(manager.request_hover(&second, at(9)), Ok(true));
    let requests = sent(&mut outbox);
    assert_eq!(requests.len(), 2);
    // Answers come back out of order.
    let answer = |text: &str| json!({"result": {"contents": text}});
    reply(&mut manager, &path, &requests[1]["id"], answer("second"));
    reply(&mut manager, &path, &requests[0]["id"], answer("first"));
    let text = |manager: &LspManager, key| {
        manager
            .hover_docs(key)
            .flatten()
            .map(|docs| docs.text.clone())
    };
    assert_eq!(text(&manager, &first).as_deref(), Some("first"));
    assert_eq!(text(&manager, &second).as_deref(), Some("second"));
    // The same word after an edit is a new spot: not answered yet.
    let edited = spot(&path, 2, (0, 3));
    assert_eq!(manager.hover_docs(&edited), None);
    assert!(!manager.hover_asked(&edited));
}

#[test]
fn null_and_failed_answers_mean_no_docs_and_are_asked_again() {
    let path = crate::test_path("/tmp/empty.rs");
    let (mut manager, mut outbox) = hovering(&path);
    let blank = spot(&path, 1, (0, 2));
    assert_eq!(manager.request_hover(&blank, at(0)), Ok(true));
    let request = sent(&mut outbox).pop().expect("hover");
    reply(&mut manager, &path, &request["id"], json!({"result": null}));
    assert_eq!(manager.hover_docs(&blank), Some(None));
    // A server still loading answers null: the next ask goes out again.
    assert!(!manager.hover_asked(&blank));
    assert_eq!(manager.request_hover(&blank, at(0)), Ok(true));
    assert_eq!(sent(&mut outbox).len(), 1, "asked again");
    let busy = spot(&path, 1, (4, 6));
    assert_eq!(manager.request_hover(&busy, at(4)), Ok(true));
    let request = sent(&mut outbox).pop().expect("hover");
    let error = json!({"error": {"code": -32801, "message": "content modified"}});
    reply(&mut manager, &path, &request["id"], error);
    assert_eq!(
        manager.hover_docs(&busy),
        Some(None),
        "the popup stops waiting"
    );
    assert!(!manager.hover_asked(&busy));
    assert_eq!(manager.request_hover(&busy, at(4)), Ok(true));
    assert_eq!(sent(&mut outbox).len(), 1, "asked again");
}

#[test]
fn files_without_a_hover_server_send_nothing_and_a_starting_one_says_so() {
    let path = crate::test_path("/tmp/no-hover.rs");
    let mut manager = LspManager::trusting();
    let mut outbox = manager.open_on_fake_server(&path, json!({}));
    let word = spot(&path, 1, (0, 3));
    assert_eq!(manager.request_hover(&word, at(0)), Ok(false));
    assert!(sent(&mut outbox).is_empty());
    let untracked = crate::test_path("/tmp/notes.txt");
    assert_eq!(
        manager.request_hover(&spot(&untracked, 1, (0, 3)), at(0)),
        Ok(false)
    );
    let key = manager.docs[&path].server.clone();
    if let Some(server) = manager.servers.get_mut(&key) {
        server.ready = false;
    }
    let note = manager.request_hover(&word, at(0)).expect_err("starting");
    assert!(note.contains("Starting"), "{note}");
    assert!(sent(&mut outbox).is_empty());
}
