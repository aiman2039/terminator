use super::*;

#[test]
fn root_prefers_language_marker_over_git() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("inner/src")).expect("mkdir");
    std::fs::write(root.join("Cargo.toml"), "[package]").expect("manifest");
    std::fs::create_dir_all(root.join("inner/.git")).expect("git");
    let file = root.join("inner/src/main.rs");
    assert_eq!(project_root(&file, Language::Rust), root);
}

#[test]
fn root_falls_back_to_parent_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("lonely.py");
    assert_eq!(project_root(&file, Language::Python), dir.path());
}

#[test]
fn unsupported_language_never_syncs() {
    let mut manager = LspManager::new();
    let path = crate::test_path("/tmp/note.md");
    assert!(!manager.sync_doc(&path, Language::Markdown, 1));
    assert!(manager.row_marks(&path).is_empty());
    assert!(manager.problems_for(&path).is_empty());
}

#[test]
fn marks_keep_worst_severity_per_row_sorted() {
    let mut manager = LspManager::new();
    let path = crate::test_path("/tmp/main.rs");
    manager.diagnostics.insert(
        path.clone(),
        vec![
            L::Diagnostic {
                range: L::Range {
                    start: L::Position {
                        line: 9,
                        character: 0,
                    },
                    end: L::Position {
                        line: 9,
                        character: 1,
                    },
                },
                severity: Some(L::Severity::Warning),
                message: "w".into(),
            },
            L::Diagnostic {
                range: L::Range {
                    start: L::Position {
                        line: 9,
                        character: 5,
                    },
                    end: L::Position {
                        line: 9,
                        character: 6,
                    },
                },
                severity: Some(L::Severity::Error),
                message: "e".into(),
            },
            L::Diagnostic {
                range: L::Range {
                    start: L::Position {
                        line: 2,
                        character: 0,
                    },
                    end: L::Position {
                        line: 2,
                        character: 1,
                    },
                },
                severity: None,
                message: "plain".into(),
            },
        ],
    );
    let marks = manager.row_marks(&path);
    assert_eq!(marks.len(), 2);
    assert_eq!(marks[0].line, 2);
    assert_eq!(marks[0].severity, L::Severity::Hint);
    assert_eq!(marks[1].line, 9);
    assert_eq!(marks[1].severity, L::Severity::Error);
    // Status-bar counts: one error, one warning, one unseverity'd.
    assert_eq!(manager.problem_counts(), (1, 1, 3));
    assert!(manager.tracked(&path) || !manager.problems_for(&path).is_empty());
}

#[test]
fn problem_counts_start_empty_and_untracked() {
    let manager = LspManager::new();
    assert_eq!(manager.problem_counts(), (0, 0, 0));
    assert!(!manager.tracked(std::path::Path::new("/tmp/never-opened.rs")));
    assert_eq!(
        manager.server_note(std::path::Path::new("/tmp/never-opened.rs")),
        None
    );
}

#[test]
fn a_missing_server_stops_per_frame_resubmits() {
    let mut manager = LspManager::new();
    let mut overrides = std::collections::BTreeMap::new();
    overrides.insert(
        "rust".to_owned(),
        "/nonexistent/terminator-test-rust-analyzer".to_owned(),
    );
    manager.set_server_overrides(&overrides);
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("main.rs");
    assert!(manager.sync_doc(&path, Language::Rust, 1));
    manager.submit_text(&path, Language::Rust, 1, "fn main() {}");
    assert!(manager.unavailable.contains_key("rust"));
    // Without a server there is nothing to open: frames must not ask
    // for another text clone.
    for _ in 0..3 {
        assert!(!manager.sync_doc(&path, Language::Rust, 1));
    }
}

#[test]
fn debounce_fires_without_further_edits() {
    let mut manager = LspManager::new();
    let path = crate::test_path("/tmp/debounced.rs");
    tracked(&mut manager, &path);
    // Nothing new to send for the already-submitted revision.
    assert!(!manager.sync_doc(&path, Language::Rust, 1));
    // A fresh edit starts the delay...
    assert!(!manager.sync_doc(&path, Language::Rust, 2));
    // ...and frames with no further edits must not extend it:
    // backdated past the delay, the same revision submits.
    manager.docs.get_mut(&path).expect("doc").last_change = Instant::now()
        .checked_sub(SYNC_DEBOUNCE + Duration::from_secs(1))
        .expect("backdate");
    assert!(manager.sync_doc(&path, Language::Rust, 2));
}

