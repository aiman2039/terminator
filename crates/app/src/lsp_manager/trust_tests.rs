//! Trust gate and per-frame pump budget regressions.
use super::*;
use crate::lsp_manager::SubmitTextInput;

#[test]
fn poll_routes_a_bounded_batch_and_reports_backlog() {
    let mut manager = LspManager::trusting();
    let key = ServerKey {
        language: "rust",
        root: crate::test_path("/tmp/burst"),
    };
    for _ in 0..POLL_BUDGET.saturating_add(5) {
        let _ = manager.pump_tx.send(PumpEvent::Down { key: key.clone() });
    }
    assert!(manager.poll(), "events beyond the budget wait a frame");
    assert!(!manager.poll(), "the next frame drains the rest");
}

#[test]
fn reader_decode_drops_ignored_notifications_and_types_diagnostics() {
    let key = ServerKey {
        language: "rust",
        root: crate::test_path("/tmp/decode"),
    };
    let progress = br#"{"jsonrpc":"2.0","method":"$/progress","params":{}}"#;
    assert!(responses::decode_body(&key, progress).is_none());
    let path = crate::test_path("/tmp/decode/a.rs");
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": {"uri": L::file_uri(&path).unwrap().as_str(), "diagnostics": []},
    })
    .to_string();
    let event = responses::decode_body(&key, body.as_bytes());
    assert!(matches!(event, Some(PumpEvent::Diagnostics { path: got, .. }) if got == path));
}

#[test]
fn untrusted_project_asks_once_and_never_spawns_a_server() {
    let mut manager = LspManager::new();
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("Cargo.toml"), "[package]").expect("manifest");
    let path = dir.path().join("main.rs");
    let submit = |manager: &mut LspManager| {
        manager.submit_text(SubmitTextInput {
            path: &path,
            language: Language::Rust,
            revision: 1,
            text: "fn main() {}",
        });
    };
    submit(&mut manager);
    submit(&mut manager);
    assert!(manager.servers.is_empty());
    assert_eq!(manager.trust_request_for(&path), Some(dir.path()));
    assert_eq!(manager.trust_asks.len(), 1);
    assert!(!manager.sync_doc(&path, Language::Rust, 1));
    manager.set_trust(dir.path(), false);
    assert!(manager.trust_request_for(&path).is_none());
    submit(&mut manager);
    assert!(manager.servers.is_empty());
    assert!(
        manager.trust_asks.is_empty(),
        "a refusal is not asked again"
    );
    assert!(
        manager
            .server_note(&path)
            .is_some_and(|note| note.contains("not trusted"))
    );
}

#[test]
fn saved_trust_lets_the_next_sync_start_the_server() {
    let mut manager = LspManager::new();
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("Cargo.toml"), "[package]").expect("manifest");
    let path = dir.path().join("main.rs");
    manager.submit_text(SubmitTextInput {
        path: &path,
        language: Language::Rust,
        revision: 1,
        text: "fn main() {}",
    });
    assert!(!manager.sync_doc(&path, Language::Rust, 1));
    let saved = HashMap::from([(dir.path().to_path_buf(), true)]);
    manager.set_trust_decisions(&saved);
    assert!(manager.trust_request_for(&path).is_none());
    assert!(manager.sync_doc(&path, Language::Rust, 1));
}

#[test]
fn go_and_user_servers_wait_for_the_trust_answer() {
    let mut manager = LspManager::new();
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("go.mod"), "module x").expect("manifest");
    let path = dir.path().join("main.go");
    manager.submit_text(SubmitTextInput {
        path: &path,
        language: Language::Go,
        revision: 1,
        text: "package main",
    });
    assert!(manager.servers.is_empty());
    assert_eq!(manager.trust_request_for(&path), Some(dir.path()));
    assert!(L::family_needs_trust("custom:anything"));
}

#[test]
fn missing_server_note_names_the_install_command() {
    let manager = LspManager::new();
    let note = manager.missing_server_note("go");
    assert!(note.contains("gopls"), "{note}");
    assert!(note.contains("Install: go install"), "{note}");
}
