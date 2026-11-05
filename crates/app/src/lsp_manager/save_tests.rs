//! `didSave` after native buffer writes, and the diagnostics a save
//! brings back (rust-analyzer runs `cargo check` on it).
use super::tests::ready_key;
use super::*;
use crate::lsp_manager::SubmitTextInput;

type Outbox = tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>;

/// rust-analyzer's sync options: saves wanted, without the text.
fn rust_analyzer_sync() -> serde_json::Value {
    serde_json::json!({"textDocumentSync": {"openClose": true, "change": 2, "save": {}}})
}

/// `path` open at buffer revision 1 on `key`'s server.
fn open_doc(path: &Path, key: &ServerKey) -> OpenDoc {
    OpenDoc {
        uri: L::file_uri(path).expect("uri"),
        version: 1,
        server: key.clone(),
        language_id: "rust",
        submitted: 1,
        observed: 1,
        last_change: Instant::now(),
        opened: true,
        pending_text: None,
    }
}

/// Track `path` as open on the Rust server for its folder.
fn track(manager: &mut LspManager, path: &Path) -> ServerKey {
    let key = ServerKey {
        language: "rust",
        root: project_root(path, Language::Rust),
    };
    manager.docs.insert(path.to_owned(), open_doc(path, &key));
    key
}

/// A ready stand-in server for `key` with these `initialize` capabilities.
fn serve(manager: &mut LspManager, key: &ServerKey, capabilities: serde_json::Value) -> Outbox {
    let outbox = ready_key(manager, key);
    manager.servers.get_mut(key).expect("server").capabilities = capabilities;
    outbox
}

impl LspManager {
    /// Track `path` as open at buffer revision 1 on a ready stand-in
    /// server with these `initialize` capabilities, for tests outside
    /// this module. The receiver gets every frame the server is sent.
    pub(crate) fn open_on_fake_server(
        &mut self,
        path: &Path,
        capabilities: serde_json::Value,
    ) -> Outbox {
        let key = track(self, path);
        serve(self, &key, capabilities)
    }
}

/// Every notification the server was sent, in order: `(method, params)`.
fn sent(outbox: &mut Outbox) -> Vec<(String, serde_json::Value)> {
    let mut decoder = L::FrameDecoder::default();
    let mut out = Vec::new();
    while let Ok(framed) = outbox.try_recv() {
        for body in decoder.feed(&framed) {
            let value: serde_json::Value = serde_json::from_slice(&body).expect("json");
            let method = value["method"].as_str().unwrap_or_default().to_owned();
            out.push((method, value["params"].clone()));
        }
    }
    out
}

fn methods(outbox: &mut Outbox) -> Vec<String> {
    sent(outbox).into_iter().map(|(method, _)| method).collect()
}

#[test]
fn a_save_sends_the_live_text_then_did_save_to_the_tracking_server() {
    let mut manager = LspManager::trusting();
    let path = crate::test_path("/tmp/saved.rs");
    let mut outbox = manager.open_on_fake_server(&path, rust_analyzer_sync());
    // The buffer moved past the text the server holds (debounce pending).
    manager.did_save(&path, (2, "fn main() {}\n"));
    let frames = sent(&mut outbox);
    let names: Vec<&str> = frames.iter().map(|(method, _)| method.as_str()).collect();
    assert_eq!(names, ["textDocument/didChange", "textDocument/didSave"]);
    assert_eq!(frames[0].1["contentChanges"][0]["text"], "fn main() {}\n");
    assert_eq!(frames[0].1["textDocument"]["version"], 2);
    let uri = L::file_uri(&path).unwrap().to_string();
    assert_eq!(frames[1].1["textDocument"]["uri"], uri.as_str());
    assert!(
        frames[1].1.get("text").is_none(),
        "the text was not asked for"
    );
    // The flush was the sync: the debounce sends nothing more for it.
    assert!(!manager.sync_doc(&path, Language::Rust, 2));
    // Saving again without an edit only says so.
    manager.did_save(&path, (2, "fn main() {}\n"));
    assert_eq!(methods(&mut outbox), ["textDocument/didSave"]);
}

