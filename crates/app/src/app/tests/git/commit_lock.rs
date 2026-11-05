//! A running commit locks the actions that could change it, in its own
//! repository only, until the commit ends or its push starts.
use super::git_panel::{click_commit, reply, run_git_outcome, sent_commit};
use super::*;
use crate::workspace_ops::{Op, Report};
use sidebar_ui::GitPanelOutcome;

/// What the status bar says when a running commit refuses an action.
const RUNNING: &str = "A commit is running in this repository. Wait for it to finish.";

fn change(path: &Path, status: &str) -> terminator_git::Change {
    terminator_git::Change {
        path: path.into(),
        status: status.into(),
    }
}

/// The worktree `root` of the repository whose worktrees share the Git
/// common dir `common`, with a staged and an edited file.
fn repository(root: &str, common: &str) -> services::ContextData {
    let root = PathBuf::from(root);
    let common = PathBuf::from(common);
    let git_dir = if common == root.join(".git") {
        common.clone()
    } else {
        common.join("worktrees").join(root.file_name().unwrap())
    };
    services::ContextData {
        cwd: root.clone(),
        root: Some(root.clone()),
        perforce: None,
        git_dirs: vec![git_dir, common],
        branch: "main".into(),
        changes: vec![
            change(&root.join("staged.rs"), "M "),
            change(&root.join("edited.rs"), " M"),
        ],
        decorations: HashMap::default(),
        stats: HashMap::default(),
        error: None,
    }
}

/// Project `a` on its repository `/a`, and a commit (with a push when
/// `push`) that the Git panel started there.
struct Committing {
    app: App,
    ctx: egui::Context,
    _dir: tempfile::TempDir,
    requests: mpsc::Receiver<Job>,
    commit: (PathBuf, Op),
}

