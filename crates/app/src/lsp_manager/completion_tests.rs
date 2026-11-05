//! Completion requests and answers: why a list was asked for, the
//! `isIncomplete` flag, trigger characters, and `completionItem/resolve`.
use super::*;
use crate::lsp_manager::SubmitTextInput;
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

/// rust-analyzer's completion provider.
fn rust_analyzer() -> serde_json::Value {
    json!({"completionProvider": {
        "triggerCharacters": [":", ".", "'", "("],
        "resolveProvider": true,
    }})
}

/// The server that tracks `path` answers request `id` with `result`.
fn answer(
    manager: &mut LspManager,
    path: &Path,
    (id, result): (&serde_json::Value, serde_json::Value),
) {
    let key = manager.docs[path].server.clone();
    let body = json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string();
    manager.on_body(&key, body.as_bytes());
}

#[test]
fn completion_requests_say_why_and_answers_flag_incomplete_lists() {
    let mut manager = LspManager::trusting();
    let path = crate::test_path("/tmp/complete.rs");
    let mut outbox = manager.open_on_fake_server(&path, rust_analyzer());
    assert_eq!(manager.completion_triggers(&path), [':', '.', '\'', '(']);
    let at = L::Position {
        line: 2,
        character: 6,
    };
    let generation = manager
        .request_completion(&path, at, L::CompletionTrigger::Character('.'))
        .expect("sent");
    let request = sent(&mut outbox).pop().expect("request");
    assert_eq!(request["method"], "textDocument/completion");
    assert_eq!(
        request["params"]["context"],
        json!({"triggerKind": 2, "triggerCharacter": "."})
    );
    let list = json!({"isIncomplete": true, "items": [
        {"label": "len", "labelDetails": {"description": "fn(&self) -> usize"}},
    ]});
    answer(&mut manager, &path, (&request["id"], list));
    let results = manager.take_completion().expect("answer");
    assert_eq!(results.generation, generation);
    assert!(results.incomplete, "the popup asks again as the user types");
    assert_eq!(
        results.items[0].label_description.as_deref(),
        Some("fn(&self) -> usize")
    );
}

#[test]
fn resolve_goes_only_to_servers_that_resolve_items() {
    let mut manager = LspManager::trusting();
    let path = crate::test_path("/tmp/no-resolve/main.rs");
    let mut outbox = manager.open_on_fake_server(&path, json!({"completionProvider": {}}));
    let target = ResolveFor::Popup {
        generation: 1,
        index: 0,
    };
    let item = json!({"label": "len"});
    assert!(!manager.request_completion_resolve(&path, &item, target));
    assert!(sent(&mut outbox).is_empty(), "nothing is sent");
    assert!(manager.completion_triggers(&path).is_empty());
    let untracked = crate::test_path("/tmp/untracked.rs");
    assert!(!manager.request_completion_resolve(&untracked, &item, target));
}

#[test]
fn resolve_answers_wait_for_their_own_editor() {
    let mut manager = LspManager::trusting();
    let path = crate::test_path("/tmp/resolve.rs");
    let mut outbox = manager.open_on_fake_server(&path, rust_analyzer());
    let item = json!({"label": "HashSet", "data": {"imports": ["std::collections::HashSet"]}});
    let target = ResolveFor::Accepted {
        line: 3,
        col: 4,
        before: 9,
    };
    assert!(manager.request_completion_resolve(&path, &item, target));
    let request = sent(&mut outbox).pop().expect("resolve");
    assert_eq!(request["method"], "completionItem/resolve");
    assert_eq!(request["params"], item, "the item goes back as listed");
    let resolved = json!({
        "label": "HashSet",
        "documentation": {"kind": "markdown", "value": "A hash set."},
        "additionalTextEdits": [{
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
            "newText": "use std::collections::HashSet;\n",
        }],
    });
    answer(&mut manager, &path, (&request["id"], resolved));
    let other = crate::test_path("/tmp/other.rs");
    assert!(manager.take_resolved_completions(&other).is_empty());
    let taken = manager.take_resolved_completions(&path);
    assert_eq!(taken.len(), 1);
    assert_eq!(taken[0].target, target);
    assert_eq!(taken[0].item.additional.len(), 1);
    assert!(taken[0].item.documentation.is_some());
    assert!(
        manager.take_resolved_completions(&path).is_empty(),
        "taken once"
    );
    // A failed resolve leaves nothing to merge.
    assert!(manager.request_completion_resolve(&path, &item, target));
    let request = sent(&mut outbox).pop().expect("resolve");
    let key = manager.docs[&path].server.clone();
    let failed = json!({"jsonrpc": "2.0", "id": request["id"],
        "error": {"code": -32801, "message": "content modified"}});
    manager.on_body(&key, failed.to_string().as_bytes());
    assert!(manager.take_resolved_completions(&path).is_empty());
}

