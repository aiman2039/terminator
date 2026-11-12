use super::*;

fn doc(base: &str, ours: &str, theirs: &str) -> MergeDoc {
    MergeDoc::new(
        Side::new(base.into(), true),
        Side::new(ours.into(), true),
        Side::new(theirs.into(), true),
    )
}

fn kinds(doc: &MergeDoc) -> Vec<ChunkKind> {
    doc.chunks.iter().map(|chunk| chunk.kind).collect()
}

#[test]
fn chunks_split_one_sided_same_and_conflicting_changes() {
    let merge = doc(
        "a\nb\nc\nd\ne\nf\ng\n",
        "a\nB\nc\nd\nE\nf\ng\n",
        "a\nb\nc\nD\nE2\nf\nG\n",
    );
    assert_eq!(
        kinds(&merge),
        [
            ChunkKind::Stable,
            ChunkKind::Ours,
            ChunkKind::Stable,
            ChunkKind::Conflict,
            ChunkKind::Stable,
            ChunkKind::Theirs,
        ]
    );
    let conflict = &merge.chunks[3];
    assert_eq!(merge.base.slice(&conflict.base), "d\ne\n");
    assert_eq!(merge.ours.slice(&conflict.ours), "d\nE\n");
    assert_eq!(merge.theirs.slice(&conflict.theirs), "D\nE2\n");
    assert_eq!(merge.conflicts_left(), 1);
    assert_eq!(merge.changes_left(), 2);
    // The result starts as the base.
    assert_eq!(merge.result_text().matches("<<<<<<< ours").count(), 1);

    let same = doc("x\n", "y\n", "y\n");
    assert_eq!(kinds(&same), [ChunkKind::Same]);
    assert_eq!(same.conflicts_left(), 0);
    let clean = doc("x\n", "x\n", "x\n");
    assert_eq!(kinds(&clean), [ChunkKind::Stable]);
    assert_eq!(clean.result_text(), "x\n");
}

#[test]
fn insertions_at_both_ends_and_add_add_conflicts_align() {
    let merge = doc("m\n", "top\nm\n", "m\nbottom\n");
    assert_eq!(
        kinds(&merge),
        [ChunkKind::Ours, ChunkKind::Stable, ChunkKind::Theirs]
    );
    let mut merge = merge;
    assert_eq!(merge.apply_non_conflicting(), 2);
    assert_eq!(merge.result_text(), "top\nm\nbottom\n");

    let added = MergeDoc::new(
        Side::new(String::new(), false),
        Side::new("one\n".into(), true),
        Side::new("two\n".into(), true),
    );
    assert_eq!(kinds(&added), [ChunkKind::Conflict]);
}

#[test]
fn apply_non_conflicting_leaves_conflicts_for_the_user() {
    let mut merge = doc("a\nb\nc\n", "A\nb\nc1\n", "a\nb\nc2\n");
    assert_eq!(merge.apply_non_conflicting(), 1);
    assert_eq!(merge.changes_left(), 0);
    assert_eq!(merge.conflicts_left(), 1);
    assert_eq!(
        merge.result_text(),
        "A\nb\n<<<<<<< ours\nc1\n||||||| base\nc\n=======\nc2\n>>>>>>> theirs\n"
    );
    let conflict = merge.neighbor(None, true).unwrap();
    merge.accept(conflict, Pick::Both);
    assert_eq!(merge.conflicts_left(), 0);
    assert_eq!(merge.result_text(), "A\nb\nc1\nc2\n");
    merge.accept(conflict, Pick::Theirs);
    assert_eq!(merge.result_text(), "A\nb\nc2\n");
    merge.accept(conflict, Pick::Base);
    assert_eq!(merge.result_text(), "A\nb\nc\n");
    assert_eq!(merge.neighbor(None, true), None);
}

#[test]
fn hand_edits_resolve_the_chunk_and_navigation_skips_resolved() {
    let mut merge = doc("1\n2\n3\n4\n5\n", "x\n2\n3\n4\ny\n", "z\n2\n3\n4\nw\n");
    let first = merge.neighbor(None, true).unwrap();
    let last = merge.neighbor(Some(first), true).unwrap();
    assert!(last > first);
    assert_eq!(merge.neighbor(Some(last), false), Some(first));
    merge.chunks[first].result = "mine\n".into();
    merge.edited(first);
    assert_eq!(merge.neighbor(None, true), Some(last));
    assert_eq!(merge.conflicts_left(), 1);
}

