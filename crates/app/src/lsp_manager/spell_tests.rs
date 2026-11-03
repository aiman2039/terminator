//! Spell checker (`harper-ls`) beside the language server.
use super::*;

type Outbox = tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>;

fn open_doc(path: &Path, language: &'static str) -> OpenDoc {
    OpenDoc {
        uri: L::file_uri(path).expect("uri"),
        version: 1,
        server: ServerKey {
            language,
            root: project_root(path, Language::Plain),
        },
        language_id: "rust",
        submitted: 1,
        observed: 1,
        last_change: Instant::now(),
        opened: true,
        pending_text: None,
    }
}

use super::tests::ready_key as ready;

/// A file tracked by both a language server and the spell checker.
fn both(manager: &mut LspManager, path: &Path) -> (ServerKey, Outbox, ServerKey, Outbox) {
    manager.set_spell_enabled(true);
    let main = open_doc(path, "rust");
    let spell = open_doc(path, L::SPELL_SERVER);
    let (main_key, spell_key) = (main.server.clone(), spell.server.clone());
    manager.docs.insert(path.to_owned(), main);
    manager.spell_docs.insert(path.to_owned(), spell);
    let main_out = ready(manager, &main_key);
    let spell_out = ready(manager, &spell_key);
    (main_key, main_out, spell_key, spell_out)
}

fn typo(manager: &mut LspManager, key: &ServerKey, path: &Path) {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": {
            "uri": L::file_uri(path).unwrap().as_str(),
            "diagnostics": [{
                "range": {"start": {"line": 0, "character": 3},
                          "end": {"line": 0, "character": 11}},
                "severity": 4,
                "message": "Did you mean to spell `recieved` this way?",
            }],
        },
    })
    .to_string();
    manager.on_body(key, body.as_bytes());
}

/// The one request on `outbox`: `(id, method, params)`.
fn sent(outbox: &mut Outbox) -> (i64, String, serde_json::Value) {
    let framed = outbox.try_recv().expect("request");
    let bodies = L::FrameDecoder::default().feed(&framed);
    let body: serde_json::Value = serde_json::from_slice(&bodies[0]).expect("json");
    (
        body["id"].as_i64().expect("id"),
        body["method"].as_str().expect("method").to_owned(),
        body["params"].clone(),
    )
}

fn answer(manager: &mut LspManager, key: &ServerKey, id: i64, result: serde_json::Value) {
    let body = serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string();
    manager.on_body(key, body.as_bytes());
}

#[test]
fn spelling_underlines_stay_out_of_problems() {
    let mut manager = LspManager::trusting();
    let path = crate::test_path("/tmp/spell.rs");
    let (_, _main, spell_key, _spell) = both(&mut manager, &path);
    typo(&mut manager, &spell_key, &path);
    let underlines = manager.editor_diagnostics(&path);
    assert_eq!(underlines.len(), 1);
    assert_eq!(underlines[0].severity, Some(L::Severity::Spelling));
    assert!(manager.problems_for(&path).is_empty());
    assert!(manager.all_problems().is_empty());
    assert_eq!(manager.problem_counts(), (0, 0, 0));
    assert!(manager.row_marks(&path).is_empty());
}

