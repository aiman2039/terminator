//! Compare with… and the Git commands in Find Action.
use super::*;
use crate::git_log::HistoryEntry;
use crate::palette::PaletteItem;
use crate::revision_compare::PickerMode;
use crate::workspace_ops::{Op, Report};

const FILE: &str = "/a/src/lib.rs";

fn git_context(changes: Vec<terminator_git::Change>) -> services::ContextData {
    services::ContextData {
        cwd: "/a".into(),
        root: Some("/a".into()),
        perforce: None,
        git_dirs: vec![],
        branch: "main".into(),
        changes,
        decorations: HashMap::default(),
        stats: HashMap::default(),
        error: None,
    }
}

fn change(status: &str) -> terminator_git::Change {
    terminator_git::Change {
        path: FILE.into(),
        status: status.into(),
    }
}

/// Project `a` with an editor on `FILE` focused, a Git context, and a job
/// queue to read.
fn editing() -> (App, egui::Context, tempfile::TempDir, mpsc::Receiver<Job>) {
    let (mut app, ctx, dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.selected = Some("a".into());
    app.context = Some(git_context(vec![]));
    let editor = Tab::NativeEditor { path: FILE.into() };
    app.layouts.insert(
        "a".into(),
        Workspace::from_layout(DockState::new(vec![editor])),
    );
    (app, ctx, dir, requests)
}

fn paint_picker(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
    let input = egui::RawInput {
        events,
        ..Default::default()
    };
    let mut output = ctx.run_ui(input, |ui| app.show_compare_picker(ui.ctx()));
    output.textures_delta.clear();
}

fn workspace_job(requests: &mpsc::Receiver<Job>) -> (PathBuf, Op) {
    match requests.try_recv() {
        Ok(Job::Workspace(cwd, op)) => (cwd, op),
        _ => panic!("expected a workspace job"),
    }
}

fn reply(
    app: &mut App,
    ctx: &egui::Context,
    (cwd, op): (PathBuf, Op),
    result: Result<Report, String>,
) {
    app.update_tx
        .send(Update::Workspace(cwd, op, result))
        .unwrap();
    drain_updates(app, ctx);
}

fn diff(left: &str) -> Tab {
    Tab::RevisionDiff {
        cwd: "/a/src".into(),
        path: FILE.into(),
        left: left.into(),
        right: String::new(),
    }
}

fn listed(app: &App, action: Action) -> bool {
    app.palette_items()
        .iter()
        .any(|item| matches!(item, PaletteItem::Action(shown) if *shown == action))
}

#[test]
fn compare_with_lists_refs_and_opens_the_picked_branch_against_the_working_file() {
    let (mut app, ctx, _dir, requests) = editing();
    app.run_shortcut(&ctx, Action::GitCompareWith);
    let job = workspace_job(&requests);
    assert_eq!(
        job.0,
        PathBuf::from("/a/src"),
        "Git runs in the file's folder"
    );
    assert!(matches!(job.1, Op::CompareTargets));
    assert!(
        app.command_dialog_open(),
        "global shortcuts wait for the popup"
    );
    let refs = "refs/heads/main\t*\t1\t\nrefs/heads/feature\t \t2\t\n";
    let targets = crate::revision_compare::parse_targets(refs);
    reply(&mut app, &ctx, job, Ok(Report::CompareTargets(targets)));
    paint_picker(&mut app, &ctx, vec![]);
    let keys = vec![key_press(egui::Key::ArrowDown), key_press(egui::Key::Enter)];
    paint_picker(&mut app, &ctx, keys);
    assert!(app.git.picker.is_none(), "a pick closes the popup");
    assert!(app.layouts["a"].contains(&diff("feature")));
    assert!(requests.try_recv().is_err(), "a listed ref needs no check");
}

#[test]
fn a_typed_revision_is_checked_with_git_before_its_diff_opens() {
    let (mut app, ctx, _dir, requests) = editing();
    app.open_compare_picker(Path::new(FILE), PickerMode::Refs);
    let job = workspace_job(&requests);
    reply(&mut app, &ctx, job, Ok(Report::CompareTargets(vec![])));
    paint_picker(&mut app, &ctx, vec![]);
    let enter = || vec![key_press(egui::Key::Enter)];
    // Text Git would read as an option is refused without a Git call.
    app.git.picker.as_mut().unwrap().query = "--output=/tmp/x".into();
    paint_picker(&mut app, &ctx, enter());
    assert!(requests.try_recv().is_err());
    let note = app
        .git
        .picker
        .as_ref()
        .and_then(|picker| picker.note.clone());
    assert!(note.is_some_and(|note| note.contains("is not a revision")));
    // Other text goes to rev-parse; a miss keeps the popup open.
    app.git.picker.as_mut().unwrap().query = "abc123".into();
    paint_picker(&mut app, &ctx, enter());
    let job = workspace_job(&requests);
    assert!(matches!(&job.1, Op::VerifyRevision(text) if text == "abc123"));
    let missed = Err("No commit matches 'abc123'".to_owned());
    reply(&mut app, &ctx, job, missed);
    let picker = app.git.picker.as_ref().unwrap();
    assert_eq!(picker.note.as_deref(), Some("No commit matches 'abc123'"));
    assert!(picker.checking.is_none());
    assert!(!app.layouts["a"].contains(&diff("abc123")));
    // A hit opens the diff against the commit it named.
    paint_picker(&mut app, &ctx, enter());
    let job = workspace_job(&requests);
    let hash = "b".repeat(40);
    reply(&mut app, &ctx, job, Ok(Report::Revision(hash.clone())));
    assert!(app.git.picker.is_none());
    assert!(app.layouts["a"].contains(&diff(&hash)));
}

#[test]
fn git_panel_compare_with_revision_lists_history_and_opens_the_picked_commit() {
    let (mut app, ctx, _dir, requests) = editing();
    let mut outcome = Some(sidebar_ui::GitPanelOutcome {
        compare: Some((FILE.into(), PickerMode::History)),
        ..Default::default()
    });
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        if let Some(outcome) = outcome.take() {
            app.perform_git_outcome(ui, outcome);
        }
    });
    output.textures_delta.clear();
    let job = workspace_job(&requests);
    assert!(matches!(&job.1, Op::FileHistory(path) if path == Path::new(FILE)));
    let entry = |hash: &str, subject: &str| HistoryEntry {
        hash: hash.into(),
        short_hash: hash.chars().take(7).collect(),
        author: "Ada".into(),
        date: "2024-03-01".into(),
        subject: subject.into(),
    };
    let history = vec![
        entry(&"c".repeat(40), "Grow"),
        entry(&"d".repeat(40), "Add"),
    ];
    reply(&mut app, &ctx, job, Ok(Report::History(history)));
    paint_picker(&mut app, &ctx, vec![]);
    app.git.picker.as_mut().unwrap().query = "add".into();
    paint_picker(&mut app, &ctx, vec![key_press(egui::Key::Enter)]);
    assert!(app.layouts["a"].contains(&diff(&"d".repeat(40))));
}

