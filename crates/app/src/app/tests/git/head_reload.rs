//! Blame and File History reload when the Git refresh finds a new HEAD.
use super::*;
use crate::workspace_ops::{Op, Report};

fn app_with_views() -> (App, egui::Context, tempfile::TempDir, mpsc::Receiver<Job>) {
    let (mut app, ctx, dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.context = Some(services::ContextData {
        cwd: "/a".into(),
        root: Some("/a".into()),
        perforce: None,
        git_dirs: vec![],
        branch: "main".into(),
        changes: vec![],
        decorations: HashMap::default(),
        stats: HashMap::default(),
        error: None,
    });
    app.git_views
        .insert("blame".into(), crate::GitView::Blame(Err("x".into())));
    app.git_views
        .insert("history".into(), crate::GitView::History(Ok(Vec::new())));
    app.git_views.insert(
        "log".into(),
        crate::GitView::Log(crate::git_log::CommitLog::default()),
    );
    (app, ctx, dir, requests)
}

fn reply(app: &mut App, ctx: &egui::Context, result: Result<Report, String>) {
    app.update_tx
        .send(Update::Workspace(PathBuf::from("/a"), Op::Head, result))
        .unwrap();
    drain_updates(app, ctx);
}

#[test]
fn blame_and_history_reload_when_head_moves_and_stay_when_it_does_not() {
    let (mut app, ctx, _dir, requests) = app_with_views();
    let head = |app: &mut App, hash: &str| reply(app, &ctx, Ok(Report::Revision(hash.into())));
    app.check_head_for_git_views();
    assert!(matches!(
        requests.try_recv(),
        Ok(Job::Workspace(_, Op::Head))
    ));
    head(&mut app, "aaa");
    assert_eq!(app.git_views.len(), 3, "the first read only notes HEAD");
    head(&mut app, "aaa");
    assert_eq!(app.git_views.len(), 3, "same HEAD: nothing reloads");
    head(&mut app, "bbb");
    assert!(!app.git_views.contains_key("blame"));
    assert!(!app.git_views.contains_key("history"));
    assert!(
        app.git_views.contains_key("log"),
        "the Commit Log has its own reload"
    );
    // Without a Blame or History view, a refresh reads nothing.
    app.check_head_for_git_views();
    assert!(requests.try_recv().is_err());
    // A failed read changes nothing and says nothing.
    reply(&mut app, &ctx, Err("no HEAD".into()));
    assert_eq!(app.git.head, Some((PathBuf::from("/a"), "bbb".into())));
    assert!(app.info.is_none());
}

#[test]
fn head_reads_the_full_hash() {
    let dir = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["-c", "user.email=t@example.invalid", "-c", "user.name=T"])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    };
    git(&["init", "-q"]);
    git(&["commit", "-q", "--allow-empty", "-m", "one"]);
    let Ok(Report::Revision(hash)) = crate::workspace_ops::perform(dir.path(), Op::Head) else {
        panic!("HEAD is read");
    };
    assert_eq!(hash.len(), 40, "{hash}");
}