/// Server startup and stdio must not need a Tokio runtime on the
/// calling thread: the GUI thread owns none, and spawning from it
/// panicked. Supervised startup only passes channels, so spawning a
/// (promptly exiting) server here must complete its lifecycle with
/// events, never a panic.
#[cfg(unix)]
#[test]
fn supervised_spawn_needs_no_caller_runtime() {
    assert!(
        tokio::runtime::Handle::try_current().is_err(),
        "regression needs a runtime-less thread"
    );
    let dir = tempfile::tempdir().expect("tempdir");
    let ctx = eframe::egui::Context::default();
    let (tx, _rx) = std::sync::mpsc::channel::<crate::Update>();
    let paths = terminator_core::Paths::at(dir.path().to_owned());
    let (services, _owner) = crate::gui_services::Services::new(paths, ctx, tx).expect("services");
    let mut manager = LspManager::new();
    manager.set_services(services);
    let key = ServerKey {
        language: "rust",
        root: dir.path().to_owned(),
    };
    // /bin/true can exit before initialize is queued. Both an immediate
    // None and a later Down event are valid; neither needs a caller runtime.
    let _ = manager.spawn_supervised(&key, Path::new("/bin/true"));
    let start = Instant::now();
    while manager.servers.contains_key(&key) {
        manager.poll();
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "exited server is reaped"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    manager.poll();
    assert_eq!(
        manager.failures.get(key.language),
        Some(&1),
        "the exited server is recorded exactly once"
    );
}

#[test]
fn server_resources_scope_by_root() {
    let first = ServerKey {
        language: "rust",
        root: PathBuf::from("/proj-a"),
    };
    let second = ServerKey {
        language: "rust",
        root: PathBuf::from("/proj-b"),
    };
    assert_ne!(server_resource(&first), server_resource(&second));
    assert_eq!(server_resource(&first), server_resource(&first.clone()));
}

/// Two projects, one language: the second server must admit
/// alongside the first. The supervisor runs a single active task
/// per key, so a language-only key parks the second spawn forever.
#[cfg(unix)]
#[test]
fn second_project_server_starts_alongside_first() {
    let dir = tempfile::tempdir().expect("tempdir");
    let ctx = eframe::egui::Context::default();
    let (tx, _rx) = std::sync::mpsc::channel::<crate::Update>();
    let paths = terminator_core::Paths::at(dir.path().to_owned());
    let (services, _owner) = crate::gui_services::Services::new(paths, ctx, tx).expect("services");
    let mut manager = LspManager::new();
    manager.set_services(services.clone());
    // `/bin/cat` stays alive on its piped stdin, so both actors are
    // long-lived: without root-scoped keys the second never admits.
    // (The supervisor's own processes actor is already admitted, so
    // the first server brings the count to two, the second to three.)
    let first = ServerKey {
        language: "rust",
        root: dir.path().join("proj-a"),
    };
    std::fs::create_dir_all(&first.root).expect("root");
    manager
        .spawn_supervised(&first, Path::new("/bin/cat"))
        .expect("spawn accepted");
    let start = Instant::now();
    while services.handle().diagnostics().actors < 2 {
        manager.poll();
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "first server admits"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let second = ServerKey {
        language: "rust",
        root: dir.path().join("proj-b"),
    };
    std::fs::create_dir_all(&second.root).expect("root");
    manager
        .spawn_supervised(&second, Path::new("/bin/cat"))
        .expect("spawn accepted");
    let start = Instant::now();
    while services.handle().diagnostics().actors < 3 {
        manager.poll();
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "second project's server admits alongside the first"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    manager.prune(&std::collections::HashSet::new());
    manager.poll();
}

/// Text submitted while the server starts must open at handshake
/// completion: `initialized` first, then the parked `didOpen`, with
/// no further submit needed.
#[test]
fn parked_text_opens_at_handshake_completion() {
    let mut manager = LspManager::new();
    let path = crate::test_path("/tmp/pending.rs");
    tracked(&mut manager, &path);
    let key = manager.docs.get(&path).expect("doc").server.clone();
    let (inbox, mut outbox) = tokio::sync::mpsc::unbounded_channel();
    manager.servers.insert(
        key.clone(),
        Server {
            inbox,
            child: None,
            cancel: None,
            ids: L::IdGen::default(),
            pending: HashMap::new(),
            init_id: Some(7),
            ready: false,
        },
    );
    let doc = manager.docs.get_mut(&path).expect("doc");
    doc.opened = false;
    doc.version = 0;
    doc.pending_text = Some("fn main() {}".into());
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 7,
        "result": {"capabilities": {}},
    })
    .to_string();
    manager.on_body(&key, body.as_bytes());
    let doc = manager.docs.get(&path).expect("doc");
    assert!(doc.opened);
    assert!(doc.pending_text.is_none());
    assert!(manager.servers.get(&key).is_some_and(|server| server.ready));
    // Inbox traffic is framed for the wire: decode before parsing.
    let mut decoder = L::FrameDecoder::default();
    let framed = outbox.try_recv().expect("initialized");
    let bodies = decoder.feed(&framed);
    assert_eq!(bodies.len(), 1);
    let first: serde_json::Value = serde_json::from_slice(&bodies[0]).expect("json");
    assert_eq!(
        first.get("method").and_then(|m| m.as_str()),
        Some("initialized")
    );
    let framed = outbox.try_recv().expect("didOpen");
    let bodies = decoder.feed(&framed);
    assert_eq!(bodies.len(), 1);
    let second: serde_json::Value = serde_json::from_slice(&bodies[0]).expect("json");
    assert_eq!(
        second.get("method").and_then(|m| m.as_str()),
        Some("textDocument/didOpen")
    );
    let document = second
        .get("params")
        .and_then(|params| params.get("textDocument"))
        .expect("document");
    assert_eq!(
        document.get("text").and_then(|text| text.as_str()),
        Some("fn main() {}")
    );
    assert_eq!(document.get("version").and_then(|v| v.as_i64()), Some(1));
    assert!(outbox.try_recv().is_err());
}

fn tracked(manager: &mut LspManager, path: &Path) {
    let root = project_root(path, Language::Rust);
    manager.docs.insert(
        path.to_owned(),
        OpenDoc {
            uri: L::file_uri(path).expect("uri"),
            version: 1,
            server: ServerKey {
                language: "rust",
                root,
            },
            language_id: "rust",
            submitted: 1,
            observed: 1,
            last_change: Instant::now(),
            opened: true,
            pending_text: None,
        },
    );
}

#[test]
fn rename_response_is_preview_only_and_rejects_stale_buffer() {
    let mut manager = LspManager::new();
    let path = crate::test_path("/tmp/rename.rs");
    tracked(&mut manager, &path);
    let key = manager.docs.get(&path).unwrap().server.clone();
    let (inbox, mut outbox) = tokio::sync::mpsc::unbounded_channel();
    manager.servers.insert(
        key.clone(),
        Server {
            inbox,
            child: None,
            cancel: None,
            ids: L::IdGen::default(),
            pending: HashMap::new(),
            init_id: None,
            ready: true,
        },
    );
    let at = L::Position {
        line: 0,
        character: 4,
    };
    manager.request_rename(&path, at, "bar").unwrap();
    let mut decoder = L::FrameDecoder::default();
    let body = decoder.feed(&outbox.try_recv().unwrap()).remove(0);
    let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(request["method"], "textDocument/rename");
    assert_eq!(request["params"]["newName"], "bar");
    let uri = L::file_uri(&path).unwrap().to_string();
    let response = serde_json::json!({
        "jsonrpc": "2.0", "id": request["id"],
        "result": { "changes": {
            (uri): [{
                "range": {
                    "start": {"line": 0, "character": 4},
                    "end": {"line": 0, "character": 7}
                },
                "newText": "bar"
            }]
        }}
    });
    manager.on_body(&key, response.to_string().as_bytes());
    let result = manager.take_rename().unwrap();
    assert_eq!(result.new_name, "bar");
    assert_eq!(result.edits.unwrap()[0].edits[0].new_text, "bar");

    manager.request_rename(&path, at, "baz").unwrap();
    manager.docs.get_mut(&path).unwrap().observed = 2;
    let body = decoder.feed(&outbox.try_recv().unwrap()).remove(0);
    let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let response = serde_json::json!({"jsonrpc": "2.0", "id": request["id"], "result": null});
    manager.on_body(&key, response.to_string().as_bytes());
    assert!(manager.take_rename().unwrap().edits.is_err());
}

#[test]
fn workspace_symbol_results_follow_the_latest_query() {
    let mut manager = LspManager::new();
    let path = crate::test_path("/tmp/symbols.rs");
    tracked(&mut manager, &path);
    let key = manager.docs.get(&path).unwrap().server.clone();
    let (inbox, mut outbox) = tokio::sync::mpsc::unbounded_channel();
    manager.servers.insert(
        key.clone(),
        Server {
            inbox,
            child: None,
            cancel: None,
            ids: L::IdGen::default(),
            pending: HashMap::new(),
            init_id: None,
            ready: true,
        },
    );
    manager.request_scoped_symbols("old", None);
    assert!(manager.workspace_symbols_pending());
    let mut decoder = L::FrameDecoder::default();
    let request = decoder.feed(&outbox.try_recv().unwrap()).remove(0);
    let request: serde_json::Value = serde_json::from_slice(&request).unwrap();
    assert_eq!(request["method"], "workspace/symbol");
    manager.request_scoped_symbols("new", None);
    let newer = decoder.feed(&outbox.try_recv().unwrap()).remove(0);
    let newer: serde_json::Value = serde_json::from_slice(&newer).unwrap();
    let location = serde_json::json!({
        "uri": L::file_uri(&path).unwrap().as_str(),
        "range": {"start": {"line": 2, "character": 0}, "end": {"line": 2, "character": 3}}
    });
    let old = serde_json::json!({
        "jsonrpc": "2.0", "id": request["id"],
        "result": [{"name": "old", "kind": 12, "location": location.clone()}]
    });
    manager.on_body(&key, old.to_string().as_bytes());
    assert!(manager.workspace_symbols().is_empty());
    let new = serde_json::json!({
        "jsonrpc": "2.0", "id": newer["id"],
        "result": [{"name": "new", "kind": 12, "location": location}]
    });
    manager.on_body(&key, new.to_string().as_bytes());
    assert_eq!(manager.workspace_symbols()[0].name, "new");
    assert!(!manager.workspace_symbols_pending());
}

#[test]
fn diagnostics_route_to_tracked_docs_only() {
    let mut manager = LspManager::new();
    let path = crate::test_path("/tmp/routed.rs");
    tracked(&mut manager, &path);
    let key = ServerKey {
        language: "rust",
        root: project_root(&path, Language::Rust),
    };
    let params = serde_json::json!({
        "uri": L::file_uri(&path).unwrap().as_str(),
        "diagnostics": [{
            "range": {"start": {"line": 0, "character": 0},
                      "end": {"line": 0, "character": 1}},
            "severity": 1,
            "message": "boom",
        }],
    });
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": params,
    })
    .to_string();
    manager.on_body(&key, body.as_bytes());
    assert_eq!(manager.problems_for(&path).len(), 1);
    // Unknown files never accumulate state.
    let unknown = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": {"uri": L::file_uri(&crate::test_path("/tmp/ghost.rs")).unwrap().as_str(), "diagnostics": []},
    })
    .to_string();
    manager.on_body(&key, unknown.as_bytes());
    assert!(
        manager
            .problems_for(&crate::test_path("/tmp/ghost.rs"))
            .is_empty()
    );
}