#[test]
fn remote_files_get_a_message_instead_of_the_popup() {
    let (mut app, _ctx, _dir, requests) = editing();
    let snapshot = app.paths.data.join("remote-files/p/src/x.rs");
    app.open_compare_picker(&snapshot, PickerMode::Refs);
    assert!(app.git.picker.is_none());
    assert!(requests.try_recv().is_err());
    assert_eq!(
        app.info.as_deref(),
        Some(crate::revision_compare::REMOTE_NOTE)
    );
}

#[test]
fn git_actions_are_listed_in_find_action_only_where_they_can_run() {
    let (mut app, _ctx, _dir, _requests) = editing();
    for action in [
        Action::GitCommit,
        Action::GitRefresh,
        Action::GitShowLog,
        Action::GitCompareWith,
        Action::GitCompareRevision,
        Action::GitShowHistory,
        Action::GitAnnotate,
    ] {
        assert!(listed(&app, action), "{}", action.id());
    }
    // Nothing staged or changed: no Commit and Push, no Stage All.
    assert!(!listed(&app, Action::GitCommitPush));
    assert!(!listed(&app, Action::GitStageAll));
    app.context = Some(git_context(vec![change("M ")]));
    app.git.commit = "Fix".into();
    assert!(listed(&app, Action::GitCommitPush));
    assert!(listed(&app, Action::GitStageAll));
    // No repository and no file: none of them.
    app.context = None;
    app.layouts.insert("a".into(), Workspace::empty());
    for &action in crate::app::actions::GIT_ACTIONS {
        assert!(!listed(&app, action), "{}", action.id());
    }
}

