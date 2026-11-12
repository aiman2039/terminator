//! A project folder or a terminal folder that was moved or deleted, such as
//! a removed Git worktree: the Explorer message and its actions.
use super::*;

fn folder_error(path: &Path, kind: std::io::ErrorKind, message: &str) -> services::DirectoryError {
    services::DirectoryError {
        path: path.into(),
        kind,
        message: message.into(),
    }
}

fn not_found(path: &Path) -> services::DirectoryError {
    folder_error(
        path,
        std::io::ErrorKind::NotFound,
        "No such file or directory (os error 2)",
    )
}

/// Project `a` keeps a live terminal, so it stays selected while its
/// folder is missing.
fn missing_root_fixture() -> (App, egui::Context, tempfile::TempDir, mpsc::Receiver<Job>) {
    let (mut app, ctx, dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.state
        .sessions
        .push(session_fixture("shell", SessionKind::Shell));
    app.insert("a", Tab::Terminal("shell".into()), None);
    app.active_session = Some("shell".into());
    app.apply_project_directories(vec![("a".into(), "/a".into(), false)]);
    app.directory_errors
        .insert("/a".into(), not_found(Path::new("/a")));
    (app, ctx, dir, requests)
}

fn with_project(app: &App, id: &str, path: &Path) -> Box<State> {
    let mut state = app.state.clone();
    state.projects.push(Project {
        id: id.into(),
        name: id.into(),
        path: path.into(),
        layout: serde_json::Value::Null,
    });
    state.revision = state.revision.saturating_add(1);
    Box::new(state)
}

fn opened_job(requests: &mpsc::Receiver<Job>) -> Option<(PathBuf, u64)> {
    requests.try_iter().find_map(|job| match job {
        Job::OpenProject(path, generation) => Some((path, generation)),
        _ => None,
    })
}

#[cfg(feature = "test-support")]
fn paint_tree(app: &mut App, ctx: &egui::Context, root: &Path, events: Vec<egui::Event>) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(320.0, 400.0),
            )),
            events,
            ..Default::default()
        },
        |ui| app.tree(ui, root, 0),
    );
    output.textures_delta.clear();
}

fn paint_projects(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(320.0, 400.0),
            )),
            events,
            ..Default::default()
        },
        |ui| app.projects(ui),
    );
    output.textures_delta.clear();
}

fn explorer_text(app: &mut App, ctx: &egui::Context, root: &Path) -> Vec<String> {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(320.0, 400.0),
            )),
            ..Default::default()
        },
        |ui| app.tree(ui, root, 0),
    );
    let text = painted_strings(&output.shapes);
    output.textures_delta.clear();
    text
}

#[cfg(feature = "test-support")]
fn click_tree_target(app: &mut App, ctx: &egui::Context, root: &Path, name: &str) {
    paint_tree(app, ctx, root, Vec::new());
    let pos = header_target(ctx)(name)
        .unwrap_or_else(|| panic!("missing {name}"))
        .center();
    paint_tree(app, ctx, root, vec![egui::Event::PointerMoved(pos)]);
    for pressed in [true, false] {
        paint_tree(
            app,
            ctx,
            root,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            }],
        );
    }
}

/// The Explorer note while the focused terminal's folder is gone.
const TERMINAL_NOTE: &str =
    "Terminal folder /a--task was moved or deleted. Showing the project folder.";

fn plain_context(cwd: &Path) -> services::ContextData {
    services::ContextData {
        cwd: cwd.into(),
        root: None,
        perforce: None,
        git_dirs: Vec::new(),
        branch: String::new(),
        changes: Vec::new(),
        decorations: HashMap::default(),
        stats: HashMap::default(),
        error: None,
    }
}

/// Project `a` (/a) with its focused shell in /a--task, for example a
/// removed Git worktree. The returned receivers get the jobs and the
/// Explorer refresh requests, so no worker reads the disk.
fn deleted_terminal_fixture() -> (
    App,
    egui::Context,
    tempfile::TempDir,
    mpsc::Receiver<Job>,
    tokio::sync::watch::Receiver<Option<refresh::Request>>,
) {
    let (mut app, ctx, dir) = fixture();
    let (jobs, sent) = mpsc::channel();
    app.jobs = jobs.into();
    let (refresh, requests) = tokio::sync::watch::channel(None);
    app.refresh = refresh;
    let mut shell = session_fixture("shell", SessionKind::Shell);
    shell.cwd = "/a--task".into();
    app.state.sessions.push(shell);
    app.insert("a", Tab::Terminal("shell".into()), None);
    app.active_session = Some("shell".into());
    (app, ctx, dir, sent, requests)
}