#[test]
fn a_save_skips_untracked_files_and_servers_that_did_not_ask() {
    let mut manager = LspManager::trusting();
    let untracked = crate::test_path("/tmp/never-opened.rs");
    manager.did_save(&untracked, (1, "fn main() {}\n"));
    assert!(!manager.tracked(&untracked), "a save opens nothing");
    let path = crate::test_path("/tmp/no-save.rs");
    let no_save = serde_json::json!({"textDocumentSync": {"openClose": true, "change": 2}});
    let mut outbox = manager.open_on_fake_server(&path, no_save);
    manager.did_save(&path, (2, "fn main() {}\n"));
    assert!(
        sent(&mut outbox).is_empty(),
        "no didSave, and no early flush"
    );
    assert_eq!(
        manager.docs[&path].submitted, 1,
        "the debounce still owes it"
    );
    // A server still starting, or a document waiting to reopen, gets
    // nothing either: its open carries the latest text.
    let key = manager.docs[&path].server.clone();
    manager.servers.get_mut(&key).expect("server").capabilities = rust_analyzer_sync();
    manager.servers.get_mut(&key).expect("server").ready = false;
    manager.did_save(&path, (2, "fn main() {}\n"));
    manager.servers.get_mut(&key).expect("server").ready = true;
    manager.docs.get_mut(&path).expect("doc").opened = false;
    manager.did_save(&path, (2, "fn main() {}\n"));
    assert!(sent(&mut outbox).is_empty());
}

#[test]
fn a_save_includes_the_text_when_the_server_asks_for_it() {
    let mut manager = LspManager::trusting();
    let path = crate::test_path("/tmp/with_text.rs");
    let include = serde_json::json!({"textDocumentSync": {"save": {"includeText": true}}});
    let mut outbox = manager.open_on_fake_server(&path, include);
    manager.did_save(&path, (1, "fn saved() {}\n"));
    let frames = sent(&mut outbox);
    assert_eq!(frames.len(), 1, "the server already holds revision 1");
    assert_eq!(frames[0].0, "textDocument/didSave");
    assert_eq!(frames[0].1["text"], "fn saved() {}\n");
}

#[test]
fn a_save_reaches_the_spell_checker_too() {
    let mut manager = LspManager::trusting();
    manager.set_spell_enabled(true);
    let path = crate::test_path("/tmp/spelled.rs");
    let mut main = manager.open_on_fake_server(&path, rust_analyzer_sync());
    let spell_key = ServerKey {
        language: L::SPELL_SERVER,
        root: project_root(&path, Language::Plain),
    };
    manager
        .spell_docs
        .insert(path.clone(), open_doc(&path, &spell_key));
    let save_on = serde_json::json!({"textDocumentSync": {"change": 1, "save": true}});
    let mut spell = serve(&mut manager, &spell_key, save_on);
    manager.did_save(&path, (1, "// recieved\n"));
    assert_eq!(methods(&mut main), ["textDocument/didSave"]);
    assert_eq!(methods(&mut spell), ["textDocument/didSave"]);
}

/// Publish one error at `line` for `path`, the way the reader task hands
/// it to [`LspManager::poll`].
fn publish(manager: &LspManager, key: &ServerKey, path: &Path, line: u32) {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": {
            "uri": L::file_uri(path).unwrap().as_str(),
            "diagnostics": [{
                "range": {"start": {"line": line, "character": 0},
                          "end": {"line": line, "character": 2}},
                "severity": 1,
                "source": "rustc",
                "message": "mismatched types",
            }],
        },
    })
    .to_string();
    let event = responses::decode_body(key, body.as_bytes()).expect("diagnostics");
    manager.pump_tx.send(event).expect("pump");
}