#[test]
fn long_stable_runs_split_into_blocks_without_changing_the_text() {
    let base: String = (0..200).map(|line| line.to_string() + "\n").collect();
    let ours = base.replace("150\n", "ours\n");
    let merge = doc(&base, &ours, &base);
    assert!(merge.chunks.len() > 3);
    assert!(
        merge
            .chunks
            .iter()
            .all(|chunk| chunk.base.len() <= STABLE_BLOCK)
    );
    assert_eq!(merge.result_text(), base);
    let mut merge = merge;
    merge.apply_non_conflicting();
    assert_eq!(merge.result_text(), ours);
}

#[test]
fn files_without_final_newline_keep_markers_on_their_own_lines() {
    let merge = doc("a", "b", "c");
    assert_eq!(
        merge.result_text(),
        "<<<<<<< ours\nb\n||||||| base\na\n=======\nc\n>>>>>>> theirs\n"
    );
}

/// Saving writes the file without staging; only Mark resolved stages it.
#[test]
fn mark_resolved_stages_only_on_explicit_request() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = eframe::egui::Context::default();
    let mut app =
        crate::App::with_context(&ctx, terminator_core::Paths::at(dir.path().join("state")));
    let (jobs, received) = std::sync::mpsc::channel();
    app.jobs = jobs.into();
    app.state.projects.push(terminator_core::Project {
        id: "p".into(),
        name: "p".into(),
        path: dir.path().into(),
        layout: serde_json::Value::Null,
    });
    app.selected = Some("p".into());
    let path = dir.path().join("conflicted.txt");
    std::fs::write(&path, "markers\n").unwrap();

    let tab = tab(dir.path().into(), path.clone());
    let key = tab.key();
    let key = key.as_str();
    let mut state = MergeTab::default();
    let mut merge = doc("a\n", "b\n", "c\n");
    merge.accept(0, Pick::Theirs);
    state.doc = Some(merge);
    state.dirty = true;
    app.merge.insert(key.into(), state);
    let staged = |received: &std::sync::mpsc::Receiver<crate::Job>| {
        received.try_iter().any(|job| {
            matches!(
                job,
                crate::Job::Workspace(_, crate::workspace_ops::Op::Stage(ref staged)) if *staged == path
            )
        })
    };
    let wait = |app: &mut crate::App| {
        let start = std::time::Instant::now();
        while app.merge.get(key).is_some_and(MergeTab::busy) {
            assert!(start.elapsed() < std::time::Duration::from_secs(10));
            app.poll_merge(&tab);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    };
    app.save_merge(key, &path, false);
    wait(&mut app);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "c\n");
    assert!(!staged(&received), "Save must not stage");
    app.save_merge(key, &path, true);
    wait(&mut app);
    assert!(staged(&received), "Mark resolved stages the file");
}

/// Mark resolved still stages when the merge tab is no longer drawn (#184).
#[test]
fn mark_resolved_stages_after_tab_is_left() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = eframe::egui::Context::default();
    let mut app =
        crate::App::with_context(&ctx, terminator_core::Paths::at(dir.path().join("state")));
    let (jobs, received) = std::sync::mpsc::channel();
    app.jobs = jobs.into();
    app.state.projects.push(terminator_core::Project {
        id: "p".into(),
        name: "p".into(),
        path: dir.path().into(),
        layout: serde_json::Value::Null,
    });
    app.selected = Some("p".into());
    let path = dir.path().join("conflicted.txt");
    std::fs::write(&path, "markers\n").unwrap();
    let key = tab(dir.path().into(), path.clone()).key();
    let mut state = MergeTab::default();
    let mut merge = doc("a\n", "b\n", "c\n");
    merge.accept(0, Pick::Theirs);
    state.doc = Some(merge);
    state.dirty = true;
    app.merge.insert(key.clone(), state);
    app.save_merge(&key, &path, true);
    let start = std::time::Instant::now();
    while app.merge.get(&key).is_some_and(MergeTab::busy) {
        assert!(start.elapsed() < std::time::Duration::from_secs(10));
        app.poll_merge_saves();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "c\n");
    assert!(received.try_iter().any(|job| matches!(
        job,
        crate::Job::Workspace(_, crate::workspace_ops::Op::Stage(ref staged)) if *staged == path
    )));
}