/// One frame of the Explorer sidebar, then the refresh request that the
/// frame sends. Returns the painted strings.
fn explorer_frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<String> {
    app.visible_dirs.clear();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(320.0, 400.0),
            )),
            events,
            ..Default::default()
        },
        |ui| app.sidebar_tool(SidebarTool::Explorer, ui),
    );
    app.sync_refresh_request();
    let text = painted_strings(&output.shapes);
    output.textures_delta.clear();
    text
}

/// The refresh worker reads the root of the current request.
fn refresh_root(app: &mut App, ctx: &egui::Context, found: bool) {
    let request = app.refresh_request.clone().expect("refresh request");
    let listing = if found {
        Ok(Vec::new())
    } else {
        Err(not_found(&request.cwd))
    };
    app.update_tx
        .send(Update::Refresh(
            request.generation,
            plain_context(&request.cwd),
            vec![(request.cwd.clone(), listing)],
            false,
        ))
        .unwrap();
    drain_updates(app, ctx);
}

fn explorer_root(app: &App) -> Option<PathBuf> {
    app.refresh_request
        .as_ref()
        .map(|request| request.cwd.clone())
}

fn sent_terminal_input(jobs: &mpsc::Receiver<Job>) -> bool {
    jobs.try_iter().any(|job| {
        matches!(job, Job::Control(request, _)
            if matches!(*request, Request::Input { .. } | Request::ShellCommand { .. }))
    })
}

/// Root cause of "nothing happens": the Explorer followed the focused
/// terminal into its deleted folder, and stayed there. It now shows the
/// project folder with a note. The root change clears `directory_errors`,
/// but the Explorer does not go back to the deleted folder.
#[test]
fn deleted_terminal_folder_shows_the_project_folder_with_a_note() {
    let (mut app, ctx, _dir, jobs, requests) = deleted_terminal_fixture();
    explorer_frame(&mut app, &ctx, Vec::new());
    assert_eq!(explorer_root(&app), Some("/a--task".into()));
    refresh_root(&mut app, &ctx, false);
    assert_eq!(app.cwd(), Some(PathBuf::from("/a")));
    let text = explorer_frame(&mut app, &ctx, Vec::new());
    assert!(text.iter().any(|line| line == TERMINAL_NOTE), "{text:?}");
    assert!(
        !text.iter().any(|line| line.contains("Folder not found")),
        "{text:?}"
    );
    assert_eq!(
        requests
            .borrow()
            .as_ref()
            .map(|request| request.cwd.clone()),
        Some("/a".into())
    );
    assert!(app.directory_errors.is_empty());
    refresh_root(&mut app, &ctx, true);
    for _ in 0..3 {
        explorer_frame(&mut app, &ctx, Vec::new());
        assert_eq!(app.cwd(), Some(PathBuf::from("/a")));
        assert_eq!(explorer_root(&app), Some("/a".into()));
    }
    assert!(!sent_terminal_input(&jobs));
}

/// The session service reports a change to the focused shell.
fn update_shell(app: &mut App, change: impl FnOnce(&mut Session)) {
    let mut state = app.state.clone();
    if let Some(shell) = state.sessions.iter_mut().find(|s| s.id == "shell") {
        change(shell);
    }
    state.revision = state.revision.saturating_add(1);
    app.apply_state(state);
}

#[test]
fn the_explorer_follows_the_terminal_again_after_it_changes_folder() {
    let (mut app, ctx, _dir, _jobs, _requests) = deleted_terminal_fixture();
    explorer_frame(&mut app, &ctx, Vec::new());
    refresh_root(&mut app, &ctx, false);
    assert_eq!(app.cwd(), Some(PathBuf::from("/a")));
    update_shell(&mut app, |shell| shell.cwd = "/a/src".into());
    assert_eq!(app.cwd(), Some(PathBuf::from("/a/src")));
    // The marker went with the old folder: it is not kept for later.
    update_shell(&mut app, |shell| shell.cwd = "/a--task".into());
    assert_eq!(app.cwd(), Some(PathBuf::from("/a--task")));
}