#[test]
fn git_actions_run_the_git_panel_and_file_commands() {
    let (mut app, ctx, _dir, requests) = editing();
    app.context = Some(git_context(vec![change(" M")]));
    app.preferences.visible = false;
    app.git.history = true;
    app.run_shortcut(&ctx, Action::GitCommit);
    assert!(app.preferences.visible);
    assert_eq!(app.preferences.tool, SidebarTool::Git);
    assert!(app.git.focus_message && !app.git.history);
    app.run_shortcut(&ctx, Action::GitStageAll);
    let (root, op) = workspace_job(&requests);
    assert_eq!(root, PathBuf::from("/a"));
    assert!(matches!(&op, Op::StageAll(paths) if *paths == [PathBuf::from(FILE)]));
    app.git.list_root = Some("/a".into());
    app.run_shortcut(&ctx, Action::GitRefresh);
    assert!(app.git.list_root.is_none());
    app.run_shortcut(&ctx, Action::GitAnnotate);
    assert!(app.editor_blame.contains_key(Path::new(FILE)));
    app.run_shortcut(&ctx, Action::GitShowHistory);
    let history = Tab::FileHistory {
        cwd: "/a".into(),
        path: FILE.into(),
    };
    assert!(app.layouts["a"].contains(&history));
    app.run_shortcut(&ctx, Action::GitShowLog);
    assert!(app.layouts["a"].contains(&Tab::CommitLog { cwd: "/a".into() }));
}

#[test]
fn commit_and_push_action_runs_the_commit_job_with_push() {
    let (mut app, ctx, _dir, requests) = editing();
    app.context = Some(git_context(vec![change("M ")]));
    app.git.commit = "Fix the parser".into();
    app.run_shortcut(&ctx, Action::GitCommitPush);
    let (_, op) = workspace_job(&requests);
    assert!(matches!(
        &op,
        Op::Commit { message, push: true, .. } if message == "Fix the parser"
    ));
    assert!(app.git.run.is_some(), "the panel shows its progress");
    assert_eq!(app.preferences.tool, SidebarTool::Git);
}

#[cfg(feature = "test-support")]
#[test]
fn explorer_file_menu_opens_the_compare_popup() {
    let (mut app, ctx, dir) = fixture();
    let root = dir.path().to_path_buf();
    fs::write(root.join("a.rs"), "").unwrap();
    let entry = terminator_git::Entry {
        path: root.join("a.rs"),
        directory: false,
        ignored: false,
    };
    app.dirs.insert(root.clone(), vec![entry]);
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    let mut context = git_context(vec![]);
    context.root = Some(root.clone());
    app.context = Some(context);
    let draw = |app: &mut App, events: Vec<egui::Event>| {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(420.0, 480.0),
            )),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| app.tree(ui, &root, 0));
        output.textures_delta.clear();
    };
    draw(&mut app, vec![]);
    let row = crate::diagnostics::current_rect(&ctx, "explorer-file:a.rs").unwrap();
    let click = |pos, button, pressed| egui::Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    let secondary = egui::PointerButton::Secondary;
    draw(&mut app, vec![egui::Event::PointerMoved(row.center())]);
    draw(&mut app, vec![click(row.center(), secondary, true)]);
    draw(&mut app, vec![click(row.center(), secondary, false)]);
    let mut item = None;
    for _ in 0..3 {
        draw(&mut app, vec![]);
        item = crate::diagnostics::current_rect(&ctx, "Compare with Revision…");
    }
    let item = item.expect("the Git items are in the file menu").center();
    let primary = egui::PointerButton::Primary;
    draw(&mut app, vec![egui::Event::PointerMoved(item)]);
    draw(&mut app, vec![click(item, primary, true)]);
    draw(&mut app, vec![click(item, primary, false)]);
    let (cwd, op) = workspace_job(&requests);
    assert_eq!(cwd, root);
    assert!(matches!(&op, Op::FileHistory(path) if *path == root.join("a.rs")));
    assert!(app.git.picker.is_some());
}