#[test]
fn diagnostics_after_a_save_reach_open_files_and_skip_closed_ones() {
    let mut manager = LspManager::trusting();
    let saved = crate::test_path("/tmp/flycheck/src/main.rs");
    let other = crate::test_path("/tmp/flycheck/src/lib.rs");
    let closed = crate::test_path("/tmp/flycheck/src/unopened.rs");
    let key = track(&mut manager, &saved);
    assert_eq!(track(&mut manager, &other), key, "one server for both");
    let mut outbox = serve(&mut manager, &key, rust_analyzer_sync());
    manager.did_save(&saved, (1, "fn main() {}\n"));
    assert_eq!(methods(&mut outbox), ["textDocument/didSave"]);
    let before = manager.diagnostics_generation();
    // `cargo check` reports across the crate, open or not.
    for (path, line) in [(&saved, 0), (&other, 3), (&closed, 7)] {
        publish(&manager, &key, path, line);
    }
    assert!(!manager.poll(), "one poll routes them all");
    assert_eq!(manager.problems_for(&saved).len(), 1);
    assert_eq!(manager.problems_for(&other)[0].range.start.line, 3);
    assert!(
        manager.problems_for(&closed).is_empty(),
        "files that are not open keep no state"
    );
    assert_eq!(manager.all_problems().len(), 2);
    assert_eq!(manager.problem_counts(), (2, 0, 2));
    assert_ne!(manager.diagnostics_generation(), before);
}

/// Poll like the frame until `file` has a warning at `line`, for at most
/// `limit`.
async fn warning_within(manager: &mut LspManager, file: &Path, line: u32, limit: Duration) -> bool {
    let start = Instant::now();
    loop {
        manager.poll();
        if manager.problems_for(file).iter().any(|item| {
            item.severity == Some(L::Severity::Warning) && item.range.start.line == line
        }) {
            return true;
        }
        if start.elapsed() >= limit {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn problems_note(manager: &LspManager, file: &Path) -> String {
    format!(
        "problems={:?} note={:?}",
        manager.problems_for(file),
        manager.server_note(file)
    )
}

/// Real rust-analyzer when installed; self-skips otherwise (live-provider
/// verification). Dead code is a rustc warning that only `cargo check`
/// reports, and rust-analyzer checks once after loading and then on each
/// `didSave`: a function added later is reported only after the save.
#[tokio::test]
async fn live_rust_analyzer_checks_again_after_a_save() {
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
    // With no lock file, the first `cargo check` writes one, rust-analyzer
    // then reloads the workspace and can drop the check a save asks for.
    std::fs::write(
        root.join("Cargo.lock"),
        "# This file is automatically @generated by Cargo.\n\
         # It is not intended for manual editing.\n\
         version = 4\n\n[[package]]\nname = \"proj\"\nversion = \"0.0.0\"\n",
    )
    .expect("lock file");
    let file = root.join("src/main.rs");
    let first = "fn main() {}\nfn first() {}\n";
    std::fs::write(&file, first).expect("main");
    let mut manager = LspManager::trusting();
    assert!(manager.sync_doc(&file, Language::Rust, 1));
    manager.submit_text(SubmitTextInput {
        path: &file,
        language: Language::Rust,
        revision: 1,
        text: first,
    });
    // The check after loading reports `first`.
    assert!(
        warning_within(&mut manager, &file, 1, Duration::from_mins(2)).await,
        "no warning at line 1; {}",
        problems_note(&manager, &file)
    );
    // Let that check finish: a save while it still runs restarts it, and
    // the restarted check can miss the new text.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let second = "fn main() {}\nfn first() {}\nfn second() {}\n";
    std::fs::write(&file, second).expect("save");
    // On a busy machine rust-analyzer can drop the check that a save asks
    // for while it still loads build data. A user then saves again, and
    // so does this test.
    let start = Instant::now();
    loop {
        manager.did_save(&file, (2, second));
        if warning_within(&mut manager, &file, 2, Duration::from_secs(20)).await {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_mins(2),
            "no warning at line 2 after the saves; {}",
            problems_note(&manager, &file)
        );
    }
    manager.prune(&std::collections::HashSet::new());
}