#[test]
fn an_ended_terminal_forgets_its_deleted_folder() {
    let (mut app, ctx, _dir, _jobs, _requests) = deleted_terminal_fixture();
    explorer_frame(&mut app, &ctx, Vec::new());
    refresh_root(&mut app, &ctx, false);
    assert!(app.missing_folder.terminals.contains_key("shell"));
    update_shell(&mut app, |shell| shell.lifecycle = Lifecycle::Ended);
    assert!(app.missing_folder.terminals.is_empty());
    assert_eq!(app.cwd(), Some(PathBuf::from("/a")));
}

/// The project folder itself is gone: no terminal fallback, only the
/// Folder-not-found flow.
#[test]
fn terminal_in_the_missing_project_folder_gets_no_note() {
    let (mut app, ctx, _dir, _jobs, _requests) = deleted_terminal_fixture();
    update_shell(&mut app, |shell| shell.cwd = "/a".into());
    explorer_frame(&mut app, &ctx, Vec::new());
    refresh_root(&mut app, &ctx, false);
    assert!(app.missing_folder.terminals.is_empty());
    let text = explorer_frame(&mut app, &ctx, Vec::new());
    assert!(
        text.iter()
            .any(|line| line == "Folder not found: /a. It was moved or deleted."),
        "{text:?}"
    );
    assert!(
        !text.iter().any(|line| line.starts_with("Terminal folder")),
        "{text:?}"
    );
}

/// The project folder is gone too: the usual Folder-not-found actions,
/// below the terminal note. Retry reads the terminal folder again.
#[test]
fn missing_project_folder_too_keeps_the_note_and_the_project_actions() {
    let (mut app, ctx, _dir, _jobs, _requests) = deleted_terminal_fixture();
    explorer_frame(&mut app, &ctx, Vec::new());
    refresh_root(&mut app, &ctx, false);
    assert_eq!(app.cwd(), Some(PathBuf::from("/a")));
    explorer_frame(&mut app, &ctx, Vec::new());
    refresh_root(&mut app, &ctx, false);
    let text = explorer_frame(&mut app, &ctx, Vec::new());
    for expected in [
        TERMINAL_NOTE,
        "Folder not found: /a. It was moved or deleted.",
        "Locate Folder…",
        "Remove Project",
        "Retry",
    ] {
        assert!(
            text.iter().any(|line| line == expected),
            "{expected}: {text:?}"
        );
    }
    assert_eq!(app.cwd(), Some(PathBuf::from("/a")));
    #[cfg(feature = "test-support")]
    {
        click_explorer_target(&mut app, &ctx, "directory-retry");
        assert_eq!(app.cwd(), Some(PathBuf::from("/a--task")));
        assert_eq!(explorer_root(&app), Some("/a--task".into()));
    }
}

/// Without a probe of the terminal folder, Refresh is the way to show it
/// again once it is back.
#[test]
#[cfg(feature = "test-support")]
fn refresh_reads_the_deleted_terminal_folder_again() {
    let (mut app, ctx, _dir, jobs, _requests) = deleted_terminal_fixture();
    explorer_frame(&mut app, &ctx, Vec::new());
    refresh_root(&mut app, &ctx, false);
    assert_eq!(app.cwd(), Some(PathBuf::from("/a")));
    click_explorer_target(&mut app, &ctx, "explorer-refresh");
    assert_eq!(app.cwd(), Some(PathBuf::from("/a--task")));
    assert_eq!(explorer_root(&app), Some("/a--task".into()));
    // Still gone: back to the project folder, once.
    refresh_root(&mut app, &ctx, false);
    explorer_frame(&mut app, &ctx, Vec::new());
    assert_eq!(explorer_root(&app), Some("/a".into()));
    assert!(!sent_terminal_input(&jobs));
}

#[cfg(feature = "test-support")]
fn click_explorer_target(app: &mut App, ctx: &egui::Context, name: &str) {
    explorer_frame(app, ctx, Vec::new());
    let pos = header_target(ctx)(name)
        .unwrap_or_else(|| panic!("missing {name}"))
        .center();
    explorer_frame(app, ctx, vec![egui::Event::PointerMoved(pos)]);
    for pressed in [true, false] {
        explorer_frame(
            app,
            ctx,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            }],
        );
    }
}

