use super::*;
use crate::diff::ExpandGapInput;
use crate::gui_services::Owner;

impl Source {
    /// A tracked regular file `path` (relative) in the worktree `root`, for
    /// view tests that need a document with hunk actions.
    pub(crate) fn fixture(root: &Path, path: &Path) -> Self {
        Self {
            root: root.into(),
            path: path.into(),
            index: b"100644 0000000000000000000000000000000000000000 0\tfile".to_vec(),
            head: Vec::new(),
            exists: true,
            mode: 0o100644,
        }
    }
}

impl Undo {
    /// The undo of a change to `path` (relative) in the worktree `root`:
    /// of an index change when `index`, else of a working-tree one.
    pub(crate) fn fixture(root: &Path, path: &Path, index: bool) -> Self {
        let version = Version {
            text: String::new(),
            exists: true,
            mode: 0o100644,
        };
        Self {
            root: root.into(),
            path: path.into(),
            index,
            before: version.clone(),
            after: version,
            label: if index {
                "Stage hunk 1"
            } else {
                "Discard hunk 1"
            }
            .into(),
        }
    }
}

struct Fixture {
    service: Services,
    _owner: Owner,
    dir: tempfile::TempDir,
    path: PathBuf,
}

impl Fixture {
    fn new(path: &str, initial: Option<&str>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (updates, _) = std::sync::mpsc::channel();
        let (service, owner) = Services::new(
            terminator_core::Paths::at(dir.path().join("state")),
            eframe::egui::Context::default(),
            updates,
        )
        .unwrap();
        let fixture = Self {
            service,
            _owner: owner,
            dir,
            path: path.into(),
        };
        fixture.git(&["init", "-q"]);
        fixture.git(&["config", "user.email", "fixture@example.invalid"]);
        fixture.git(&["config", "user.name", "Fixture"]);
        fixture.git(&["config", "commit.gpgsign", "false"]);
        fixture.git(&["config", "core.autocrlf", "false"]);
        fixture.git(&["config", "core.hooksPath", ".no-fixture-hooks"]);
        if let Some(initial) = initial {
            std::fs::write(fixture.absolute(), initial).unwrap();
            fixture.git(&["add", "--", path]);
            fixture.git(&["commit", "-qm", "fixture"]);
        }
        fixture
    }

    fn absolute(&self) -> PathBuf {
        self.dir.path().join(&self.path)
    }

    fn git(&self, args: &[&str]) -> Vec<u8> {
        terminator_git::run_blocking(self.dir.path(), args, CommandOptions::default()).unwrap()
    }

    fn index(&self) -> String {
        String::from_utf8(self.git(&["show", &format!(":{}", self.path.to_str().unwrap())]))
            .unwrap()
    }

    async fn doc(&self, staged: bool) -> Arc<DiffDocument> {
        Arc::new(
            super::super::document_async(super::super::DocumentAsyncInput {
                service: &self.service,
                cwd: self.dir.path().into(),
                path: self.absolute(),
                staged,
                cancel: &CancellationToken::new(),
            })
            .await
            .unwrap(),
        )
    }

    async fn change(&self, doc: Arc<DiffDocument>, hunk: usize, kind: Kind) -> Result<Outcome> {
        execute(ExecuteInput {
            service: &self.service,
            request: Request::Hunk {
                document: doc,
                hunk,
                kind,
                lines: None,
            },
            staged: kind == Kind::Unstage,
            cancel: &CancellationToken::new(),
        })
        .await
    }

    async fn undo(&self, undo: Undo) -> Result<Outcome> {
        execute(ExecuteInput {
            service: &self.service,
            request: Request::Undo(undo),
            staged: false,
            cancel: &CancellationToken::new(),
        })
        .await
    }
}

fn original() -> String {
    {
        use std::fmt::Write;
        (0..30).fold(String::new(), |mut text, line| {
            writeln!(text, "line {line}").unwrap();
            text
        })
    }
}

