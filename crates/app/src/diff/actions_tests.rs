#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]
use super::*;
use crate::gui_services::Owner;

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
            super::super::document_async(
                &self.service,
                self.dir.path().into(),
                self.absolute(),
                staged,
                &CancellationToken::new(),
            )
            .await
            .unwrap(),
        )
    }

    async fn change(&self, doc: Arc<DiffDocument>, hunk: usize, kind: Kind) -> Result<Outcome> {
        execute(
            &self.service,
            Request::Hunk {
                document: doc,
                hunk,
                kind,
            },
            kind == Kind::Unstage,
            &CancellationToken::new(),
        )
        .await
    }

    async fn undo(&self, undo: Undo) -> Result<Outcome> {
        execute(
            &self.service,
            Request::Undo(undo),
            false,
            &CancellationToken::new(),
        )
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
fn changed() -> String {
    original()
        .replace("line 2\n", "first change\n")
        .replace("line 25\n", "second change\n")
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
    let fixture = Fixture::new("odd \"ü\"\t.txt", None);
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
    let compact =
        super::super::build_with_context(Path::new("file.txt"), &original(), &changed(), false, 3);
    let expanded = super::super::build_with_context(
        Path::new("file.txt"),
        &original(),
        &changed(),
        false,
        100,
    );
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
    let doc =
        super::super::build_with_context(Path::new("file.txt"), &original, &changed, false, 3);
    assert_eq!(doc.gaps.len(), 3);
    let selected = doc.gaps[1].old.clone();
    let expanded = doc.expand_gap(Path::new("file.txt"), 1, false).unwrap();
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