/// With the Explorer on the project folder, a pick of that folder opens
/// the project that is already selected. The status bar says so.
#[test]
fn picking_the_open_projects_folder_says_it_is_already_open() {
    let (mut app, ctx, _dir, jobs, _requests) = deleted_terminal_fixture();
    explorer_frame(&mut app, &ctx, Vec::new());
    refresh_root(&mut app, &ctx, false);
    assert_eq!(app.cwd(), Some(PathBuf::from("/a")));
    let (generation, _) = app.begin_project_pick(None);
    app.update_tx
        .send(Update::PickedProject(Some("/a".into()), generation))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(opened_job(&jobs), Some((PathBuf::from("/a"), generation)));
    app.update_tx
        .send(Update::OpenedProject(
            Box::new(app.state.clone()),
            "a".into(),
            generation,
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(app.selected.as_deref(), Some("a"));
    assert_eq!(app.cwd(), Some(PathBuf::from("/a")));
    assert_eq!(app.info.as_deref(), Some("a is already open."));
}

#[test]
fn locate_folder_opens_and_selects_the_pick_then_offers_to_remove_the_missing_project() {
    let (mut app, ctx, dir, requests) = missing_root_fixture();
    assert_eq!(app.selected.as_deref(), Some("a"));
    app.locate_missing_folder("a");
    assert!(app.picker.requested(&PickKind::Locate("a".into())));
    let (generation, title) = app.begin_project_pick(Some("a".into()));
    assert_eq!(title, "Locate folder");
    let moved = dir.path().join("moved");
    fs::create_dir(&moved).unwrap();
    app.update_tx
        .send(Update::PickedProject(Some(moved.clone()), generation))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(opened_job(&requests), Some((moved.clone(), generation)));
    app.update_tx
        .send(Update::OpenedProject(
            with_project(&app, "c", &moved),
            "c".into(),
            generation,
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(app.selected.as_deref(), Some("c"));
    assert_eq!(app.cwd(), Some(moved));
    assert_eq!(app.missing_folder.offer.as_deref(), Some("a"));
    assert!(app.missing_folder.locate.is_none());
    // A later ordinary pick neither locates nor offers anything.
    let (_, title) = app.begin_project_pick(None);
    assert_eq!(title, "Open project");
}

#[test]
#[cfg(feature = "test-support")]
fn offered_removal_hides_the_missing_project_and_keeps_its_terminal() {
    let (mut app, ctx, dir, requests) = missing_root_fixture();
    let moved = dir.path().join("moved");
    app.state.projects.push(Project {
        id: "c".into(),
        name: "c".into(),
        path: moved,
        layout: serde_json::Value::Null,
    });
    app.select_project("c".into());
    app.missing_folder.offer = Some("a".into());
    let paint = |app: &mut App, events: Vec<egui::Event>| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ui| app.missing_offer_dialog(ui.ctx()),
        );
        output.textures_delta.clear();
    };
    paint(&mut app, Vec::new());
    paint(&mut app, Vec::new());
    let pos = header_target(&ctx)("missing-offer-remove")
        .expect("Remove Project button")
        .center();
    paint(&mut app, vec![egui::Event::PointerMoved(pos)]);
    for pressed in [true, false] {
        paint(
            &mut app,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            }],
        );
    }
    assert!(app.preferences.hidden_projects.contains("a"));
    assert!(app.missing_folder.offer.is_none());
    assert_eq!(app.selected.as_deref(), Some("c"));
    assert!(
        app.state
            .sessions
            .iter()
            .any(|s| s.id == "shell" && s.lifecycle.live())
    );
    assert!(!requests.try_iter().any(|job| matches!(
        job,
        Job::Control(request, _) if matches!(*request, Request::Stop { .. } | Request::Remove { .. })
    )));
}

