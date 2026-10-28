use super::*;
use serde_json::json;

#[test]
fn empty_queries_are_distinct_from_login_and_permission_errors() {
    let empty = br#"{"code":"error","severity":2,"generic":17,"data":"File(s) not opened on this client."}"#;
    assert!(records(empty, true).unwrap().is_empty());
    assert!(records(empty, false).is_err());
    for generic in [6, 3] {
        let error = json!({"severity":3,"generic":generic,"data":"Access denied"}).to_string();
        assert!(records(error.as_bytes(), true).is_err());
    }
    assert!(records(b"not json", true).is_err());
    assert!(records(b"[]", true).is_err());
    assert!(records(br#"{"code":"error","data":"Failure"}"#, true).is_err());
    assert!(records(br#"{"severity":3,"generic":17,"data":"Failure"}"#, true).is_err());
}

#[test]
fn opened_changelists_override_preview_and_keep_move_sides() {
    let root = Path::new("/workspace");
    let opened = vec![
        json!({"clientFile":"/workspace/new name.rs","action":"move/add","change":"42"}),
        json!({"clientFile":"/workspace/old.rs","action":"move/delete","change":"42"}),
    ];
    let preview = vec![
        json!({"depotFile":"new name.rs","action":"add"}),
        json!({"depotFile":"offline.rs","action":"edit"}),
        json!({"clientFile":"/outside/file.rs","action":"edit"}),
        json!({"clientFile":"/workspace/../outside.rs","action":"edit"}),
        json!({"depotFile":"//depot/not-a-local-path","action":"edit"}),
    ];
    let files = parse_files(root, &opened, &preview);
    assert_eq!(files.len(), 3);
    let added = files.iter().find(|file| file.action == "move/add").unwrap();
    assert!(added.opened);
    assert_eq!(added.change, "42");
    let offline = files.iter().find(|file| !file.opened).unwrap();
    assert_eq!(offline.path, root.join("offline.rs"));
}

#[test]
fn file_specs_escape_revision_and_wildcard_characters() {
    assert_eq!(
        filespec(Path::new("/workspace/a@b#c%d*.txt")).unwrap(),
        "/workspace/a%40b%23c%25d%2A.txt"
    );
    assert!(filespec(Path::new("/workspace/.../all")).is_err());
    assert!(filespec(Path::new("/workspace/\u{fffd}.txt")).is_err());
}

/// An isolated Perforce server workspace and the GUI services that query it.
struct P4Fixture {
    service: Services,
    _owner: crate::gui_services::Owner,
    root: PathBuf,
    command_root: PathBuf,
    state: tempfile::TempDir,
    cancel: CancellationToken,
}

impl P4Fixture {
    async fn new() -> Self {
        let root = PathBuf::from(std::env::var_os("TERMINATOR_P4_FIXTURE").unwrap())
            .canonicalize()
            .unwrap();
        let port = std::env::var("P4PORT").unwrap();
        assert!(port.starts_with("rsh:"));
        assert!(port.contains("terminator-p4."));
        let state = tempfile::tempdir().unwrap();
        let (tx, _) = std::sync::mpsc::channel();
        let (service, owner) = Services::new(
            terminator_core::Paths::at(state.path().into()),
            eframe::egui::Context::default(),
            tx,
        )
        .unwrap();
        let cancel = CancellationToken::new();
        let context = crate::services::context_async(&service, root.clone(), true, &cancel).await;
        assert!(context.error.is_none(), "{:?}", context.error);
        let workspace = context.perforce.unwrap();
        assert_eq!(workspace.client, "terminator-fixture");
        assert_eq!(workspace.files.len(), 4);
        assert_eq!(workspace.files.iter().filter(|file| file.opened).count(), 2);
        assert!(!workspace.history.is_empty());
        Self {
            service,
            _owner: owner,
            root,
            command_root: workspace.command_root,
            state,
            cancel,
        }
    }

    async fn perform(&self, cwd: &Path, op: Op) -> Result<crate::workspace_ops::Report> {
        perform(&self.service, cwd, op, &self.cancel).await
    }

    /// Runs a raw p4 command from the client root; it must succeed.
    async fn p4(&self, args: &[String]) {
        run(&self.service, &self.command_root, args, true)
            .await
            .unwrap();
    }

    fn spec(&self, name: &str) -> String {
        filespec(&self.command_root.join(name)).unwrap()
    }

    async fn reopen_new_file(&self, number: &str) {
        self.p4(&[
            "reopen".into(),
            "-c".into(),
            number.into(),
            self.spec("new file.txt"),
        ])
        .await;
    }

    async fn assert_have_and_workspace_text(&self) {
        for (name, before, after) in [
            ("edit.txt", "original\n", "opened edit\n"),
            ("delete.txt", "original\n", ""),
            ("new file.txt", "", "new file\n"),
            ("special@file#1.txt", "original\n", "original\n"),
        ] {
            let doc = crate::diff::document_async(crate::diff::DocumentAsyncInput {
                service: &self.service,
                cwd: self.command_root.clone(),
                path: self.root.join(name),
                staged: false,
                cancel: &self.cancel,
            })
            .await
            .unwrap();
            assert_eq!(doc.left_text, before, "{name}");
            assert_eq!(doc.right_text, after, "{name}");
            assert!(doc.source.is_none());
        }
    }

    async fn assert_open_actions(&self) {
        let root = &self.root;
        self.perform(&self.command_root, Op::Reconcile(root.join("new file.txt")))
            .await
            .unwrap();
        self.perform(
            &self.command_root,
            Op::Edit(root.join("special@file#1.txt")),
        )
        .await
        .unwrap();
        assert!(
            self.perform(root, Op::Edit(self.state.path().join("outside.txt")))
                .await
                .is_err()
        );
        assert!(
            self.perform(&self.command_root, Op::Submit("--help".into()))
                .await
                .is_err()
        );
    }

    /// Creates a numbered changelist with a description.
    async fn new_changelist(&self) -> String {
        use std::io::Write as _;
        let mut child = Command::new("p4")
            .current_dir(&self.root)
            .args(["change", "-i"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(b"Change: new\nClient: terminator-fixture\nUser: fixture\nDescription:\n\tFixture change\n").unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let changes = query(
            &self.service,
            &self.command_root,
            &["changes", "-s", "pending", "-c", "terminator-fixture"],
            false,
        )
        .await
        .unwrap();
        field(changes.first().unwrap(), "change").to_string()
    }

    async fn shelve_unshelve_and_submit(&self, number: String) {
        let command_root = &self.command_root;
        self.reopen_new_file(&number).await;
        self.perform(command_root, Op::Shelve(number.clone()))
            .await
            .unwrap();
        self.p4(&["revert".into(), self.spec("new file.txt")]).await;
        std::fs::remove_file(self.root.join("new file.txt")).unwrap();
        self.perform(command_root, Op::Unshelve(number.clone()))
            .await
            .unwrap();
        // Remove the shelf, then submit the same numbered changelist.
        self.reopen_new_file(&number).await;
        self.p4(&["shelve".into(), "-d".into(), "-c".into(), number.clone()])
            .await;
        self.perform(command_root, Op::Submit(number))
            .await
            .unwrap();
        self.perform(command_root, Op::Sync).await.unwrap();
    }

    async fn assert_unreviewable_paths_fail(&self) {
        // A failed raw print must not become an empty left-hand snapshot.
        assert!(
            run(
                &self.service,
                &self.command_root,
                &["print".into(), "-q".into(), "//depot/missing.txt#1".into()],
                false,
            )
            .await
            .is_err()
        );
        // Excluded client paths are not untracked additions, even inside the root.
        let excluded = self.root.join("excluded");
        std::fs::create_dir(&excluded).unwrap();
        let path = excluded.join("local.txt");
        std::fs::write(&path, "not in the client view\n").unwrap();
        assert!(
            document(DocumentInput {
                service: &self.service,
                cwd: self.command_root.clone(),
                path,
                context: 3,
                cancel: &self.cancel
            })
            .await
            .is_err()
        );
        // This cwd is outside the client; Git-to-Perforce fallback must also fail.
        let path = self.state.path().join("unrelated.txt");
        std::fs::write(&path, "unrelated\n").unwrap();
        assert!(
            crate::diff::document_async(crate::diff::DocumentAsyncInput {
                service: &self.service,
                cwd: self.state.path().into(),
                path,
                staged: false,
                cancel: &self.cancel
            })
            .await
            .is_err()
        );
    }
}

/// Run through scripts/test-perforce.sh; never connects to the user's server.
#[tokio::test]
#[ignore = "requires an isolated Perforce server; use scripts/test-perforce.sh"]
async fn isolated_server() {
    let fixture = P4Fixture::new().await;
    fixture.assert_have_and_workspace_text().await;
    fixture.assert_open_actions().await;
    let number = fixture.new_changelist().await;
    fixture.shelve_unshelve_and_submit(number).await;
    fixture.assert_unreviewable_paths_fail().await;
}
