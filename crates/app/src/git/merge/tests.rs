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