#[test]
fn cancelled_or_stale_locate_picks_offer_nothing() {
    let (mut app, ctx, dir, requests) = missing_root_fixture();
    app.locate_missing_folder("a");
    let (generation, _) = app.begin_project_pick(Some("a".into()));
    app.update_tx
        .send(Update::PickedProject(None, generation))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(app.missing_folder.locate.is_none());
    assert!(app.info.is_none());
    // The selection changes while the picker is open: say why nothing opened.
    app.locate_missing_folder("a");
    let (generation, _) = app.begin_project_pick(Some("a".into()));
    app.select_project("b".into());
    app.update_tx
        .send(Update::PickedProject(Some(dir.path().into()), generation))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(opened_job(&requests), None);
    assert!(app.missing_folder.locate.is_none());
    assert!(
        app.info
            .as_deref()
            .is_some_and(|info| info.contains("was not opened")),
        "{:?}",
        app.info
    );
}

#[test]
fn missing_root_says_folder_not_found_without_the_access_hint() {
    let (mut app, ctx, _dir, _requests) = missing_root_fixture();
    let text = explorer_text(&mut app, &ctx, Path::new("/a"));
    for expected in [
        "Folder not found: /a. It was moved or deleted.",
        "Locate Folder…",
        "Remove Project",
        "Retry",
    ] {
        assert!(
            text.iter().any(|line| line == expected),
            "{expected}: {text:?}"
        );
    }
    for hidden in [
        "NotFound",
        "Privacy",
        "Cannot refresh",
        "Choose folder again",
    ] {
        assert!(
            !text.iter().any(|line| line.contains(hidden)),
            "{hidden}: {text:?}"
        );
    }
}

/// A deleted terminal folder no longer stays the Explorer root, so a
/// missing root outside the project only offers Retry.
#[test]
fn missing_root_outside_the_project_offers_only_retry() {
    let (mut app, ctx, _dir) = fixture();
    let gone = PathBuf::from("/a--task");
    app.directory_errors.insert(gone.clone(), not_found(&gone));
    let text = explorer_text(&mut app, &ctx, &gone);
    assert!(
        text.iter()
            .any(|line| line == "Folder not found: /a--task. It was moved or deleted."),
        "{text:?}"
    );
    assert!(text.iter().any(|line| line == "Retry"), "{text:?}");
    for hidden in [
        "Open Folder…",
        "Locate Folder…",
        "Remove Project",
        "Privacy",
    ] {
        assert!(
            !text.iter().any(|line| line.contains(hidden)),
            "{hidden}: {text:?}"
        );
    }
}

#[test]
fn unreadable_folder_keeps_choose_again_and_the_access_hint() {
    let (mut app, ctx, _dir) = fixture();
    let root = Path::new("/a");
    app.directory_errors.insert(
        root.into(),
        folder_error(
            root,
            std::io::ErrorKind::PermissionDenied,
            "Permission denied (os error 13)",
        ),
    );
    let text = explorer_text(&mut app, &ctx, root);
    assert!(
        text.iter()
            .any(|line| line == "Cannot refresh /a: Permission denied (os error 13)"),
        "{text:?}"
    );
    assert!(
        text.iter().any(|line| line == "Choose folder again"),
        "{text:?}"
    );
    assert!(
        !text.iter().any(|line| line.contains("PermissionDenied")),
        "{text:?}"
    );
    assert!(
        !text.iter().any(|line| line.contains("Folder not found")),
        "{text:?}"
    );
    assert_eq!(
        text.iter().any(|line| line.contains("Privacy & Security")),
        cfg!(target_os = "macos"),
        "{text:?}"
    );
}

#[test]
#[cfg(feature = "test-support")]
fn locate_folder_button_starts_a_locate_pick() {
    let (mut app, ctx, _dir, _requests) = missing_root_fixture();
    click_tree_target(&mut app, &ctx, Path::new("/a"), "directory-locate");
    assert!(app.picker.requested(&PickKind::Locate("a".into())));
    // Nothing is armed until the Locate picker itself opens.
    assert!(app.missing_folder.locate.is_none());
}

