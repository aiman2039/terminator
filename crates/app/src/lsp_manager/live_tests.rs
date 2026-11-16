//! Live round trips against real language servers; each self-skips
//! when its server is not installed.

use super::*;
use crate::lsp_manager::SubmitTextInput;

/// Full round trip against the real TypeScript server when
/// installed; self-skips otherwise.
#[tokio::test]
async fn live_typescript_server_diagnostics_and_symbols() {
    if terminator_core::find_executable("typescript-language-server").is_none() {
        crate::console::Console::err(format_args!(
            "SKIP: typescript-language-server not installed"
        ));
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

    let mut manager = LspManager::trusting();
    assert!(manager.sync_doc(&file, Language::TypeScript, 1));
    manager.submit_text(SubmitTextInput {
        path: &file,
        language: Language::TypeScript,
        revision: 1,
        text: source,
    });
    let start = Instant::now();
    while manager.problems_for(&file).is_empty() {
        if let Some(note) = manager.server_note(&file)
            && note.starts_with("Language server failed to start")
        {
            crate::console::Console::err(format_args!("SKIP: {note}"));
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
    let main = "fn main() {\n    let bad: i32 = \"oops\";\n}\n";
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
    // Handshake + analysis need seconds; poll until diagnostics land.
    let start = Instant::now();
    while manager.problems_for(&file).is_empty() {
        assert!(
            start.elapsed() < Duration::from_secs(90),
            "no diagnostics; note={:?}",
            manager.server_note(&file)
        );
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
    // Hover the `bad` binding (line 1, chars 8..11). Early answers are
    // null until analysis settles; a new key (as after an edit) asks
    // again, since an answer stays with the key it came for.
    let start = Instant::now();
    let mut hover = None;
    for revision in 1_u64.. {
        assert!(start.elapsed() < Duration::from_mins(1), "no hover");
        let key = HoverKey {
            path: file.clone(),
            revision,
            line: 1,
            start: 8,
            end: 11,
        };
        let at = L::Position {
            line: 1,
            character: 9,
        };
        assert_eq!(manager.request_hover(&key, at), Ok(true));
        for _ in 0..10 {
            manager.poll();
            hover = manager.hover_docs(&key).flatten();
            if hover.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if hover.is_some() {
            break;
        }
    }
    assert!(
        hover.as_ref().is_some_and(|docs| docs.text.contains("i32")),
        "hover: {hover:?}"
    );
    manager.prune(&std::collections::HashSet::new());
}

/// Live completion round trip against rust-analyzer (self-skips without
/// it): method completion after `v.` lists `len`.
#[tokio::test]
async fn live_rust_analyzer_completion() {
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
    let main = "fn main() {\n    let v: Vec<i32> = Vec::new();\n    v.le\n}\n";
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
        line: 2,
        character: 8,
    };
    let start = Instant::now();
    let mut labels: Vec<String> = Vec::new();
    while !labels.iter().any(|label| label.starts_with("len")) {
        assert!(
            start.elapsed() < Duration::from_secs(90),
            "no completion: {labels:?}"
        );
        if manager
            .request_completion(&file, at, L::CompletionTrigger::Invoked)
            .is_none()
        {
            manager.poll();
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        }
        for _ in 0..20 {
            manager.poll();
            if let Some(results) = manager.take_completion() {
                labels = results.items.into_iter().map(|item| item.label).collect();
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    manager.prune(&std::collections::HashSet::new());
}

/// Live signature help against rust-analyzer (self-skips without it).
#[tokio::test]
async fn live_rust_analyzer_signature_help() {
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
    let main =
        "fn add(left: i32, right: i32) -> i32 { left + right }\nfn main() {\n    add(1, \n}\n";
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
        line: 2,
        character: 11,
    };
    let start = Instant::now();
    let mut found = None;
    while found.is_none() {
        assert!(start.elapsed() < Duration::from_secs(90), "no signature");
        if manager.request_signature_help(&file, at).is_none() {
            manager.poll();
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        }
        for _ in 0..20 {
            manager.poll();
            if let Some((_, _, help)) = manager.take_signature() {
                found = help;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    let help = found.expect("signature");
    let active: String = help
        .label
        .chars()
        .skip(help.active.as_ref().map_or(0, |range| range.start))
        .take(help.active.as_ref().map_or(0, |range| range.len()))
        .collect();
    assert_eq!(active, "right: i32", "{help:?}");
    manager.prune(&std::collections::HashSet::new());
}

/// Live formatting and code actions against rust-analyzer (self-skips):
/// formatting proposes edits, and "Insert explicit type" resolves to an
/// edit adding `: i32`.
#[tokio::test]
async fn live_rust_analyzer_format_and_code_action() {
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
    let main = "fn main() {\n    let x = 1;\n    let _ = x;\n}\n";
    let file = root.join("src/main.rs");
    std::fs::write(&file, main).expect("main");
    let mut manager = LspManager::trusting();
    assert!(manager.sync_doc(&file, Language::Rust, 1));
    manager.submit_text(SubmitTextInput {
        path: &file,
        language: Language::Rust,
        revision: 1,
        text: "fn main(){\n    let x=1;\n    let _ = x;\n}\n",
    });
    let start = Instant::now();
    // Formatting the cramped text proposes edits.
    let mut formatted = None;
    while formatted.is_none() {
        assert!(start.elapsed() < Duration::from_secs(90), "no formatting");
        if !manager.request_format(&file, 1) {
            manager.poll();
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        }
        for _ in 0..30 {
            manager.poll();
            if let Some((_, _, edits)) = manager.edit_answers.format.take() {
                formatted = Some(edits);
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    assert!(!formatted.unwrap_or_default().is_empty(), "format edits");
    manager.submit_text(SubmitTextInput {
        path: &file,
        language: Language::Rust,
        revision: 2,
        text: main,
    });
    let at = L::Position {
        line: 1,
        character: 8,
    };
    let range = L::Range { start: at, end: at };
    let mut action = None;
    while action.is_none() {
        assert!(start.elapsed() < Duration::from_secs(90), "no code action");
        let Some(_) = manager.request_code_actions(&file, range) else {
            manager.poll();
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        };
        for _ in 0..30 {
            manager.poll();
            if let Some((_, _, items)) = manager.edit_answers.actions.take() {
                action = items
                    .into_iter()
                    .find(|item| item.title.to_lowercase().contains("explicit type"));
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    let action = action.expect("explicit type action");
    let edits = if let Ok(edits) = L::code_action_edits(&action.raw) {
        edits
    } else {
        {
            // Resolution can answer without the edit while analysis is
            // still settling; the editor re-asks, and so does this test.
            let mut edits = None;
            while edits.is_none() {
                assert!(start.elapsed() < Duration::from_secs(90), "no resolve");
                assert!(manager.request_resolve(&file, &action.raw));
                let mut resolved = None;
                while resolved.is_none() {
                    assert!(start.elapsed() < Duration::from_secs(90), "no resolve");
                    manager.poll();
                    resolved = manager.edit_answers.resolved.take();
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                edits = resolved.and_then(|(_, action)| L::code_action_edits(&action).ok());
            }
            edits.unwrap_or_default()
        }
    };
    let inserted: String = edits
        .iter()
        .flat_map(|file| file.edits.iter().map(|edit| edit.new_text.clone()))
        .collect();
    assert!(inserted.contains("i32"), "{inserted:?}");
    manager.prune(&std::collections::HashSet::new());
}

/// Live inlay hints against rust-analyzer (self-skips): an inferred
/// `let` gets a `: i32` type hint right after its name, and a call gets
/// parameter-name hints.
#[tokio::test]
async fn live_rust_analyzer_inlay_hints() {
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
    let main = "fn add(left: i32, right: i32) -> i32 { left + right }\nfn main() {\n    let total = add(1, 2);\n    let _ = total;\n}\n";
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
    let start = Instant::now();
    let hints = loop {
        assert!(start.elapsed() < Duration::from_secs(90), "no inlay hints");
        manager.request_inlay_hints(&file, 5);
        manager.poll();
        if let Some((revision, hints)) = manager.inlay_hints(&file)
            && hints.iter().any(|hint| hint.label.contains("i32"))
        {
            assert_eq!(revision, 1);
            break hints;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let typed = hints
        .iter()
        .find(|hint| hint.label.contains("i32"))
        .expect("type hint");
    assert_eq!(
        typed.position,
        L::Position {
            line: 2,
            character: 13
        },
        "{hints:?}"
    );
    assert!(
        hints.iter().any(|hint| hint.label.contains("left")),
        "{hints:?}"
    );
    manager.prune(&std::collections::HashSet::new());
}

/// Live call hierarchy against rust-analyzer (self-skips): `add` is called
/// by `main`, and `main` calls `add`.
#[tokio::test]
async fn live_rust_analyzer_call_hierarchy() {
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
    let main = "fn add(left: i32, right: i32) -> i32 { left + right }\nfn main() {\n    let _ = add(1, 2);\n}\n";
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
    let mut live = LiveHierarchy {
        manager: &mut manager,
        file: &file,
        start: Instant::now(),
    };
    // Prepare until the server is ready and names the function.
    let (generation, root_item) = live
        .first_root(L::Position {
            line: 0,
            character: 4,
        })
        .await;
    assert_eq!(root_item.name, "add");
    for (direction, expected) in [
        (L::HierarchyDirection::Up, "main"),
        (L::HierarchyDirection::Down, "add"),
    ] {
        // Callees start from `main`: prepare it the same way.
        let item = if direction == L::HierarchyDirection::Up {
            root_item.clone()
        } else {
            live.root(L::Position {
                line: 1,
                character: 4,
            })
            .await
        };
        let items = live.children(&item, direction, generation).await;
        assert_eq!(
            items
                .iter()
                .map(|item| item.name.as_str())
                .collect::<Vec<_>>(),
            [expected],
            "{direction:?}"
        );
    }
    manager.prune(&std::collections::HashSet::new());
}

/// Polling helpers for the live call hierarchy scenario.
struct LiveHierarchy<'a> {
    manager: &'a mut LspManager,
    file: &'a Path,
    start: Instant,
}

impl LiveHierarchy<'_> {
    /// Prepare at `position` until the server is ready and answers a root.
    async fn first_root(&mut self, position: L::Position) -> (u64, L::HierarchyItem) {
        loop {
            assert!(
                self.start.elapsed() < Duration::from_secs(90),
                "no hierarchy"
            );
            self.manager.poll();
            let Ok(generation) =
                self.manager
                    .request_hierarchy(self.file, position, L::HierarchyKind::Call)
            else {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            };
            if let Some(item) = self.poll_roots(generation).await {
                return (generation, item);
            }
        }
    }

    /// Up to 50 polls for the roots answer to `generation`.
    async fn poll_roots(&mut self, generation: u64) -> Option<L::HierarchyItem> {
        for _ in 0..50 {
            self.manager.poll();
            if let Some(HierarchyAnswer::Roots {
                generation: g,
                items,
            }) = self.manager.hierarchy_answers.pop()
                && g == generation
            {
                return items.into_iter().next();
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        None
    }

    /// Prepare answers empty too while indexing: ask again until a root.
    async fn root(&mut self, position: L::Position) -> L::HierarchyItem {
        loop {
            let generation = self
                .manager
                .request_hierarchy(self.file, position, L::HierarchyKind::Call)
                .expect("ready");
            if let Some(item) = self.wait_roots(generation).await {
                return item;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    /// Poll until the roots answer to `generation`; its first item.
    async fn wait_roots(&mut self, generation: u64) -> Option<L::HierarchyItem> {
        loop {
            assert!(
                self.start.elapsed() < Duration::from_secs(90),
                "no main root"
            );
            self.manager.poll();
            if let Some(HierarchyAnswer::Roots {
                generation: g,
                items,
            }) = self.manager.hierarchy_answers.pop()
                && g == generation
            {
                return items.into_iter().next();
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// An indexing server answers empty at first: ask until it knows.
    async fn children(
        &mut self,
        item: &L::HierarchyItem,
        direction: L::HierarchyDirection,
        generation: u64,
    ) -> Vec<L::HierarchyItem> {
        loop {
            assert!(
                self.manager
                    .request_hierarchy_children(RequestHierarchyChildrenInput {
                        path: self.file,
                        item: &item.raw,
                        kind: L::HierarchyKind::Call,
                        direction,
                        generation,
                        node: 0
                    })
            );
            let items = self.wait_children().await;
            if !items.is_empty() {
                return items;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }

    async fn wait_children(&mut self) -> Vec<L::HierarchyItem> {
        loop {
            assert!(
                self.start.elapsed() < Duration::from_secs(90),
                "no children"
            );
            self.manager.poll();
            if let Some(HierarchyAnswer::Children { items, .. }) =
                self.manager.hierarchy_answers.pop()
            {
                return items;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}