#[test]
fn accepting_the_deleted_side_never_writes_an_empty_file() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = eframe::egui::Context::default();
    let mut app =
        crate::App::with_context(&ctx, terminator_core::Paths::at(dir.path().join("state")));
    let (jobs, received) = std::sync::mpsc::channel();
    app.jobs = jobs.into();
    let path = dir.path().join("conflicted.txt");
    std::fs::write(&path, "markers\n").unwrap();
    let key = tab(dir.path().into(), path.clone()).key();
    let mut merge = MergeDoc::new(
        Side::new("a\nb\nc\n".into(), true),
        Side::new(String::new(), false),
        Side::new("a\nB\nc\n".into(), true),
    );
    merge.accept(0, Pick::Ours);
    assert!(merge.result_text().is_empty());
    let mut state = MergeTab::default();
    state.doc = Some(merge);
    state.dirty = true;
    app.merge.insert(key.clone(), state);
    app.save_merge(&key, &path, true);
    assert!(!app.merge[&key].busy(), "no write starts");
    assert_eq!(app.merge[&key].error.as_deref(), Some(io::DELETION_REFUSED));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "markers\n");
    assert!(received.try_iter().next().is_none(), "nothing is staged");
}

#[test]
fn unsaved_merge_result_blocks_closing_and_quit() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = eframe::egui::Context::default();
    let mut app =
        crate::App::with_context(&ctx, terminator_core::Paths::at(dir.path().join("state")));
    let key = tab(dir.path().into(), dir.path().join("conflicted.txt")).key();
    let mut state = MergeTab::default();
    state.dirty = true;
    app.merge.insert(key.clone(), state);
    assert!(app.block_unsaved_diff(Some(&key)));
    assert!(app.block_unsaved_diff(None));
    assert!(!app.block_unsaved_diff(Some("other")));
    app.merge.get_mut(&key).unwrap().dirty = false;
    assert!(!app.block_unsaved_diff(None));
}

/// What the status bar says when a running commit refuses an action.
const RUNNING: &str = "A commit is running in this repository. Wait for it to finish.";

/// An app on the repository `root` with a resolved, unsaved merge of
/// `conflicted.txt`: its job queue, the merge tab key, and the file.
struct ResolvedMerge {
    app: crate::App,
    ctx: eframe::egui::Context,
    received: std::sync::mpsc::Receiver<crate::Job>,
    key: String,
    path: PathBuf,
}

fn resolved_merge(root: &std::path::Path) -> ResolvedMerge {
    let ctx = eframe::egui::Context::default();
    let mut app = crate::App::with_context(&ctx, terminator_core::Paths::at(root.join("state")));
    let (jobs, received) = std::sync::mpsc::channel();
    app.jobs = jobs.into();
    app.state.projects.push(terminator_core::Project {
        id: "p".into(),
        name: "p".into(),
        path: root.into(),
        layout: serde_json::Value::Null,
    });
    app.selected = Some("p".into());
    app.context = Some(crate::services::ContextData {
        cwd: root.into(),
        root: Some(root.into()),
        perforce: None,
        git_dirs: vec![],
        branch: "main".into(),
        changes: vec![],
        decorations: std::collections::HashMap::default(),
        stats: std::collections::HashMap::default(),
        error: None,
    });
    let path = root.join("conflicted.txt");
    std::fs::write(&path, "markers\n").unwrap();
    let key = tab(root.into(), path.clone()).key();
    let mut state = MergeTab::default();
    let mut merge = doc("a\n", "b\n", "c\n");
    merge.accept(0, Pick::Theirs);
    state.doc = Some(merge);
    state.dirty = true;
    app.merge.insert(key.clone(), state);
    ResolvedMerge {
        app,
        ctx,
        received,
        key,
        path,
    }
}

impl ResolvedMerge {
    /// Commits from the Git panel; returns the commit job's root and op.
    fn commit(&mut self) -> (PathBuf, crate::workspace_ops::Op) {
        self.app.git.commit = "msg".into();
        let app = &mut self.app;
        let mut output = self.ctx.run_ui(eframe::egui::RawInput::default(), |ui| {
            let commit = crate::sidebar_ui::GitPanelOutcome {
                commit: true,
                ..Default::default()
            };
            app.perform_git_outcome(ui, commit);
        });
        output.textures_delta.clear();
        self.received
            .try_iter()
            .find_map(|job| match job {
                crate::Job::Workspace(root, op @ crate::workspace_ops::Op::Commit { .. }) => {
                    Some((root, op))
                }
                _ => None,
            })
            .expect("a commit job was queued")
    }