#[test]
fn prune_closes_docs_and_clears_state() {
    let mut manager = LspManager::new();
    let path = crate::test_path("/tmp/gone.rs");
    tracked(&mut manager, &path);
    manager.diagnostics.insert(
        path.clone(),
        vec![L::Diagnostic {
            range: L::Range {
                start: L::Position {
                    line: 0,
                    character: 0,
                },
                end: L::Position {
                    line: 0,
                    character: 1,
                },
            },
            severity: None,
            message: "x".into(),
        }],
    );
    manager.prune(&std::collections::HashSet::new());
    assert!(manager.problems_for(&path).is_empty());
    assert!(manager.row_marks(&path).is_empty());
}

/// Full round trip against the real TypeScript server when
/// installed; self-skips otherwise.
#[tokio::test]
async fn live_typescript_server_diagnostics_and_symbols() {
    if terminator_core::find_executable("typescript-language-server").is_none() {
        eprintln!("SKIP: typescript-language-server not installed");
        return;
    }
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("proj");
    std::fs::create_dir_all(&root).expect("mkdir");
    std::fs::write(
        root.join("package.json"),
        "{\"name\":\"proj\",\"version\":\"0.0.0\"}",
    )
    .expect("package");
    std::fs::write(
        root.join("tsconfig.json"),
        "{\"compilerOptions\":{\"strict\":true}}",
    )
    .expect("tsconfig");
    let source = "export const bad: number = \"oops\";\nexport function ok() { return 1; }\n";
    let file = root.join("index.ts");
    std::fs::write(&file, source).expect("source");

    let mut manager = LspManager::new();
    assert!(manager.sync_doc(&file, Language::TypeScript, 1));
    manager.submit_text(&file, Language::TypeScript, 1, source);
    let start = Instant::now();
    while manager.problems_for(&file).is_empty() {
        if let Some(note) = manager.server_note(&file)
            && note.starts_with("Language server failed to start")
        {
            eprintln!("SKIP: {note}");
            return;
        }
        assert!(
            start.elapsed() < Duration::from_mins(1),
            "no diagnostics; note={:?}",
            manager.server_note(&file)
        );
        manager.poll();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let problems = manager.problems_for(&file);
    assert!(
        problems
            .iter()
            .any(|item| item.severity == Some(L::Severity::Error)),
        "type error reported: {problems:?}"
    );

    let start = Instant::now();
    while manager.symbols_for(&file).is_empty() {
        assert!(start.elapsed() < Duration::from_secs(30), "no symbols");
        manager.request_symbols(&file);
        manager.poll();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let names: Vec<String> = manager
        .symbols_for(&file)
        .iter()
        .map(|symbol| symbol.name.clone())
        .collect();
    assert!(names.iter().any(|name| name == "ok"), "{names:?}");
    manager.prune(&std::collections::HashSet::new());
}

/// Full round trip against the real rust-analyzer when installed;
/// self-skips otherwise (live-provider verification, not CI-gated).
#[tokio::test]
async fn live_rust_analyzer_diagnostics() {
    let path_env = std::env::var_os("PATH")
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    if L::find_server(&["rust-analyzer"], &path_env).is_none() {
        eprintln!("SKIP: rust-analyzer not on PATH");
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
    let main = "fn main() {\n    let bad: i32 = \"oops\";\n}\n";
    let file = root.join("src/main.rs");
    std::fs::write(&file, main).expect("main");

    let mut manager = LspManager::new();
    assert!(manager.sync_doc(&file, Language::Rust, 1));
    manager.submit_text(&file, Language::Rust, 1, main);
    // Handshake + analysis need seconds; poll until diagnostics land.
    let start = Instant::now();
    while manager.problems_for(&file).is_empty() {
        assert!(start.elapsed() < Duration::from_secs(90), "no diagnostics");
        manager.poll();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let problems = manager.problems_for(&file);
    // Error severity at the `bad` binding (line 1), not message
    // text: wording varies across server versions, but a type error
    // there is the point of the fixture.
    assert!(
        problems
            .iter()
            .any(|item| item.severity == Some(L::Severity::Error) && item.range.start.line == 1),
        "type error reported at line 1: {:?}",
        problems
            .iter()
            .map(|item| (&item.message, item.range.start.line))
            .collect::<Vec<_>>()
    );
    assert!(!manager.row_marks(&file).is_empty());
    // Hover the `bad` binding (line 1, chars 8..11), retrying the
    // request like a user pressing K again: early answers are null
    // until analysis settles.
    let start = Instant::now();
    let mut hover = None;
    while hover.is_none() {
        assert!(start.elapsed() < Duration::from_mins(1), "no hover");
        manager.request_hover(
            &file,
            L::Position {
                line: 1,
                character: 9,
            },
            (1, 8),
        );
        for _ in 0..10 {
            manager.poll();
            hover = manager.hover_popup(&file, (1, 8));
            if hover.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    assert!(
        hover.as_ref().is_some_and(|(text, _)| text.contains("i32")),
        "hover: {hover:?}"
    );
    manager.prune(&std::collections::HashSet::new());
}

/// Fake ready server so symbol requests have an inbox to land in.
fn ready_server(
    manager: &mut LspManager,
    path: &Path,
) -> (ServerKey, tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) {
    let key = manager.docs.get(path).expect("doc").server.clone();
    let (inbox, outbox) = tokio::sync::mpsc::unbounded_channel();
    manager.servers.insert(
        key.clone(),
        Server {
            inbox,
            child: None,
            cancel: None,
            ids: L::IdGen::default(),
            pending: HashMap::new(),
            init_id: None,
            ready: true,
        },
    );
    (key, outbox)
}

#[test]
fn empty_outline_retries_on_a_delay_instead_of_caching_forever() {
    let mut manager = LspManager::new();
    let path = crate::test_path("/tmp/outline.rs");
    tracked(&mut manager, &path);
    let (key, mut sent) = ready_server(&mut manager, &path);

    manager.request_symbols(&path);
    assert!(sent.try_recv().is_ok(), "first request goes out");
    let id = manager
        .servers
        .get(&key)
        .expect("server")
        .pending
        .keys()
        .copied()
        .next()
        .expect("pending symbols");
    manager.on_response(
        &key,
        L::RequestId::Number(id),
        Some(serde_json::json!([])),
        None,
    );
    assert!(manager.symbols_for(&path).is_empty());

    // Immediately after an empty answer the gate holds...
    manager.request_symbols(&path);
    assert!(sent.try_recv().is_err(), "no per-frame spam while indexing");
    // ...and releases once the delay passed.
    manager.symbols_retry.insert(
        path.clone(),
        Instant::now().checked_sub(SYMBOLS_RETRY).expect("backdate"),
    );
    manager.request_symbols(&path);
    assert!(sent.try_recv().is_ok(), "indexing retry asks again");
}

#[test]
fn restart_clears_unavailable_and_reopens_documents() {
    let mut manager = LspManager::new();
    let path = crate::test_path("/tmp/retry.rs");
    tracked(&mut manager, &path);
    let _ = ready_server(&mut manager, &path);
    manager
        .unavailable
        .insert("rust", "No language server".into());
    manager.failures.insert("rust", MAX_CRASHES);

    manager.restart(&path);

    assert!(manager.unavailable.is_empty());
    assert!(manager.failures.is_empty());
    assert!(!manager.docs.get(&path).expect("doc").opened);
}

#[test]
fn failed_initialize_reports_the_server_message() {
    let mut manager = LspManager::new();
    let path = crate::test_path("/tmp/app.ts");
    tracked(&mut manager, &path);
    let (key, _sent) = ready_server(&mut manager, &path);
    manager.servers.get_mut(&key).expect("server").init_id = Some(7);

    manager.on_response(
        &key,
        L::RequestId::Number(7),
        None,
        Some(L::ResponseError {
            code: -32603,
            message: "Could not find a valid TypeScript installation.".into(),
        }),
    );

    let note = manager.server_note(&path).expect("note");
    assert!(note.contains("Could not find a valid TypeScript"), "{note}");
    assert!(!manager.servers.contains_key(&key));
}

#[test]
fn saved_server_path_restarts_only_that_language() {
    let mut manager = LspManager::new();
    let path = crate::test_path("/tmp/live.rs");
    tracked(&mut manager, &path);
    let (key, _sent) = ready_server(&mut manager, &path);
    let mut overrides = std::collections::BTreeMap::new();
    overrides.insert("rust".to_owned(), " /opt/rust-analyzer ".to_owned());

    manager.set_server_overrides(&overrides);

    assert!(!manager.servers.contains_key(&key), "rust server restarts");
    assert!(!manager.docs.get(&path).expect("doc").opened);
    assert_eq!(
        manager.overrides.get("rust").map(String::as_str),
        Some("/opt/rust-analyzer"),
        "path is trimmed"
    );

    // Unrelated settings saves leave the live server alone.
    let (key, _sent) = ready_server(&mut manager, &path);
    manager.set_server_overrides(&overrides);
    assert!(manager.servers.contains_key(&key));
}