#[tokio::test]
async fn read_only_document_rejects_save_and_hunk_changes_even_with_a_git_source() {
    let fixture = Fixture::new("file.txt", Some("old\n"));
    std::fs::write(fixture.absolute(), "new\n").unwrap();
    let mut doc = (*fixture.doc(false).await).clone();
    assert!(doc.source.is_some());
    doc.read_only = true;
    let doc = Arc::new(doc);
    assert!(
        fixture
            .change(Arc::clone(&doc), 0, Kind::Stage)
            .await
            .is_err()
    );
    assert!(
        execute(ExecuteInput {
            service: &fixture.service,
            request: Request::Save {
                document: doc,
                text: "changed\n".into()
            },
            staged: false,
            cancel: &CancellationToken::new()
        })
        .await
        .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(fixture.absolute()).unwrap(),
        "new\n"
    );
    assert_eq!(fixture.index(), "old\n");
}
fn changed() -> String {
    original()
        .replace("line 2\n", "first change\n")
        .replace("line 25\n", "second change\n")
}

#[tokio::test]
async fn saving_diff_edits_preserves_index_and_supports_undo() {
    let fixture = Fixture::new("file.txt", Some("base\n"));
    std::fs::write(fixture.absolute(), "local\n").unwrap();
    let outcome = execute(ExecuteInput {
        service: &fixture.service,
        request: Request::Save {
            document: fixture.doc(false).await,
            text: "edited λ\nlast line".into(),
        },
        staged: false,
        cancel: &CancellationToken::new(),
    })
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(fixture.absolute()).unwrap(),
        "edited λ\nlast line"
    );
    assert_eq!(fixture.index(), "base\n");
    fixture.undo(outcome.undo.unwrap()).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(fixture.absolute()).unwrap(),
        "local\n"
    );
}

#[tokio::test]
async fn saving_diff_edits_refuses_stale_disk_and_staged_snapshots() {
    let fixture = Fixture::new("file.txt", Some("base\n"));
    let document = fixture.doc(false).await;
    std::fs::write(fixture.absolute(), "external edit\n").unwrap();
    let error = execute(ExecuteInput {
        service: &fixture.service,
        request: Request::Save {
            document,
            text: "draft\n".into(),
        },
        staged: false,
        cancel: &CancellationToken::new(),
    })
    .await
    .unwrap_err();
    assert!(error.to_string().contains("changed"));
    let error = execute(ExecuteInput {
        service: &fixture.service,
        request: Request::Save {
            document: fixture.doc(true).await,
            text: "draft\n".into(),
        },
        staged: true,
        cancel: &CancellationToken::new(),
    })
    .await
    .unwrap_err();
    assert!(error.to_string().contains("working tree"));
    assert_eq!(fixture.index(), "base\n");
    assert_eq!(
        std::fs::read_to_string(fixture.absolute()).unwrap(),
        "external edit\n"
    );
}

#[tokio::test]
async fn staging_and_unstaging_one_hunk_preserve_other_changes_and_support_undo() {
    let fixture = Fixture::new("file.txt", Some(&original()));
    std::fs::write(fixture.absolute(), changed()).unwrap();
    let doc = fixture.doc(false).await;
    assert_eq!(doc.hunks.len(), 2);
    let outcome = fixture
        .change(Arc::clone(&doc), 0, Kind::Stage)
        .await
        .unwrap();
    assert_eq!(
        fixture.index(),
        original().replace("line 2\n", "first change\n")
    );
    assert_eq!(
        std::fs::read_to_string(fixture.absolute()).unwrap(),
        changed()
    );
    assert!(
        fixture
            .change(doc, 1, Kind::Stage)
            .await
            .unwrap_err()
            .to_string()
            .contains("changed")
    );
    fixture.undo(outcome.undo.unwrap()).await.unwrap();
    assert_eq!(fixture.index(), original());

    fixture.git(&["add", "--", "file.txt"]);
    let doc = fixture.doc(true).await;
    let outcome = fixture.change(doc, 0, Kind::Unstage).await.unwrap();
    assert_eq!(
        fixture.index(),
        original().replace("line 25\n", "second change\n")
    );
    assert_eq!(
        std::fs::read_to_string(fixture.absolute()).unwrap(),
        changed()
    );
    fixture.undo(outcome.undo.unwrap()).await.unwrap();
    assert_eq!(fixture.index(), changed());
}