    fn wait_for_the_write(&mut self) {
        let start = std::time::Instant::now();
        while self.app.merge.get(&self.key).is_some_and(MergeTab::busy) {
            assert!(start.elapsed() < std::time::Duration::from_secs(10));
            self.app.poll_merge_saves();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn staged(&self) -> bool {
        self.received.try_iter().any(|job| {
            matches!(
                job,
                crate::Job::Workspace(_, crate::workspace_ops::Op::Stage(ref staged)) if *staged == self.path
            )
        })
    }

    /// One frame of the merge tab at `time`; returns the painted texts
    /// with their top-left corners.
    fn paint(
        &mut self,
        time: f64,
        events: Vec<eframe::egui::Event>,
    ) -> Vec<(eframe::egui::Pos2, String)> {
        let input = eframe::egui::RawInput {
            screen_rect: Some(eframe::egui::Rect::from_min_size(
                eframe::egui::Pos2::ZERO,
                eframe::egui::vec2(900.0, 600.0),
            )),
            time: Some(time),
            events,
            ..Default::default()
        };
        let root = self.path.parent().unwrap().to_path_buf();
        let merge = tab(root, self.path.clone());
        let app = &mut self.app;
        let mut output = self.ctx.run_ui(input, |ui| app.merge_view(ui, &merge));
        output.textures_delta.clear();
        crate::app::painted_text(&output.shapes)
    }
}

#[test]
fn mark_resolved_says_why_it_waits_for_a_running_commit() {
    let dir = tempfile::tempdir().unwrap();
    let mut merge = resolved_merge(&dir.path().canonicalize().unwrap());
    merge.commit();
    let at = merge
        .paint(1.0, vec![])
        .into_iter()
        .find_map(|(at, text)| (text == "Mark resolved").then_some(at))
        .expect("Mark resolved is painted");
    let pointer = at + eframe::egui::vec2(4.0, 4.0);
    let mut texts = Vec::new();
    for frame in 0..12 {
        let events = if frame == 0 {
            vec![eframe::egui::Event::PointerMoved(pointer)]
        } else {
            vec![]
        };
        texts = merge.paint(1.1 + f64::from(frame) * 0.1, events);
    }
    assert!(
        texts
            .iter()
            .any(|(_, text)| text == "Wait for the commit to finish"),
        "{texts:?}"
    );
}

#[test]
fn mark_resolved_waits_while_a_commit_runs_in_the_repository() {
    let dir = tempfile::tempdir().unwrap();
    let mut merge = resolved_merge(&dir.path().canonicalize().unwrap());
    let (root, op) = merge.commit();
    let (key, path) = (merge.key.clone(), merge.path.clone());
    merge.app.save_merge(&key, &path, true);
    assert!(!merge.app.merge[&key].busy(), "nothing is written");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "markers\n");
    assert_eq!(merge.app.info.as_deref(), Some(RUNNING));
    // Once the commit ends, Mark resolved writes and stages the file.
    let committed = Ok(crate::workspace_ops::Report::Message("Committed".into()));
    let reply = crate::Update::Workspace(root, op, committed);
    merge.app.update_tx.send(reply).unwrap();
    crate::app::drain_updates(&mut merge.app, &merge.ctx);
    merge.app.save_merge(&key, &path, true);
    merge.wait_for_the_write();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "c\n");
    assert!(merge.staged(), "Mark resolved stages the file");
}

#[test]
fn a_commit_that_starts_before_the_resolved_file_is_written_leaves_it_unstaged() {
    let dir = tempfile::tempdir().unwrap();
    let mut merge = resolved_merge(&dir.path().canonicalize().unwrap());
    let (key, path) = (merge.key.clone(), merge.path.clone());
    merge.app.save_merge(&key, &path, true);
    merge.commit();
    merge.wait_for_the_write();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "c\n");
    assert!(!merge.staged(), "the stage waits for the commit");
    assert_eq!(merge.app.merge[&key].message.as_deref(), Some("Saved"));
    assert_eq!(merge.app.info.as_deref(), Some(RUNNING));
}

/// A merge read the pool refused, or one that ended without a result,
/// shows an error instead of loading forever (#267).
#[test]
fn refused_or_dead_merge_read_shows_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = eframe::egui::Context::default();
    let mut app =
        crate::App::with_context(&ctx, terminator_core::Paths::at(dir.path().join("state")));
    app.services.handle().close_admission();
    let tab = tab(dir.path().into(), dir.path().join("conflicted.txt"));
    app.poll_merge(&tab);
    let state = &app.merge[&tab.key()];
    assert!(!state.busy(), "nothing is loading");
    assert!(
        state
            .error
            .as_deref()
            .is_some_and(|e| e.contains("Could not start"))
    );
    // A job whose sender went away (timeout, panic) is an error too.
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel::<Result<(), String>>();
    drop(sender);
    assert!(matches!(super::take_done(&mut receiver), Some(Err(_))));
}