#[test]
fn resolve_answers_for_a_closed_file_are_dropped() {
    let mut manager = LspManager::trusting();
    let path = crate::test_path("/tmp/closed.rs");
    let mut outbox = manager.open_on_fake_server(&path, rust_analyzer());
    let key = manager.docs[&path].server.clone();
    let target = ResolveFor::Accepted {
        line: 1,
        col: 4,
        before: 9,
    };
    let item = json!({"label": "HashMap"});
    let resolved = json!({"label": "HashMap", "additionalTextEdits": [{
        "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
        "newText": "use std::collections::HashMap;\n",
    }]});
    let answer_to = |manager: &mut LspManager, request: &serde_json::Value| {
        let body = json!({"jsonrpc": "2.0", "id": request["id"], "result": resolved});
        manager.on_body(&key, body.to_string().as_bytes());
    };
    // An answer that waited when the tab closed goes with it.
    assert!(manager.request_completion_resolve(&path, &item, target));
    let early = sent(&mut outbox).pop().expect("resolve");
    answer_to(&mut manager, &early);
    // One that lands after the close is not kept for a reopened file.
    assert!(manager.request_completion_resolve(&path, &item, target));
    let late = sent(&mut outbox).pop().expect("resolve");
    manager.close_path(&path);
    answer_to(&mut manager, &late);
    assert!(manager.take_resolved_completions(&path).is_empty());
}

/// One completion request at `at`: its items, or `None` when the server
/// is not ready or does not answer within 2 s.
async fn ask_once(
    manager: &mut LspManager,
    file: &Path,
    at: L::Position,
) -> Option<Vec<L::CompletionItem>> {
    manager.request_completion(file, at, L::CompletionTrigger::Invoked)?;
    for _ in 0..40 {
        manager.poll();
        if let Some(results) = manager.take_completion() {
            return Some(results.items);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    None
}

/// Ask for completions at `at` until an item labeled `label` arrives.
async fn listed_item(
    manager: &mut LspManager,
    file: &Path,
    (at, label): (L::Position, &str),
) -> L::CompletionItem {
    let start = Instant::now();
    loop {
        assert!(
            start.elapsed() < Duration::from_mins(2),
            "no {label}; note={:?}",
            manager.server_note(file)
        );
        let items = ask_once(manager, file, at).await.unwrap_or_default();
        if let Some(found) = items.into_iter().find(|item| item.label == label) {
            return found;
        }
        manager.poll();
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Resolve `item` and wait for the answer.
async fn resolved_item(
    manager: &mut LspManager,
    file: &Path,
    item: &L::CompletionItem,
) -> L::CompletionItem {
    let target = ResolveFor::Popup {
        generation: 1,
        index: 0,
    };
    assert!(manager.request_completion_resolve(file, &item.raw, target));
    let start = Instant::now();
    loop {
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "no resolve answer"
        );
        manager.poll();
        if let Some(answer) = manager.take_resolved_completions(file).pop() {
            return answer.item;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Real rust-analyzer when installed; self-skips otherwise (live-provider
/// verification). With lazy resolve advertised it lists `HashSet` from a
/// module that is not imported yet; resolving it brings the docs and the
/// `use` line.
#[tokio::test]
async fn live_rust_analyzer_resolves_docs_and_import_edits() {
    let path_env = std::env::var_os("PATH")
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    if L::find_server(&["rust-analyzer"], &path_env).is_none() {
        crate::console::Console::err(format_args!("SKIP: rust-analyzer not on PATH"));
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("proj");
    std::fs::create_dir_all(root.join("src")).expect("mkdir");
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"proj\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .expect("manifest");
    let main = "fn main() {\n    let s = HashSe\n}\n";
    let file = root.join("src/main.rs");
    std::fs::write(&file, main).expect("main");
    let mut manager = LspManager::trusting();
    assert!(manager.sync_doc(&file, Language::Rust, 1));
    manager.submit_text(SubmitTextInput {
        path: &file,
        language: Language::Rust,
        revision: 1,
        text: main,
    });
    let at = L::Position {
        line: 1,
        character: 18,
    };
    // Under load an answer can come while the server still loads the
    // standard library: its resolve finds no import yet, and the server
    // returns the item as listed. List and resolve again until it does.
    let start = Instant::now();
    let resolved = loop {
        let set = listed_item(&mut manager, &file, (at, "HashSet")).await;
        assert!(set.additional.is_empty(), "the import waits for resolve");
        let resolved = resolved_item(&mut manager, &file, &set).await;
        if resolved
            .additional
            .iter()
            .any(|edit| edit.new_text.contains("use std::collections::HashSet"))
        {
            break resolved;
        }
        assert!(
            start.elapsed() < Duration::from_mins(2),
            "no import edit: {:?}",
            resolved.additional
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    };
    let docs = resolved.documentation.expect("docs");
    assert!(docs.markdown && !docs.text.trim().is_empty(), "{docs:?}");
    manager.prune(&std::collections::HashSet::new());
}