#[tokio::test]
async fn discard_and_undo_preserve_other_hunks_and_refuse_newer_edits() {
    let fixture = Fixture::new("file.txt", Some(&original()));
    std::fs::write(fixture.absolute(), changed()).unwrap();
    let outcome = fixture
        .change(fixture.doc(false).await, 0, Kind::Discard)
        .await
        .unwrap();
    assert_eq!(fixture.index(), original());
    assert_eq!(
        std::fs::read_to_string(fixture.absolute()).unwrap(),
        original().replace("line 25\n", "second change\n")
    );
    let undo = outcome.undo.unwrap();
    fixture.undo(undo.clone()).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(fixture.absolute()).unwrap(),
        changed()
    );
    fixture
        .change(fixture.doc(false).await, 0, Kind::Discard)
        .await
        .unwrap();
    std::fs::write(fixture.absolute(), "new edit\n").unwrap();
    assert!(fixture.undo(undo).await.is_err());
    assert_eq!(
        std::fs::read_to_string(fixture.absolute()).unwrap(),
        "new edit\n"
    );
}

#[tokio::test]
async fn untracked_file_can_be_staged_discarded_and_restored_with_quoted_paths() {
    // Quotes and tabs are legal on Unix and need C-style quoting.
    // Windows rejects both names. `ü` is legal on both and still forces octal escapes.
    let name = if cfg!(windows) {
        "odd ü.txt"
    } else {
        "odd \"ü\"\t.txt"
    };
    let fixture = Fixture::new(name, None);
    std::fs::write(fixture.absolute(), "new file without newline").unwrap();
    let doc = fixture.doc(false).await;
    let outcome = fixture.change(doc, 0, Kind::Stage).await.unwrap();
    assert_eq!(fixture.index(), "new file without newline");
    fixture.undo(outcome.undo.unwrap()).await.unwrap();
    assert!(fixture.git(&["ls-files", "-z"]).is_empty());
    let outcome = fixture
        .change(fixture.doc(false).await, 0, Kind::Discard)
        .await
        .unwrap();
    assert!(!fixture.absolute().exists());
    assert!(fixture.doc(false).await.hunks.is_empty());
    fixture.undo(outcome.undo.unwrap()).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(fixture.absolute()).unwrap(),
        "new file without newline"
    );
}

#[tokio::test]
async fn deleted_file_stage_unstage_and_discard_keep_file_identity() {
    let fixture = Fixture::new("deleted.txt", Some("original\r\nlast line"));
    std::fs::remove_file(fixture.absolute()).unwrap();
    let outcome = fixture
        .change(fixture.doc(false).await, 0, Kind::Discard)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(fixture.absolute()).unwrap(),
        b"original\r\nlast line"
    );
    fixture.undo(outcome.undo.unwrap()).await.unwrap();
    assert!(!fixture.absolute().exists());
    fixture
        .change(fixture.doc(false).await, 0, Kind::Stage)
        .await
        .unwrap();
    assert!(fixture.git(&["ls-files", "-z"]).is_empty());
    fixture
        .change(fixture.doc(true).await, 0, Kind::Unstage)
        .await
        .unwrap();
    assert_eq!(fixture.index(), "original\r\nlast line");
    assert!(!fixture.absolute().exists());
}

#[tokio::test]
async fn stale_disk_snapshot_cannot_discard_any_changes() {
    let fixture = Fixture::new("file.txt", Some(&original()));
    std::fs::write(fixture.absolute(), changed()).unwrap();
    let doc = fixture.doc(false).await;
    std::fs::write(fixture.absolute(), "newer content\n").unwrap();
    assert!(fixture.change(doc, 0, Kind::Discard).await.is_err());
    assert_eq!(
        std::fs::read_to_string(fixture.absolute()).unwrap(),
        "newer content\n"
    );
    assert_eq!(fixture.index(), original());
}