fn committing(push: bool) -> Committing {
    let (mut app, ctx, dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.selected = Some("a".into());
    app.context = Some(repository("/a", "/a/.git"));
    app.git.commit = "msg".into();
    click_commit(&mut app, &ctx, push);
    let commit = sent_commit(&requests);
    Committing {
        app,
        ctx,
        _dir: dir,
        requests,
        commit,
    }
}

impl Committing {
    fn outcome(&mut self, outcome: GitPanelOutcome) {
        run_git_outcome(&mut self.app, &self.ctx, outcome);
    }

    /// The workspace ops sent since the last call.
    fn sent(&self) -> Vec<(PathBuf, Op)> {
        self.requests
            .try_iter()
            .filter_map(|job| match job {
                Job::Workspace(root, op) => Some((root, op)),
                _ => None,
            })
            .collect()
    }

    /// `act` sends nothing, and the status bar says why.
    fn assert_refused(&mut self, act: impl FnOnce(&mut Self)) {
        self.app.info = None;
        act(self);
        assert_eq!(self.app.info.as_deref(), Some(RUNNING));
        let sent: Vec<Job> = self.requests.try_iter().collect();
        assert!(sent.is_empty(), "a refused action sends nothing");
    }
}

fn stage(path: impl Into<PathBuf>) -> GitPanelOutcome {
    GitPanelOutcome {
        stage: vec![path.into()],
        ..Default::default()
    }
}

fn shelve() -> GitPanelOutcome {
    GitPanelOutcome {
        shelve: true,
        ..Default::default()
    }
}

fn switch() -> GitPanelOutcome {
    GitPanelOutcome {
        switch: Some("feature".into()),
        ..Default::default()
    }
}

fn unshelve() -> GitPanelOutcome {
    GitPanelOutcome {
        unshelve: Some("stash@{0}".into()),
        ..Default::default()
    }
}

fn gutter_block() -> terminator_native_edit::view::ChangeMark {
    terminator_native_edit::view::ChangeMark {
        start: 0,
        end: 1,
        kind: terminator_native_edit::view::ChangeKind::Modified,
    }
}

#[test]
fn actions_that_could_change_a_running_commit_are_refused() {
    let mut run = committing(false);
    let refused = [
        stage("/a/edited.rs"),
        GitPanelOutcome {
            unstage: vec!["/a/staged.rs".into()],
            ..Default::default()
        },
        GitPanelOutcome {
            stage_all: true,
            ..Default::default()
        },
        shelve(),
        unshelve(),
        switch(),
    ];
    for outcome in refused {
        run.assert_refused(|run| run.outcome(outcome));
    }
    // The Discard and Drop confirmation bars send through the same queue.
    let discard = Op::Discard {
        path: "/a/edited.rs".into(),
        untracked: false,
    };
    for op in [discard, Op::DropShelf("stash@{0}".into())] {
        run.assert_refused(|run| {
            run.app.queue_workspace(op);
        });
    }
    run.assert_refused(|run| run.app.run_shortcut(&run.ctx, Action::GitStageAll));
    run.assert_refused(|run| {
        let path = Path::new("/a/edited.rs");
        run.app
            .stage_gutter_block(path, gutter_block(), "text\n".into());
    });
    // Reads and Explorer file operations still run.
    run.app.queue_workspace(Op::Branches);
    run.app.queue_workspace(Op::CreateFile("/a/new.rs".into()));
    let sent = run.sent();
    assert!(
        matches!(sent.as_slice(), [(_, Op::Branches), (_, Op::CreateFile(_))]),
        "{sent:?}"
    );
    assert!(run.app.git.run.is_some(), "the commit still runs");
}

#[test]
fn locked_actions_run_again_once_the_push_starts_or_the_commit_ends() {
    let mut run = committing(true);
    run.assert_refused(|run| run.outcome(stage("/a/edited.rs")));
    // The commit landed; its push runs. Index and stash writers run again.
    let job = run.commit.clone();
    reply(&mut run.app, &run.ctx, job.clone(), Ok(Report::Pushing));
    run.outcome(stage("/a/edited.rs"));
    run.outcome(shelve());
    let sent = run.sent();
    assert!(
        matches!(sent.as_slice(), [(_, Op::Stage(_)), (_, Op::Shelve(_))]),
        "{sent:?}"
    );
    // `git push` reads HEAD when it starts: the branch switch waits for it.
    run.assert_refused(|run| run.outcome(switch()));
    let pushed = Ok(Report::Message("Committed and pushed".into()));
    reply(&mut run.app, &run.ctx, job, pushed);
    run.outcome(switch());
    let sent = run.sent();
    assert!(matches!(sent.as_slice(), [(_, Op::Switch(_))]), "{sent:?}");
    // A commit that fails or is cancelled unlocks too, once its job
    // replies: while Cancel stops Git, the lock holds.
    let failed = "Commit failed (exit status: 1)\nhook said no".to_owned();
    for end in [failed, workspace_ops::COMMIT_CANCELLED.to_owned()] {
        run.app.git.commit = "next".into();
        click_commit(&mut run.app, &run.ctx, false);
        let job = sent_commit(&run.requests);
        run.outcome(GitPanelOutcome {
            cancel_commit: true,
            ..Default::default()
        });
        assert!(
            run.app
                .git
                .run
                .as_ref()
                .is_some_and(|run| run.progress().cancelling)
        );
        run.assert_refused(|run| run.outcome(stage("/a/edited.rs")));
        reply(&mut run.app, &run.ctx, job, Err(end));
        run.outcome(stage("/a/edited.rs"));
        let sent = run.sent();
        assert!(matches!(sent.as_slice(), [(_, Op::Stage(_))]), "{sent:?}");
    }
}

#[test]
fn a_running_commit_locks_only_its_own_repository() {
    let mut run = committing(false);
    // Another repository: every action runs there.
    run.app.context = Some(repository("/b", "/b/.git"));
    for outcome in [stage("/b/edited.rs"), shelve(), switch()] {
        run.outcome(outcome);
    }
    let sent = run.sent();
    assert_eq!(sent.len(), 3, "{sent:?}");
    assert!(sent.iter().all(|(root, _)| root == Path::new("/b")));
    assert!(run.app.info.is_none());
    // Another worktree of the same repository has its own index and HEAD,
    // but every worktree shares the stash.
    run.app.context = Some(repository("/w", "/a/.git"));
    run.outcome(stage("/w/edited.rs"));
    run.outcome(switch());
    let sent = run.sent();
    assert!(
        matches!(sent.as_slice(), [(_, Op::Stage(_)), (_, Op::Switch(_))]),
        "{sent:?}"
    );
    let lock = run.app.git_panel_lock(run.app.context.as_ref().unwrap());
    assert_eq!(lock.index, None, "the panel stages in its own worktree");
    assert_eq!(lock.stash, Some("Wait for the commit to finish"));
    run.assert_refused(|run| run.outcome(shelve()));
    run.assert_refused(|run| run.outcome(unshelve()));
    run.assert_refused(|run| {
        run.app.queue_workspace(Op::DropShelf("stash@{0}".into()));
    });
}

#[test]
fn a_gutter_stage_waits_only_for_a_commit_in_its_own_repository() {
    let mut run = committing(true);
    let file = |root: &str| PathBuf::from(root).join("src/edited.rs");
    for root in ["/a", "/b"] {
        let entry = (0, Some("old\n".into()), Some(PathBuf::from(root)));
        run.app.git_bases.insert(file(root), entry);
    }
    let stage = |run: &mut Committing, root: &str| {
        let path = file(root);
        run.app
            .stage_gutter_block(&path, gutter_block(), "new\n".into());
    };
    // The job runs Git in the repository root, so it queues with that
    // root's actions and commit (see `JobRouting`).
    let staged_in = |run: &Committing| -> Vec<PathBuf> {
        run.requests
            .try_iter()
            .filter_map(|job| match job {
                Job::StageGutterBlock { cwd, .. } => Some(cwd),
                _ => None,
            })
            .collect()
    };
    run.assert_refused(|run| stage(run, "/a"));
    // Another repository runs.
    stage(&mut run, "/b");
    assert_eq!(staged_in(&run), [PathBuf::from("/b")]);
    // Once the push starts, the commit's own repository runs too.
    let job = run.commit.clone();
    reply(&mut run.app, &run.ctx, job, Ok(Report::Pushing));
    stage(&mut run, "/a");
    assert_eq!(staged_in(&run), [PathBuf::from("/a")]);
    // A file whose repository is not known yet runs Git in its folder.
    let unknown = PathBuf::from("/c/src/new.rs");
    let entry = (0, Some("old\n".into()), None);
    run.app.git_bases.insert(unknown.clone(), entry);
    run.app
        .stage_gutter_block(&unknown, gutter_block(), "new\n".into());
    assert_eq!(staged_in(&run), [PathBuf::from("/c/src")]);
}

/// One frame of the Git sidebar.
fn paint_git(run: &mut Committing) {
    let app = &mut run.app;
    let mut output = run.ctx.run_ui(egui::RawInput::default(), |ui| {
        app.sidebar_tool(SidebarTool::Git, ui);
    });
    output.textures_delta.clear();
}

#[test]
fn a_commit_that_does_not_land_reloads_the_shelves_and_closes_a_drop_question() {
    let failed = "Commit failed (exit status: 1)\nhook said no".to_owned();
    for end in [failed, workspace_ops::COMMIT_CANCELLED.to_owned()] {
        let mut run = committing(false);
        run.app.git.shelves = vec![("stash@{0}".into(), "WIP".into())];
        run.app.git.list_root = Some("/a".into());
        let question = sidebar_ui::GitConfirm::DropShelf("stash@{0}".into());
        run.app.git.confirm = Some(question);
        // A hook stopped between its `stash push` and `stash pop` leaves
        // its own entry at stash@{0}; the shelf moved to stash@{1}.
        let job = run.commit.clone();
        reply(&mut run.app, &run.ctx, job, Err(end));
        assert!(
            run.app.git.confirm.is_none(),
            "a Drop question names a position"
        );
        paint_git(&mut run);
        assert!(run.app.git.shelves.is_empty(), "old positions are gone");
        let sent = run.sent();
        assert!(
            sent.iter().any(|(_, op)| matches!(op, Op::Shelves)),
            "{sent:?}"
        );
    }
}

#[test]
fn find_action_lists_stage_all_only_when_no_commit_locks_it() {
    let mut run = committing(true);
    let listed = |app: &App| {
        app.palette_items().iter().any(|item| {
            matches!(
                item,
                crate::palette::PaletteItem::Action(Action::GitStageAll)
            )
        })
    };
    assert!(!listed(&run.app), "a running commit hides Stage All");
    let job = run.commit.clone();
    reply(&mut run.app, &run.ctx, job, Ok(Report::Pushing));
    assert!(
        listed(&run.app),
        "Stage All runs again once the push starts"
    );
}

#[test]
fn a_refusal_shows_in_place_of_an_older_status_error() {
    let mut run = committing(false);
    // The status bar shows an error before a notice.
    run.app.error = Some("Commit failed (exit status: 1)".into());
    run.assert_refused(|run| run.outcome(stage("/a/edited.rs")));
    assert!(run.app.error.is_none(), "the refusal is what the bar shows");
}

/// Real Git and a `pre-commit` hook (a shell script).
#[cfg(unix)]
mod hook {
    use super::*;

    fn git(cwd: &Path, args: &[&str]) -> String {
        let options = CommandOptions {
            timeout: Duration::from_secs(20),
            ..Default::default()
        };
        let output = run_command(terminator_core::git::command(cwd, args), options).unwrap();
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }

    /// Pump updates the way frames do until `ready`.
    fn pump(app: &mut App, ctx: &egui::Context, ready: impl Fn(&App) -> bool) {
        let started = Instant::now();
        while !ready(app) {
            assert!(started.elapsed() < Duration::from_secs(30), "timed out");
            app.process_updates(ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// A repository with `a.txt` changed and staged, whose `pre-commit` hook
    /// writes `hook.pid` and then waits for a `release` file.
    fn repository_with_waiting_hook(dir: &Path) -> PathBuf {
        let (root, hooks) = (dir.join("repo"), dir.join("hooks"));
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&hooks).unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        for (key, value) in [
            ("user.email", "lock@example.com"),
            ("user.name", "Lock"),
            ("commit.gpgsign", "false"),
            ("core.hooksPath", &hooks.to_string_lossy()),
        ] {
            git(&root, &["config", key, value]);
        }
        fs::write(root.join("a.txt"), "base\n").unwrap();
        git(&root, &["add", "a.txt"]);
        git(&root, &["commit", "-qm", "base"]);
        fs::write(root.join("a.txt"), "next\n").unwrap();
        git(&root, &["add", "a.txt"]);
        let (pid, release) = (dir.join("hook.pid"), dir.join("release"));
        let script = format!(
            "#!/bin/sh\necho $$ > '{}'\ni=0\nwhile [ ! -f '{}' ] && [ $i -lt 600 ]; do sleep 0.05; i=$((i+1)); done\n",
            pid.display(),
            release.display()
        );
        let hook = hooks.join("pre-commit");
        fs::write(&hook, script).unwrap();
        let executable = std::os::unix::fs::PermissionsExt::from_mode(0o755);
        fs::set_permissions(&hook, executable).unwrap();
        root.canonicalize().unwrap()
    }

    #[test]
    fn a_stage_asked_for_during_the_pre_commit_hook_stays_out_of_the_commit() {
        let dir = tempfile::Builder::new()
            .prefix("commit-lock-")
            .tempdir_in("/tmp")
            .unwrap();
        let root = repository_with_waiting_hook(dir.path());
        let (mut app, ctx, _data) = fixture();
        app.selected = Some("a".into());
        app.context = Some(services::ContextData {
            cwd: root.clone(),
            root: Some(root.clone()),
            perforce: None,
            git_dirs: vec![root.join(".git"), root.join(".git")],
            branch: "main".into(),
            changes: vec![change(&root.join("a.txt"), "M ")],
            decorations: HashMap::default(),
            stats: HashMap::default(),
            error: None,
        });
        app.git.commit = "next".into();
        click_commit(&mut app, &ctx, false);
        let started = Instant::now();
        while !dir.path().join("hook.pid").exists() {
            assert!(started.elapsed() < Duration::from_secs(20), "no hook");
            std::thread::sleep(Duration::from_millis(5));
        }
        // Git reads the index again after `pre-commit`: a Stage now would be
        // in the commit.
        fs::write(root.join("b.txt"), "b\n").unwrap();
        run_git_outcome(&mut app, &ctx, stage(root.join("b.txt")));
        assert_eq!(app.info.as_deref(), Some(RUNNING));
        // A Stage that went out would finish before this read on its queue.
        app.queue_workspace(Op::Branches);
        pump(&mut app, &ctx, |app| !app.git.branches.is_empty());
        fs::write(dir.path().join("release"), "").unwrap();
        pump(&mut app, &ctx, |app| app.git.run.is_none());
        assert!(
            app.git.commit_failure.is_none(),
            "{:?}",
            app.git.commit_failure
        );
        assert_eq!(git(&root, &["log", "-1", "--format=%s"]), "next");
        assert_eq!(
            git(&root, &["show", "--name-only", "--format=", "HEAD"]),
            "a.txt"
        );
        assert_eq!(
            git(&root, &["status", "--porcelain", "--", "b.txt"]),
            "?? b.txt"
        );
        // Once the commit ends, the same Stage runs.
        run_git_outcome(&mut app, &ctx, stage(root.join("b.txt")));
        pump(&mut app, &ctx, |app| {
            app.info.as_deref() == Some("Staged b.txt")
        });
        assert_eq!(git(&root, &["diff", "--cached", "--name-only"]), "b.txt");
    }
}

/// One Git sidebar frame at a clock that moves 0.1 s per frame.
#[cfg(feature = "test-support")]
struct GitPane {
    time: f64,
}

#[cfg(feature = "test-support")]
impl GitPane {
    fn paint(&mut self, run: &mut Committing, events: Vec<egui::Event>) -> Vec<String> {
        self.paint_at(run, events)
            .into_iter()
            .map(|(_, text)| text)
            .collect()
    }

    /// One frame; returns each painted text with its top-left corner.
    fn paint_at(
        &mut self,
        run: &mut Committing,
        events: Vec<egui::Event>,
    ) -> Vec<(egui::Pos2, String)> {
        self.time += 0.1;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(360.0, 700.0),
            )),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let app = &mut run.app;
        let mut output = run
            .ctx
            .run_ui(input, |ui| app.sidebar_tool(SidebarTool::Git, ui));
        output.textures_delta.clear();
        crate::app::painted_text(&output.shapes)
    }

    /// Where `text` is painted now.
    fn text(&mut self, run: &mut Committing, text: &str) -> egui::Pos2 {
        self.paint_at(run, vec![])
            .into_iter()
            .find_map(|(at, painted)| (painted == text).then_some(at))
            .unwrap_or_else(|| panic!("{text} is painted"))
    }

    /// The group checkbox, just left of the group's `LABEL count`.
    fn group_checkbox(&mut self, run: &mut Committing, label: &str) -> egui::Pos2 {
        let at = self.text(run, label);
        egui::pos2(at.x - 9.0, at.y + 5.0)
    }

    /// Rests the pointer on `at`; returns the texts painted then.
    fn hover(&mut self, run: &mut Committing, at: egui::Pos2) -> Vec<String> {
        self.paint(run, vec![egui::Event::PointerMoved(at)]);
        let mut texts = Vec::new();
        for _ in 0..12 {
            texts = self.paint(run, vec![]);
        }
        texts
    }

    fn click(&mut self, run: &mut Committing, pos: egui::Pos2, button: egui::PointerButton) {
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        self.paint(run, vec![egui::Event::PointerMoved(pos)]);
        self.paint(run, vec![press(true)]);
        self.paint(run, vec![press(false)]);
        self.paint(run, vec![]);
    }

    fn target(run: &Committing, name: &str) -> egui::Rect {
        agent_target(&run.ctx, name).unwrap_or_else(|| panic!("{name} is painted"))
    }

    /// Opens the context menu of the file row `file`, then clicks `item`.
    fn menu_item(&mut self, run: &mut Committing, file: &str, item: &str) {
        let row = Self::target(run, &format!("git-file-{file}")).center();
        self.click(run, row, egui::PointerButton::Secondary);
        let mut rect = None;
        for _ in 0..3 {
            self.paint(run, vec![]);
            rect = crate::diagnostics::current_rect(&run.ctx, item);
        }
        let item = rect
            .unwrap_or_else(|| panic!("{item} is in the menu"))
            .center();
        self.click(run, item, egui::PointerButton::Primary);
        self.paint(run, vec![key_press(egui::Key::Escape)]);
    }
}

#[cfg(feature = "test-support")]
#[test]
fn git_panel_controls_that_could_change_the_commit_are_disabled_while_it_runs() {
    let mut run = committing(true);
    run.app.git.shelves = vec![("stash@{0}".into(), "WIP".into())];
    run.app.git.branches = vec!["main".into(), "feature".into()];
    run.app.git.list_root = Some("/a".into());
    let mut pane = GitPane { time: 1.0 };
    let primary = egui::PointerButton::Primary;
    pane.paint(&mut run, vec![]);
    for name in [
        "git-stage-all",
        "git-shelve",
        "git-shelf-unshelve-stash@{0}",
        "git-shelf-drop-stash@{0}",
    ] {
        let at = GitPane::target(&run, name).center();
        pane.click(&mut run, at, primary);
    }
    // The checkboxes of the file row and its group, the group's Stage all,
    // and the row's menu items.
    let row = GitPane::target(&run, "git-file-edited.rs");
    let checkbox = egui::pos2(row.left() - 9.0, row.center().y);
    pane.click(&mut run, checkbox, primary);
    let group = pane.group_checkbox(&mut run, "CHANGES 1");
    pane.click(&mut run, group, primary);
    let stage_group = GitPane::target(&run, "git-group-Stage all").center();
    pane.click(&mut run, stage_group, primary);
    for item in ["Stage", "Discard changes"] {
        pane.menu_item(&mut run, "edited.rs", item);
    }
    // The branch menu does not open.
    let branch = GitPane::target(&run, "git-branch").center();
    pane.click(&mut run, branch, primary);
    assert!(crate::diagnostics::current_rect(&run.ctx, "feature").is_none());
    pane.paint(&mut run, vec![key_press(egui::Key::Escape)]);
    assert!(run.sent().is_empty());
    assert!(run.app.git.confirm.is_none() && run.app.pending_delete.is_none());
    assert!(run.app.info.is_none(), "disabled controls refuse nothing");
    // A disabled control says why.
    let wait = "Wait for the commit to finish".to_owned();
    for name in ["git-stage-all", "git-group-Stage all"] {
        let at = GitPane::target(&run, name).center();
        assert!(pane.hover(&mut run, at).contains(&wait), "{name}");
    }
    // The commit landed; its push runs. (The landed commit reloads the
    // branch, log, shelf, and compare lists.)
    let job = run.commit.clone();
    reply(&mut run.app, &run.ctx, job.clone(), Ok(Report::Pushing));
    pane.paint(&mut run, vec![]);
    // Staging runs again. The shelf list reloads, so the rows move up.
    let stage_all = GitPane::target(&run, "git-stage-all").center();
    pane.click(&mut run, stage_all, primary);
    let row = GitPane::target(&run, "git-file-edited.rs");
    let checkbox = egui::pos2(row.left() - 9.0, row.center().y);
    pane.click(&mut run, checkbox, primary);
    let group = pane.group_checkbox(&mut run, "CHANGES 1");
    pane.click(&mut run, group, primary);
    let stage_group = GitPane::target(&run, "git-group-Stage all").center();
    pane.click(&mut run, stage_group, primary);
    let sent = run.sent();
    let count = |wanted: fn(&Op) -> bool| sent.iter().filter(|(_, op)| wanted(op)).count();
    assert_eq!(count(|op| matches!(op, Op::StageAll(_))), 1, "{sent:?}");
    let edited = |op: &Op| matches!(op, Op::Stage(path) if path.ends_with("edited.rs"));
    assert_eq!(count(edited), 3, "the checkboxes stage again: {sent:?}");
    // `git push` reads HEAD when it starts: the branch switch waits for
    // the push to end.
    run.app.git.branches = vec!["main".into(), "feature".into()];
    let branch = GitPane::target(&run, "git-branch").center();
    assert!(pane.hover(&mut run, branch).contains(&wait));
    pane.click(&mut run, branch, primary);
    assert!(crate::diagnostics::current_rect(&run.ctx, "feature").is_none());
    let pushed = Ok(Report::Message("Committed and pushed".into()));
    reply(&mut run.app, &run.ctx, job, pushed);
    // The push reloads the lists too.
    pane.paint(&mut run, vec![]);
    run.app.git.branches = vec!["main".into(), "feature".into()];
    let branch = GitPane::target(&run, "git-branch").center();
    pane.click(&mut run, branch, primary);
    let mut menu = None;
    for _ in 0..3 {
        pane.paint(&mut run, vec![]);
        menu = crate::diagnostics::current_rect(&run.ctx, "feature");
    }
    assert!(menu.is_some(), "the branch menu opens");
}

#[cfg(feature = "test-support")]
#[test]
fn folder_staging_waits_for_a_running_commit() {
    let mut run = committing(true);
    run.app.git.list_root = Some("/a".into());
    if let Some(context) = run.app.context.as_mut() {
        context
            .changes
            .push(change(Path::new("/a/src/lib.rs"), " M"));
    }
    let mut pane = GitPane { time: 1.0 };
    let mut stage_folder = |run: &mut Committing| {
        let folder = pane.text(run, "src") + egui::vec2(2.0, 4.0);
        pane.click(run, folder, egui::PointerButton::Secondary);
        let mut item = None;
        for _ in 0..3 {
            pane.paint(run, vec![]);
            item = crate::diagnostics::current_rect(&run.ctx, "Stage folder");
        }
        let item = item.expect("Stage folder is in the menu").center();
        pane.click(run, item, egui::PointerButton::Primary);
        pane.paint(run, vec![key_press(egui::Key::Escape)]);
        run.sent()
    };
    assert!(stage_folder(&mut run).is_empty());
    let job = run.commit.clone();
    reply(&mut run.app, &run.ctx, job, Ok(Report::Pushing));
    let sent = stage_folder(&mut run);
    assert!(
        sent.iter()
            .any(|(_, op)| matches!(op, Op::Stage(path) if path.ends_with("src/lib.rs"))),
        "{sent:?}"
    );
}

#[cfg(feature = "test-support")]
#[test]
fn a_question_asked_before_the_commit_waits_for_it_to_end() {
    let mut run = committing(false);
    let question = sidebar_ui::GitConfirm::Discard {
        path: "/a/edited.rs".into(),
        untracked: false,
    };
    run.app.git.confirm = Some(question.clone());
    run.app.git.list_root = Some("/a".into());
    let mut pane = GitPane { time: 1.0 };
    let verb = pane.text(&mut run, "Discard") + egui::vec2(4.0, 4.0);
    pane.click(&mut run, verb, egui::PointerButton::Primary);
    assert!(run.sent().is_empty());
    assert_eq!(run.app.git.confirm, Some(question), "the question stays");
    let texts = pane.hover(&mut run, verb);
    assert!(
        texts
            .iter()
            .any(|text| text == "Wait for the commit to finish")
    );
    // Once the commit ends, the same click discards.
    let job = run.commit.clone();
    reply(
        &mut run.app,
        &run.ctx,
        job,
        Ok(Report::Message("Committed".into())),
    );
    pane.click(&mut run, verb, egui::PointerButton::Primary);
    let sent = run.sent();
    assert!(
        sent.iter().any(|(_, op)| matches!(op, Op::Discard { .. })),
        "{sent:?}"
    );
    assert!(run.app.git.confirm.is_none());
}

#[cfg(feature = "test-support")]
#[test]
fn delete_file_in_the_git_panel_still_moves_the_file_to_trash_during_a_commit() {
    let mut run = committing(false);
    run.app.git.list_root = Some("/a".into());
    let mut pane = GitPane { time: 1.0 };
    pane.paint(&mut run, vec![]);
    // Like the Explorer's Delete, it writes no index, HEAD, or stash.
    pane.menu_item(&mut run, "edited.rs", "Delete file");
    assert_eq!(
        run.app.pending_delete.as_deref(),
        Some(Path::new("/a/edited.rs"))
    );
    let trash = pane.text(&mut run, "Move to Trash") + egui::vec2(4.0, 4.0);
    pane.click(&mut run, trash, egui::PointerButton::Primary);
    let sent = run.sent();
    assert!(
        matches!(sent.as_slice(), [(_, Op::Delete(path))] if path == Path::new("/a/edited.rs")),
        "{sent:?}"
    );
}
