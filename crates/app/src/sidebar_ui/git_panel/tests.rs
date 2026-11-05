use super::*;

fn change(path: &str) -> terminator_git::Change {
    terminator_git::Change {
        path: PathBuf::from(path),
        status: " M".into(),
    }
}

#[test]
fn check_state_covers_all_none_and_some() {
    assert_eq!(
        check_state(CheckStateInput {
            checked: 0,
            total: 0
        }),
        CheckState::Unchecked
    );
    assert_eq!(
        check_state(CheckStateInput {
            checked: 0,
            total: 3
        }),
        CheckState::Unchecked
    );
    assert_eq!(
        check_state(CheckStateInput {
            checked: 3,
            total: 3
        }),
        CheckState::Checked
    );
    assert_eq!(
        check_state(CheckStateInput {
            checked: 1,
            total: 3
        }),
        CheckState::Partial
    );
}

#[test]
fn change_tree_compresses_single_child_chains() {
    let root = Path::new("/repo");
    let deep = change("/repo/apps/backend/.sqlx/query.json");
    let top = change("/repo/apps/top.rs");
    let tree = build_change_tree(&[&deep, &top], Some(root));
    // `apps` holds a file, so it stays; the chain under it compresses.
    let apps = tree.dirs.get("apps").expect("apps node");
    assert!(apps.dirs.contains_key("backend/.sqlx"));
    assert_eq!(apps.count_files(), 2);
}

#[test]
fn change_tree_keeps_branching_directories_separate() {
    let root = Path::new("/repo");
    let a = change("/repo/src/a.rs");
    let b = change("/repo/tests/b.rs");
    let tree = build_change_tree(&[&a, &b], Some(root));
    assert!(tree.dirs.contains_key("src"));
    assert!(tree.dirs.contains_key("tests"));
}

#[test]
fn commit_progress_names_the_step_and_another_repository() {
    let root = Path::new("/work/terminator");
    let progress = |phase, cancelling| CommitProgress {
        phase,
        cancelling,
        root,
    };
    let here = Some(root);
    assert_eq!(
        progress(CommitPhase::Committing, false).label(here),
        "Committing…"
    );
    assert_eq!(
        progress(CommitPhase::Pushing, false).label(here),
        "Pushing…"
    );
    assert_eq!(
        progress(CommitPhase::Pushing, true).label(here),
        "Cancelling…"
    );
    assert_eq!(
        progress(CommitPhase::Committing, false).label(Some(Path::new("/other"))),
        "Committing in terminator…"
    );
}

#[cfg(feature = "test-support")]
#[test]
fn running_commit_shows_its_step_cancel_and_the_last_failure() {
    let ctx = egui::Context::default();
    crate::appearance::install(&ctx);
    let root = PathBuf::from("/repo");
    let context = ContextData {
        cwd: root.clone(),
        root: Some(root.clone()),
        perforce: None,
        git_dirs: vec![],
        branch: "main".into(),
        changes: vec![terminator_git::Change {
            path: root.join("a.rs"),
            status: "M ".into(),
        }],
        decorations: std::collections::HashMap::default(),
        stats: std::collections::HashMap::default(),
        error: None,
    };
    let theme = AppearanceConfig::default();
    let failure = "Commit failed (exit status: 1)\nhook said no";
    let draw = |events: Vec<egui::Event>, cancelling: bool| {
        let mut outcome = GitPanelOutcome::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(360.0, 700.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                let mut draft = "msg".to_string();
                let mut input = GitPanelInput {
                    context: &context,
                    prepared: None,
                    review_mode: ReviewMode::Native,
                    neovim_review: false,
                    theme: &theme,
                    history: false,
                    view_list: false,
                    commits: &[],
                    shelves: &[],
                    branches: &[],
                    commit_draft: &mut draft,
                    recent_messages: &[],
                    amend: &mut false,
                    signoff: &mut false,
                    collapse_generation: 0,
                    open_shortcut: "",
                    compare: None,
                    base_ref: None,
                    focus_message: false,
                    commit_run: Some(CommitProgress {
                        phase: CommitPhase::Pushing,
                        cancelling,
                        root: &root,
                    }),
                    commit_lock: CommitLock::default(),
                    commit_failure: Some(failure),
                };
                outcome = git_panel(ui, &mut input);
            },
        );
        output.textures_delta.clear();
        let texts: Vec<String> = crate::app::painted_text(&output.shapes)
            .into_iter()
            .map(|(_, text)| text)
            .collect();
        (outcome, texts)
    };
    let click = |name: &str, cancelling: bool| {
        let rect = ctx
            .data(|data| data.get_temp::<egui::Rect>(egui::Id::unique(("fixture-target", name))))
            .unwrap_or_else(|| panic!("{name} is painted"));
        let pos = rect.center();
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        draw(vec![egui::Event::PointerMoved(pos)], cancelling);
        draw(vec![press(true)], cancelling);
        draw(vec![press(false)], cancelling).0
    };
    let (_, texts) = draw(vec![], false);
    for text in [
        "Pushing…",
        "Cancel",
        "Commit failed (exit status: 1)",
        "hook said no",
    ] {
        assert!(texts.iter().any(|painted| painted == text), "{text}");
    }
    // The commit buttons wait for the running job.
    assert!(!click("git-commit", false).commit);
    assert!(!click("git-commit-push", false).commit_push);
    assert!(click("git-commit-cancel", false).cancel_commit);
    assert!(click("git-commit-failure-dismiss", false).dismiss_commit_failure);
    // Once Cancel is pressed, the row says so and Cancel is disabled.
    let (_, texts) = draw(vec![], true);
    assert!(texts.iter().any(|painted| painted == "Cancelling…"));
    assert!(!click("git-commit-cancel", true).cancel_commit);
}