#[test]
fn context_expansion_keeps_hunk_ranges_and_rows_correct() {
    let compact = super::super::build_with_context(super::super::BuildWithContextInput {
        path: Path::new("file.txt"),
        left: &original(),
        right: &changed(),
        staged: false,
        context: 3,
    });
    let expanded = super::super::build_with_context(super::super::BuildWithContextInput {
        path: Path::new("file.txt"),
        left: &original(),
        right: &changed(),
        staged: false,
        context: 100,
    });
    assert!(expanded.unified.len() > compact.unified.len());
    assert_eq!(compact.hunks.len(), 2);
    for (before, after) in compact.hunks.iter().zip(&expanded.hunks) {
        assert_eq!(before.old, after.old);
        assert_eq!(before.new, after.new);
        assert_eq!(
            expanded.unified[after.unified_row].kind,
            super::super::LineKind::Delete
        );
        assert_eq!(
            expanded.split[after.split_row].left.as_ref().unwrap().kind,
            super::super::LineKind::Delete
        );
    }
    let mut state = super::super::review::State::default();
    state.advance(true, 2);
    assert_eq!(state.selected, Some(0));
    state.advance(true, 2);
    assert_eq!(state.selected, Some(1));
    state.advance(true, 2);
    assert_eq!(state.selected, Some(1));
    state.advance(false, 2);
    assert_eq!(state.selected, Some(0));
    state.advance(false, 2);
    assert_eq!(state.selected, Some(0));
}

#[test]
fn expanding_one_gap_keeps_other_gaps_and_the_change_identity() {
    let original = (0..100)
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let changed = original
        .replacen("10\n", "changed\n", 1)
        .replacen("80\n", "another\n", 1);
    let doc = super::super::build_with_context(super::super::BuildWithContextInput {
        path: Path::new("file.txt"),
        left: &original,
        right: &changed,
        staged: false,
        context: 3,
    });
    assert_eq!(doc.gaps.len(), 3);
    let selected = doc.gaps[1].old.clone();
    let expanded = doc
        .expand_gap(ExpandGapInput {
            path: Path::new("file.txt"),
            gap: 1,
            part: super::super::GapPart::All,
            staged: false,
        })
        .unwrap();
    assert_eq!(expanded.gaps.len(), 2);
    assert!(expanded.gaps.iter().all(|gap| gap.old != selected));
    assert_eq!(expanded.left_text, original);
    assert_eq!(expanded.right_text, changed);
    assert_eq!(
        expanded
            .hunks
            .iter()
            .map(|hunk| (&hunk.old, &hunk.new))
            .collect::<Vec<_>>(),
        doc.hunks
            .iter()
            .map(|hunk| (&hunk.old, &hunk.new))
            .collect::<Vec<_>>()
    );
    assert!(expanded.unified.len() > doc.unified.len());
    assert!(
        expanded
            .unified
            .iter()
            .all(|line| !line.spans.iter().any(|span| span.text.starts_with("@@")))
    );
}

struct ChangeLinesInput<'a> {
    fixture: &'a Fixture,
    doc: Arc<DiffDocument>,
    kind: Kind,
    old: &'a [usize],
    new: &'a [usize],
}

async fn change_lines(input: ChangeLinesInput<'_>) -> Result<Outcome> {
    let ChangeLinesInput {
        fixture,
        doc,
        kind,
        old,
        new,
    } = input;
    execute(ExecuteInput {
        service: &fixture.service,
        request: Request::Hunk {
            document: doc,
            hunk: 0,
            kind,
            lines: Some(LineSelection {
                old: old.iter().copied().collect(),
                new: new.iter().copied().collect(),
            }),
        },
        staged: kind == Kind::Unstage,
        cancel: &CancellationToken::new(),
    })
    .await
}

#[tokio::test]
async fn picked_lines_stage_unstage_and_discard_only_themselves() {
    let fixture = Fixture::new("file.txt", Some("a\nold\nz\n"));
    std::fs::write(fixture.absolute(), "a\nnew one\nnew two\nz\n").unwrap();
    let doc = fixture.doc(false).await;
    assert_eq!(doc.hunks.len(), 1);
    // Stage only the first added line; the removal stays unstaged.
    let outcome = change_lines(ChangeLinesInput {
        fixture: &fixture,
        doc,
        kind: Kind::Stage,
        old: &[],
        new: &[1],
    })
    .await
    .unwrap();
    assert!(outcome.message.contains("1 line"), "{}", outcome.message);
    assert_eq!(fixture.index(), "a\nold\nnew one\nz\n");
    // Unstage it again by picking that line on the staged side.
    let staged = fixture.doc(true).await;
    change_lines(ChangeLinesInput {
        fixture: &fixture,
        doc: staged,
        kind: Kind::Unstage,
        old: &[],
        new: &[2],
    })
    .await
    .unwrap();
    assert_eq!(fixture.index(), "a\nold\nz\n");
    // Discard only the second added line from the working tree.
    let working = fixture.doc(false).await;
    change_lines(ChangeLinesInput {
        fixture: &fixture,
        doc: working,
        kind: Kind::Discard,
        old: &[],
        new: &[2],
    })
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(fixture.absolute()).unwrap(),
        "a\nnew one\nz\n"
    );
}

