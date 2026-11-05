//! A commit running in the diff's repository locks its staging controls.
use super::*;

/// What the status bar says when a running commit refuses an action.
const RUNNING: &str = "A commit is running in this repository. Wait for it to finish.";

/// Project `a` with the active working-tree diff of `/a/file.rs` (its
/// first change selected) from the repository `source`, and a commit that
/// the Git panel started in `commit`.
struct LockedDiff {
    app: App,
    ctx: egui::Context,
    _dir: tempfile::TempDir,
    requests: std::sync::mpsc::Receiver<Job>,
    tab: Tab,
    time: f64,
}

fn locked_diff(commit: &str, source: &str) -> LockedDiff {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut app = App::with_context(&ctx, Paths::at(dir.path().into()));
    let (jobs, requests) = std::sync::mpsc::channel();
    app.jobs = jobs.into();
    app.selected = Some("a".into());
    let tab = Tab::Diff {
        remote: None,
        cwd: "/a".into(),
        path: "/a/file.rs".into(),
        staged: false,
    };
    let layout = egui_dock::DockState::new(vec![tab.clone()]);
    app.layouts
        .insert("a".into(), Workspace::from_layout(layout));
    let mut doc = diff::build_with_context(diff::BuildWithContextInput {
        path: std::path::Path::new("file.rs"),
        left: "old\n",
        right: "new\n",
        staged: false,
        context: 3,
    });
    doc.options = app.preferences.diff_options;
    let (root, file) = (
        std::path::Path::new(source),
        std::path::Path::new("file.rs"),
    );
    doc.source = Some(diff::actions::Source::fixture(root, file));
    app.diffs.insert(tab.key(), Ok(doc.into()));
    app.diff_reviews.entry(tab.key()).or_default().selected = Some(0);
    app.context = Some(crate::services::ContextData {
        cwd: commit.into(),
        root: Some(commit.into()),
        perforce: None,
        git_dirs: vec![],
        branch: "main".into(),
        changes: vec![],
        decorations: std::collections::HashMap::default(),
        stats: std::collections::HashMap::default(),
        error: None,
    });
    app.git.commit = "msg".into();
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        let commit = crate::sidebar_ui::GitPanelOutcome {
            commit: true,
            ..Default::default()
        };
        app.perform_git_outcome(ui, commit);
    });
    output.textures_delta.clear();
    let started = requests
        .try_iter()
        .any(|job| matches!(job, Job::Workspace(_, workspace_ops::Op::Commit { .. })));
    assert!(started, "the commit runs");
    LockedDiff {
        app,
        ctx,
        _dir: dir,
        requests,
        tab,
        time: 1.0,
    }
}

impl LockedDiff {
    fn frames(&mut self, frames: Vec<Vec<egui::Event>>) {
        let frames: Vec<(f64, Vec<egui::Event>)> = frames
            .into_iter()
            .map(|events| {
                self.time += 0.1;
                (self.time, events)
            })
            .collect();
        run_diff_frames(RunDiffFramesInput {
            app: &mut self.app,
            ctx: &self.ctx,
            tab: &self.tab,
            frames: &frames,
        });
    }

    fn key(&mut self, event: egui::Event) {
        self.frames(vec![vec![event], vec![]]);
    }

    fn click(&mut self, name: &str) {
        let pos = fixture_target(&self.ctx, name)
            .unwrap_or_else(|| panic!("{name} is painted"))
            .center();
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.frames(vec![
            vec![egui::Event::PointerMoved(pos)],
            vec![press(true)],
            vec![press(false)],
            vec![],
        ]);
    }

    /// The hunk and file actions sent since the last call.
    fn actions(&self) -> Vec<Job> {
        self.requests
            .try_iter()
            .filter(|job| matches!(job, Job::DiffAction(..) | Job::Workspace(..)))
            .collect()
    }
}

fn stage_key() -> egui::Event {
    egui::Event::Text("s".into())
}

#[test]
fn diff_staging_waits_while_a_commit_runs_in_its_repository() {
    let mut diff = locked_diff("/a", "/a");
    diff.frames(vec![vec![], vec![]]);
    // The whole-file and hunk Stage buttons are disabled.
    let file_stage = fixture_target(&diff.ctx, "diff-stage-file");
    assert!(file_stage.is_none(), "the file Stage button is disabled");
    diff.click("diff-hunk-action-0");
    assert!(diff.actions().is_empty());
    assert!(diff.app.info.is_none(), "a disabled button refuses nothing");
    // The `s` key is refused, and the status bar says why.
    diff.key(stage_key());
    assert_eq!(diff.app.info.as_deref(), Some(RUNNING));
    assert!(diff.actions().is_empty(), "a refused action sends nothing");
    // Once the push starts, the same key stages the change.
    assert!(diff.app.git.commit_pushing());
    diff.key(stage_key());
    let sent = diff.actions();
    assert!(
        matches!(sent.as_slice(), [Job::DiffAction(..)]),
        "{} jobs",
        sent.len()
    );
}

#[test]
fn hunk_discard_stays_available_while_a_commit_runs() {
    let mut diff = locked_diff("/a", "/a");
    diff.frames(vec![vec![], vec![]]);
    diff.click("diff-hunk-discard-0");
    let enter = egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    diff.key(enter);
    let sent = diff.actions();
    assert!(
        matches!(sent.as_slice(), [Job::DiffAction(..)]),
        "{} jobs",
        sent.len()
    );
    assert!(diff.app.info.is_none());
}

#[test]
fn undo_of_an_index_change_waits_for_the_commit_but_undo_of_a_discard_does_not() {
    let mut diff = locked_diff("/a", "/a");
    let (root, file) = (std::path::Path::new("/a"), std::path::Path::new("file.rs"));
    for (index, enabled) in [(true, false), (false, true)] {
        let undo = diff::actions::Undo::fixture(root, file, index);
        let review = diff.app.diff_reviews.entry(diff.tab.key()).or_default();
        review.undo = vec![undo];
        diff.frames(vec![vec![], vec![]]);
        let shown = fixture_target(&diff.ctx, "diff-undo").is_some();
        assert_eq!(shown, enabled, "Undo enabled for an index change: {index}");
    }
    // Undo of a stage runs again once the push starts.
    diff.app
        .diff_reviews
        .entry(diff.tab.key())
        .or_default()
        .undo = vec![diff::actions::Undo::fixture(root, file, true)];
    assert!(diff.app.git.commit_pushing());
    diff.frames(vec![vec![], vec![]]);
    assert!(fixture_target(&diff.ctx, "diff-undo").is_some());
}

#[test]
fn a_commit_in_another_repository_leaves_diff_staging_alone() {
    let mut diff = locked_diff("/b", "/a");
    diff.frames(vec![vec![], vec![]]);
    assert!(fixture_target(&diff.ctx, "diff-stage-file").is_some());
    diff.key(stage_key());
    let sent = diff.actions();
    assert!(
        matches!(sent.as_slice(), [Job::DiffAction(..)]),
        "{} jobs",
        sent.len()
    );
}