#[test]
fn code_actions_wait_for_both_servers_and_lead_with_spell_fixes() {
    let mut manager = LspManager::trusting();
    let path = crate::test_path("/tmp/spell_actions.rs");
    let (main_key, mut main_out, spell_key, mut spell_out) = both(&mut manager, &path);
    typo(&mut manager, &spell_key, &path);
    let at = L::Position {
        line: 0,
        character: 5,
    };
    let generation = manager
        .request_code_actions(&path, L::Range { start: at, end: at })
        .expect("sent");
    let (main_id, _, _) = sent(&mut main_out);
    let (spell_id, method, params) = sent(&mut spell_out);
    assert_eq!(method, "textDocument/codeAction");
    // The spell checker sees only its own finding.
    assert_eq!(
        params["context"]["diagnostics"].as_array().map(Vec::len),
        Some(1)
    );
    answer(
        &mut manager,
        &main_key,
        main_id,
        serde_json::json!([{
            "title": "Inline variable", "kind": "refactor.inline",
            "edit": {"changes": {}},
        }]),
    );
    assert!(
        manager.edit_answers.actions.is_none(),
        "waits for the spell checker"
    );
    let uri = L::file_uri(&path).unwrap().to_string();
    answer(
        &mut manager,
        &spell_key,
        spell_id,
        serde_json::json!([
            {"title": "Replace with: “received”", "kind": "quickfix",
             "edit": {"changes": {uri.clone(): []}}},
            {"title": "Add \"recieved\" to the user dictionary.",
             "command": "HarperAddToUserDict", "arguments": ["recieved", uri]},
        ]),
    );
    let (answered, number, items) = manager.edit_answers.actions.take().expect("merged");
    assert_eq!((answered, number), (path, generation));
    let titles: Vec<&str> = items.iter().map(|item| item.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Replace with: “received”",
            "Add \"recieved\" to the user dictionary.",
            "Inline variable",
        ]
    );
    assert!(items[0].spell && items[1].spell && !items[2].spell);
}

#[test]
fn clean_lines_never_wait_on_the_spell_checker() {
    let mut manager = LspManager::trusting();
    let path = crate::test_path("/tmp/spell_clean.rs");
    let (main_key, mut main_out, _, mut spell_out) = both(&mut manager, &path);
    let at = L::Position {
        line: 0,
        character: 0,
    };
    manager
        .request_code_actions(&path, L::Range { start: at, end: at })
        .expect("sent");
    assert!(spell_out.try_recv().is_err());
    let (id, _, _) = sent(&mut main_out);
    answer(&mut manager, &main_key, id, serde_json::json!([]));
    assert!(manager.edit_answers.actions.is_some());
}

#[test]
fn dictionary_commands_run_on_the_spell_server() {
    let mut manager = LspManager::trusting();
    let path = crate::test_path("/tmp/spell_command.rs");
    let (_, mut main_out, _, mut spell_out) = both(&mut manager, &path);
    let item = L::parse_spell_actions(&serde_json::json!([{
        "title": "Add \"recieved\" to the user dictionary.",
        "command": "HarperAddToUserDict",
        "arguments": ["recieved", "file:///tmp/spell_command.rs"],
    }]))
    .remove(0);
    assert!(manager.execute_action_command(&path, &item));
    assert!(main_out.try_recv().is_err());
    let (_, method, params) = sent(&mut spell_out);
    assert_eq!(method, "workspace/executeCommand");
    assert_eq!(params["command"], "HarperAddToUserDict");
    assert_eq!(params["arguments"][0], "recieved");
}

#[test]
fn turning_spell_check_off_closes_spell_documents() {
    let mut manager = LspManager::trusting();
    let path = crate::test_path("/tmp/spell_off.txt");
    let (_, _main, spell_key, mut spell_out) = both(&mut manager, &path);
    typo(&mut manager, &spell_key, &path);
    manager.set_spell_enabled(false);
    assert!(manager.editor_diagnostics(&path).is_empty());
    assert!(!manager.spell_docs.contains_key(&path));
    let framed = spell_out.try_recv().expect("didClose");
    let bodies = L::FrameDecoder::default().feed(&framed);
    let body: serde_json::Value = serde_json::from_slice(&bodies[0]).expect("json");
    assert_eq!(body["method"], "textDocument/didClose");
    // Plain text never syncs while it is off.
    assert!(!manager.sync_doc(&path, Language::Plain, 2));
}

#[test]
fn plain_text_syncs_for_the_spell_checker_only() {
    let mut manager = LspManager::trusting();
    let path = crate::test_path("/tmp/notes.txt");
    assert!(
        !manager.sync_doc(&path, Language::Plain, 1),
        "off by default"
    );
    manager.set_spell_enabled(true);
    assert!(manager.sync_doc(&path, Language::Plain, 1));
    assert!(!manager.docs.contains_key(&path));
}