#[tokio::test]
async fn whitespace_display_policy_stages_and_discards_exact_source_text() {
    let fixture = Fixture::new("file.txt", Some(&original()));
    let working = original()
        .replace("line 2\n", "\tfirst  change \n")
        .replace("line 25\n", "line   25  \n");
    std::fs::write(fixture.absolute(), &working).unwrap();
    let exact = fixture.doc(false).await;
    assert_eq!(exact.hunks.len(), 2);
    let view = super::super::DiffView {
        context: exact.context,
        revealed: Vec::new(),
        revealed_in: None,
        options: super::super::DiffOptions {
            whitespace: super::super::Whitespace::All,
            ..super::super::DiffOptions::default()
        },
    };
    let loose = Arc::new(exact.rebuild(Path::new("file.txt"), false, &view));
    assert_eq!(loose.hunks.len(), 1, "whitespace-only change is hidden");
    assert!(loose.source.is_some());
    // Staging writes the real working line, whitespace included, and
    // leaves the hidden whitespace-only change unstaged.
    fixture
        .change(Arc::clone(&loose), 0, Kind::Stage)
        .await
        .unwrap();
    assert_eq!(
        fixture.index(),
        original().replace("line 2\n", "\tfirst  change \n")
    );
    assert_eq!(
        std::fs::read_to_string(fixture.absolute()).unwrap(),
        working
    );
    // Discard under the same policy restores the exact index line.
    fixture.git(&["reset", "-q", "--", "file.txt"]);
    let loose = Arc::new(
        fixture
            .doc(false)
            .await
            .rebuild(Path::new("file.txt"), false, &view),
    );
    fixture.change(loose, 0, Kind::Discard).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(fixture.absolute()).unwrap(),
        original().replace("line 25\n", "line   25  \n")
    );
    assert_eq!(fixture.index(), original());
}

/// A CRLF worktree file committed under `core.autocrlf=true`.
fn autocrlf_fixture() -> Fixture {
    let fixture = Fixture::new("file.txt", None);
    fixture.git(&["config", "core.autocrlf", "true"]);
    std::fs::write(fixture.absolute(), "a\r\nb\r\nc\r\n").unwrap();
    fixture.git(&["add", "--", "file.txt"]);
    fixture.git(&["commit", "-qm", "fixture"]);
    std::fs::write(fixture.absolute(), "a\r\nB\r\nc\r\n").unwrap();
    fixture
}

#[tokio::test]
async fn autocrlf_diff_shows_only_real_changes_and_stages_lf_lines() {
    let fixture = autocrlf_fixture();
    let doc = fixture.doc(false).await;
    assert_eq!(doc.right_text, "a\nB\nc\n");
    assert_eq!(doc.hunks.len(), 1);
    assert_eq!(doc.hunks[0].old, 1..2);
    fixture.change(doc, 0, Kind::Stage).await.unwrap();
    assert_eq!(fixture.index(), "a\nB\nc\n");
}

#[tokio::test]
async fn autocrlf_diff_load_writes_no_git_objects() {
    let fixture = autocrlf_fixture();
    let before = fixture.git(&["count-objects"]);
    let doc = fixture.doc(false).await;
    assert_eq!(doc.right_text, "a\nB\nc\n");
    assert_eq!(fixture.git(&["count-objects"]), before);
}

#[tokio::test]
async fn autocrlf_discard_restores_crlf_worktree_lines() {
    let fixture = autocrlf_fixture();
    let doc = fixture.doc(false).await;
    fixture.change(doc, 0, Kind::Discard).await.unwrap();
    assert_eq!(
        std::fs::read(fixture.absolute()).unwrap(),
        b"a\r\nb\r\nc\r\n"
    );
    assert_eq!(fixture.index(), "a\nb\nc\n");
}