/// Add Project after Locate Folder… replaces the request; its pick opens a
/// project and is never taken as the missing project's new folder.
#[test]
fn an_add_project_pick_after_locate_is_not_taken_as_the_locate() {
    let (mut app, ctx, dir, requests) = missing_root_fixture();
    app.locate_missing_folder("a");
    assert!(app.request_pick(PickKind::Project));
    assert_eq!(app.picker.begin(), Some(PickKind::Project));
    let (generation, title) = app.begin_project_pick(None);
    assert_eq!(title, "Open project");
    assert!(app.missing_folder.locate.is_none());
    let other = dir.path().join("other");
    fs::create_dir(&other).unwrap();
    app.update_tx
        .send(Update::PickedProject(Some(other.clone()), generation))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(opened_job(&requests), Some((other.clone(), generation)));
    app.update_tx
        .send(Update::OpenedProject(
            with_project(&app, "c", &other),
            "c".into(),
            generation,
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(app.selected.as_deref(), Some("c"));
    assert!(app.missing_folder.offer.is_none());
}

/// Locate Folder… while another picker is open is refused with a note, and
/// the open picker's pick does not locate the missing project.
#[test]
fn locate_while_a_picker_is_open_is_refused_and_arms_nothing() {
    let (mut app, _ctx, _dir, _requests) = missing_root_fixture();
    assert!(app.request_pick(PickKind::Project));
    app.picker.begin();
    let (generation, _) = app.begin_project_pick(None);
    app.locate_missing_folder("a");
    assert_eq!(app.picker, Picker::Open(PickKind::Project));
    assert!(app.missing_folder.locate.is_none());
    assert!(
        app.info
            .as_deref()
            .is_some_and(|info| info.contains("Open project"))
    );
    app.finish_folder_pick("a", generation, false);
    assert!(app.missing_folder.offer.is_none());
}

#[test]
#[cfg(feature = "test-support")]
fn remove_project_on_a_missing_root_hides_it_and_keeps_its_terminal() {
    let (mut app, ctx, _dir, requests) = missing_root_fixture();
    click_tree_target(&mut app, &ctx, Path::new("/a"), "directory-remove-project");
    assert!(app.preferences.hidden_projects.contains("a"));
    assert_eq!(app.selected.as_deref(), Some("b"));
    assert!(
        app.state
            .sessions
            .iter()
            .any(|s| s.id == "shell" && s.lifecycle.live())
    );
    assert!(!requests.try_iter().any(|job| matches!(
        job,
        Job::Control(request, _) if matches!(*request, Request::Stop { .. } | Request::Remove { .. })
    )));
}

#[test]
fn missing_project_marker_follows_the_directory_probe() {
    let (mut app, ctx, _dir, _requests) = missing_root_fixture();
    assert!(app.project_folder_missing("a"));
    assert!(!app.project_folder_missing("b"));
    assert!(app.visible_projects().iter().any(|p| p.id == "a"));
    let project = app.state.projects[0].clone();
    assert!(
        app.project_hover(&project)
            .starts_with("Folder not found\n")
    );
    paint_projects(&mut app, &ctx, Vec::new());
    #[cfg(feature = "test-support")]
    {
        assert!(crate::diagnostics::current_rect(&ctx, "project-missing:a").is_some());
        assert!(crate::diagnostics::current_rect(&ctx, "project-missing:b").is_none());
    }
    app.apply_project_directories(vec![("a".into(), "/a".into(), true)]);
    paint_projects(&mut app, &ctx, Vec::new());
    paint_projects(&mut app, &ctx, Vec::new());
    #[cfg(feature = "test-support")]
    assert!(crate::diagnostics::current_rect(&ctx, "project-missing:a").is_none());
    assert!(!app.project_folder_missing("a"));
    assert!(!app.project_hover(&project).contains("Folder not found"));
}

/// A managed worktree deleted outside the app, with no live terminal,
/// cannot be selected. Its card is marked and a click says why.
#[test]
#[cfg(feature = "test-support")]
fn deleted_worktree_card_is_marked_and_a_click_says_the_folder_is_gone() {
    let (mut app, ctx, _dir) = fixture();
    let mut state = *with_project(&app, "w", Path::new("/a--task"));
    state.worktrees.push(worktrees::Registration {
        project_id: "w".into(),
        common_dir: "/a/.git".into(),
        path: "/a--task".into(),
        created: 0,
        removed: false,
    });
    app.apply_state(state);
    app.apply_project_directories(vec![("w".into(), "/a--task".into(), false)]);
    assert!(app.project_folder_missing("w"));
    paint_projects(&mut app, &ctx, Vec::new());
    let pos = header_target(&ctx)("worktree-row:w")
        .expect("worktree card")
        .center();
    paint_projects(&mut app, &ctx, vec![egui::Event::PointerMoved(pos)]);
    for pressed in [true, false] {
        paint_projects(
            &mut app,
            &ctx,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            }],
        );
    }
    assert_eq!(app.selected.as_deref(), Some("a"));
    assert_eq!(
        app.info.as_deref(),
        Some("Folder not found: /a--task. It was moved or deleted.")
    );
}