#[tokio::test]
async fn filtered_file_refuses_hunk_staging_and_keeps_the_index() {
    let fixture = Fixture::new("file.txt", Some("pointer\n"));
    std::fs::write(
        fixture.dir.path().join(".gitattributes"),
        // An unconfigured driver: a global `filter.lfs` (Windows runners)
        // would start git-lfs on every diff and time out the fixture.
        "file.txt filter=fixture\n",
    )
    .unwrap();
    std::fs::write(fixture.absolute(), "content\n").unwrap();
    let doc = fixture.doc(false).await;
    let error = fixture.change(doc, 0, Kind::Stage).await.unwrap_err();
    assert!(error.to_string().contains("Git filter"));
    assert_eq!(fixture.index(), "pointer\n");
}

#[tokio::test]
async fn staging_a_picked_line_after_a_missing_final_newline_keeps_lines_apart() {
    let fixture = Fixture::new("file.txt", Some("a\nb"));
    std::fs::write(fixture.absolute(), "a\nb\nc\n").unwrap();
    let doc = fixture.doc(false).await;
    assert_eq!(doc.hunks.len(), 1);
    let last = doc.hunks[0].new.end - 1;
    change_lines(ChangeLinesInput {
        fixture: &fixture,
        doc,
        kind: Kind::Stage,
        old: &[],
        new: &[last],
    })
    .await
    .unwrap();
    assert_eq!(fixture.index(), "a\nb\nc\n");
}

#[tokio::test]
async fn discarding_a_picked_removal_without_final_newline_keeps_lines_apart() {
    let fixture = Fixture::new("file.txt", Some("a\nb"));
    std::fs::write(fixture.absolute(), "a\nc\n").unwrap();
    let doc = fixture.doc(false).await;
    assert_eq!(doc.hunks.len(), 1);
    let removed = doc.hunks[0].old.start;
    change_lines(ChangeLinesInput {
        fixture: &fixture,
        doc,
        kind: Kind::Discard,
        old: &[removed],
        new: &[],
    })
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(fixture.absolute()).unwrap(),
        "a\nb\nc\n"
    );
}

#[test]
fn patch_marks_a_missing_final_newline() {
    let version = |text: &str| Version {
        text: text.into(),
        exists: true,
        mode: 0o100644,
    };
    let bytes = patch(Path::new("f.txt"), &version("a\nb"), &version("a\nb\nc\n"));
    let text = String::from_utf8(bytes).unwrap();
    assert_eq!(
        text.matches("\\ No newline at end of file").count(),
        1,
        "{text}"
    );
    assert!(
        text.contains("-b\n\\ No newline at end of file\n+b\n+c\n"),
        "{text}"
    );
}

#[tokio::test]
async fn staged_rename_diff_compares_with_the_rename_source() {
    let fixture = Fixture::new("old.txt", Some(&original()));
    fixture.git(&["mv", "old.txt", "new.txt"]);
    let renamed = original().replace("line 5\n", "line five\n");
    std::fs::write(fixture.dir.path().join("new.txt"), &renamed).unwrap();
    fixture.git(&["add", "--", "new.txt"]);
    let doc = super::super::document_async(super::super::DocumentAsyncInput {
        service: &fixture.service,
        cwd: fixture.dir.path().into(),
        path: fixture.dir.path().join("new.txt"),
        staged: true,
        cancel: &CancellationToken::new(),
    })
    .await
    .unwrap();
    assert_eq!(doc.left_text, original());
    assert_eq!(doc.hunks.len(), 1);
}

#[test]
fn only_index_requests_wait_for_a_running_commit() {
    use crate::workspace_ops::CommitRisk;
    let doc = Arc::new(DiffDocument {
        source: Some(Source::fixture(Path::new("/repo"), Path::new("file"))),
        ..DiffDocument::default()
    });
    let hunk = |kind| Request::Hunk {
        document: Arc::clone(&doc),
        hunk: 0,
        kind,
        lines: None,
    };
    assert_eq!(hunk(Kind::Stage).commit_risk(), CommitRisk::Index);
    assert_eq!(hunk(Kind::Unstage).commit_risk(), CommitRisk::Index);
    assert_eq!(hunk(Kind::Discard).commit_risk(), CommitRisk::None);
    assert_eq!(hunk(Kind::Stage).root(), Some(Path::new("/repo")));
    let save = Request::Save {
        document: Arc::clone(&doc),
        text: String::new(),
    };
    assert_eq!(save.commit_risk(), CommitRisk::None);
    let version = Version {
        text: String::new(),
        exists: true,
        mode: 0o100644,
    };
    let undo = |index| {
        Request::Undo(Undo {
            root: "/repo".into(),
            path: "file".into(),
            index,
            before: version.clone(),
            after: version.clone(),
            label: String::new(),
        })
    };
    assert_eq!(undo(true).commit_risk(), CommitRisk::Index);
    assert_eq!(undo(false).commit_risk(), CommitRisk::None);
    assert_eq!(undo(true).root(), Some(Path::new("/repo")));
}

/// A diff write and a commit sent through the job queue, with a real
/// `pre-commit` hook (a shell script).
#[cfg(unix)]
mod commit_order {
    use super::*;
    use crate::workspace_ops::{Op, Report};
    use std::time::{Duration, Instant};

    /// The reply of the commit job.
    async fn commit_reply(owner: &mut Owner) -> std::result::Result<Report, String> {
        let started = Instant::now();
        loop {
            if let Some(mut completion) = owner.supervisor.try_recv()
                && let Some(Ok(updates)) = completion.result.take()
            {
                for update in updates {
                    if let crate::Update::Workspace(_, Op::Commit { .. }, result) = update {
                        return result;
                    }
                }
            }
            assert!(started.elapsed() < Duration::from_secs(20), "no reply");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// A `pre-commit` hook that writes the names the commit holds to `seen`.
    fn record_hook(root: &Path, seen: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let hooks = root.join(".no-fixture-hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let hook = hooks.join("pre-commit");
        let script = format!(
            "#!/bin/sh\ngit diff --cached --name-only > '{}'\n",
            seen.display()
        );
        std::fs::write(&hook, script).unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[tokio::test]
    async fn a_hunk_staged_just_before_a_commit_lands_before_the_commit_hooks_run() {
        let mut fixture = Fixture::new("file.txt", Some(&original()));
        let root = fixture.dir.path().canonicalize().unwrap();
        std::fs::write(root.join("other.txt"), "other\n").unwrap();
        fixture.git(&["add", "--", "other.txt"]);
        std::fs::write(fixture.absolute(), changed()).unwrap();
        let record = tempfile::tempdir().unwrap();
        let seen = record.path().join("seen");
        record_hook(&root, &seen);
        let tab = crate::Tab::Diff {
            remote: None,
            cwd: root.clone(),
            path: root.join("file.txt"),
            staged: false,
        };
        let stage = Request::Hunk {
            document: fixture.doc(false).await,
            hunk: 0,
            kind: Kind::Stage,
            lines: None,
        };
        let commit = Op::Commit {
            message: "both".into(),
            amend: false,
            signoff: false,
            push: false,
            cancel: CancellationToken::new(),
        };
        // The stage still runs when Commit goes out.
        let held = execute::mutations().lock().await;
        let service = &fixture.service;
        service
            .send(crate::Job::DiffAction(tab, Box::new(stage)))
            .unwrap();
        service
            .send(crate::Job::Workspace(root.clone(), commit))
            .unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!seen.exists(), "Git commits once the stage landed");
        drop(held);
        let result = commit_reply(&mut fixture._owner).await;
        assert!(result.is_ok(), "{result:?}");
        // The hook checked the staged hunk, and the commit holds it.
        let checked = std::fs::read_to_string(&seen).unwrap();
        assert_eq!(
            checked.lines().collect::<Vec<_>>(),
            ["file.txt", "other.txt"]
        );
        let show = ["show", "--name-only", "--format=", "HEAD"];
        let committed = String::from_utf8(fixture.git(&show)).unwrap();
        assert_eq!(
            committed.lines().collect::<Vec<_>>(),
            ["file.txt", "other.txt"]
        );
    }
}