/// A worktree row names its branch leaf and shows `+added −deleted` from
/// the read-only Git job, which a painted row asks for at most once per
/// refresh period.
#[test]
fn a_worktree_row_shows_its_branch_leaf_and_diff_stats() {
    let (mut app, ctx, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    let mut state = *with_project(&app, "w", Path::new("/a--task"));
    state.worktrees.push(worktrees::Registration {
        project_id: "w".into(),
        common_dir: "/a/.git".into(),
        path: "/a--task".into(),
        created: 0,
        removed: false,
    });
    app.apply_state(state);
    app.apply_project_directories(vec![("w".into(), "/a--task".into(), true)]);
    let paint = |app: &mut App| {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(320.0, 400.0),
                )),
                ..Default::default()
            },
            |ui| app.projects(ui),
        );
        output.textures_delta.clear();
        painted_strings(&output.shapes).join("\n")
    };
    paint(&mut app);
    paint(&mut app);
    let asked = requests
        .try_iter()
        .filter(|job| matches!(job, Job::WorktreeStats(path, _) if path == Path::new("/a--task")))
        .count();
    assert_eq!(asked, 1, "one Git job per refresh period");
    app.worktree_stats.apply(
        "/a--task".into(),
        Some(crate::worktree_stats::WorktreeGit {
            branch: Some("user/fix-login".into()),
            added: 128,
            deleted: 12,
            ..Default::default()
        }),
    );
    let text = paint(&mut app);
    assert!(text.contains("fix-login"), "branch leaf in {text}");
    assert!(
        text.contains("+128") && text.contains("−12"),
        "stats in {text}"
    );
}

/// A managed worktree `w` under project `a`, with its folder deleted.
fn deleted_worktree(app: &mut App, live: bool) {
    let mut state = *with_project(app, "w", Path::new("/a--task"));
    state.worktrees.push(worktrees::Registration {
        project_id: "w".into(),
        common_dir: "/a/.git".into(),
        path: "/a--task".into(),
        created: 0,
        removed: false,
    });
    if live {
        let mut shell = session_fixture("worker", SessionKind::Shell);
        shell.project_id = "w".into();
        shell.cwd = "/a--task".into();
        state.sessions.push(shell);
    }
    app.apply_state(state);
    app.apply_project_directories(vec![("w".into(), "/a--task".into(), false)]);
}

#[test]
fn new_terminal_on_a_deleted_worktree_says_why_and_opens_nothing_elsewhere() {
    let (mut app, _ctx, _dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    deleted_worktree(&mut app, false);
    let worktree = app.state.projects.iter().find(|p| p.id == "w").cloned();
    app.new_worktree_terminal(&worktree.unwrap());
    assert_eq!(app.selected.as_deref(), Some("a"));
    assert_eq!(
        app.info.as_deref(),
        Some("Folder not found: /a--task. It was moved or deleted.")
    );
    assert!(!requests.try_iter().any(|job| matches!(
        job,
        Job::Control(request, _) if matches!(*request, Request::Create { .. })
    )));
}

/// Remove Project on a deleted managed worktree also removes its card
/// under the parent project. Its terminal keeps running.
#[test]
#[cfg(feature = "test-support")]
fn removing_a_deleted_worktree_hides_its_card_and_keeps_its_terminal() {
    let (mut app, ctx, _dir) = fixture();
    deleted_worktree(&mut app, true);
    app.select_project("w".into());
    assert_eq!(app.selected.as_deref(), Some("w"));
    paint_projects(&mut app, &ctx, Vec::new());
    assert!(crate::diagnostics::current_rect(&ctx, "worktree-row:w").is_some());
    app.hide_project("w");
    paint_projects(&mut app, &ctx, Vec::new());
    paint_projects(&mut app, &ctx, Vec::new());
    assert!(crate::diagnostics::current_rect(&ctx, "worktree-row:w").is_none());
    assert_eq!(app.selected.as_deref(), Some("a"));
    assert!(
        app.state
            .sessions
            .iter()
            .any(|s| s.id == "worker" && s.lifecycle.live())
    );
}
