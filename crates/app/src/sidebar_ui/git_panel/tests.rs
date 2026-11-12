use super::commit::{CommitBlockInput, commit_block_reason};
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
                    tab: GitTab::Commit,
                    view_list: false,
                    commits: &[],
                    shelves: &[],
                    branches: &[],
                    commit_draft: &mut draft,
                    recent_messages: &[],
                    amend: &mut false,
                    signoff: &mut false,
                    collapse_generation: 0,
                    tree_open: true,
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
                    list_error: None,
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

#[cfg(feature = "test-support")]
#[test]
fn culled_change_rows_keep_the_full_scroll_range() {
    let root = PathBuf::from("/repo");
    let context = ContextData {
        cwd: root.clone(),
        root: Some(root.clone()),
        perforce: None,
        git_dirs: vec![],
        branch: "main".into(),
        changes: (0..3000)
            .map(|i| terminator_git::Change {
                path: root.join(format!("file{i:04}.rs")),
                status: " M".into(),
            })
            .collect(),
        decorations: std::collections::HashMap::default(),
        stats: std::collections::HashMap::default(),
        error: None,
    };
    let theme = AppearanceConfig::default();
    for view_list in [false, true] {
        let ctx = egui::Context::default();
        crate::appearance::install(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(360.0, 700.0));
        let draw = |events: Vec<egui::Event>| {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(screen),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let mut draft = String::new();
                    let mut input = GitPanelInput {
                        context: &context,
                        prepared: None,
                        review_mode: ReviewMode::Native,
                        neovim_review: false,
                        theme: &theme,
                        tab: GitTab::Commit,
                        view_list,
                        commits: &[],
                        shelves: &[],
                        branches: &[],
                        commit_draft: &mut draft,
                        recent_messages: &[],
                        amend: &mut false,
                        signoff: &mut false,
                        collapse_generation: 0,
                        tree_open: true,
                        open_shortcut: "",
                        compare: None,
                        base_ref: None,
                        focus_message: false,
                        commit_run: None,
                        commit_lock: CommitLock::default(),
                        commit_failure: None,
                        list_error: None,
                    };
                    git_panel(ui, &mut input);
                },
            );
            output.textures_delta.clear();
            crate::app::painted_text(&output.shapes)
        };
        // Over the changes; the commit area is at the bottom.
        let pointer = egui::pos2(180.0, 200.0);
        draw(vec![egui::Event::PointerMoved(pointer)]);
        // Scroll to the very end: the last row must be there, on screen.
        for _ in 0..4 {
            draw(vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                phase: egui::TouchPhase::Move,
                delta: egui::vec2(0.0, -1.0e6),
                source: egui::MouseWheelSource::Unknown,
                modifiers: egui::Modifiers::default(),
            }]);
        }
        let texts = draw(vec![]);
        let last = texts
            .iter()
            .find(|(_, text)| text == "file2999.rs")
            .unwrap_or_else(|| panic!("last row painted (list: {view_list})"));
        assert!(screen.contains(last.0), "{:?} on screen", last.0);
        assert!(
            !texts.iter().any(|(_, text)| text == "file0000.rs"),
            "the first row scrolled away"
        );
    }
}

#[test]
fn flat_rows_show_the_name_then_its_folder() {
    assert_eq!(
        split_label("crates/app/src/palette/center.rs"),
        ("crates/app/src/palette", "center.rs")
    );
    assert_eq!(split_label("README.md"), ("", "README.md"));
    assert_eq!(files_hint(1), "1 file");
    assert_eq!(files_hint(2), "2 files");
}

#[test]
fn change_tree_names_a_single_child_chain_once_like_jetbrains() {
    let root = Path::new("/repo");
    let tests = change("/repo/crates/app/src/app/tests/center_overlays.rs");
    let palette = change("/repo/crates/app/src/palette/center.rs");
    let tree = build_change_tree(&[&tests, &palette], Some(root));
    let src = tree
        .dirs
        .get("crates/app/src")
        .expect("crates/app/src node");
    assert_eq!(src.count_files(), 2);
    assert!(src.dirs.contains_key("app/tests"));
    assert!(src.dirs.contains_key("palette"));
}

/// Painted texts of one Commit tab frame, 360 × 700, with one staged file.
fn commit_tab_texts(ctx: &egui::Context) -> Vec<(egui::Pos2, String)> {
    let root = PathBuf::from("/repo");
    let context = ContextData {
        cwd: root.clone(),
        root: Some(root.clone()),
        perforce: None,
        git_dirs: vec![],
        branch: "main".into(),
        changes: vec![terminator_git::Change {
            path: root.join("src/a.rs"),
            status: "M ".into(),
        }],
        decorations: std::collections::HashMap::default(),
        stats: std::collections::HashMap::default(),
        error: None,
    };
    let theme = AppearanceConfig::default();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(360.0, 700.0),
            )),
            ..Default::default()
        },
        |ui| {
            let mut draft = String::new();
            let mut input = GitPanelInput {
                context: &context,
                prepared: None,
                review_mode: ReviewMode::Native,
                neovim_review: false,
                theme: &theme,
                tab: GitTab::Commit,
                view_list: false,
                commits: &[],
                shelves: &[],
                branches: &[],
                commit_draft: &mut draft,
                recent_messages: &[],
                amend: &mut false,
                signoff: &mut false,
                collapse_generation: 0,
                tree_open: true,
                open_shortcut: "",
                compare: None,
                base_ref: None,
                focus_message: false,
                commit_run: None,
                commit_lock: CommitLock::default(),
                commit_failure: None,
                list_error: None,
            };
            git_panel(ui, &mut input);
        },
    );
    output.textures_delta.clear();
    crate::app::painted_text(&output.shapes)
}

#[test]
fn commit_message_and_buttons_sit_below_the_changes_at_the_bottom() {
    let ctx = egui::Context::default();
    crate::appearance::install(&ctx);
    commit_tab_texts(&ctx);
    let texts = commit_tab_texts(&ctx);
    let at = |text: &str| {
        texts
            .iter()
            .find_map(|(at, painted)| (painted == text).then_some(*at))
            .unwrap_or_else(|| panic!("{text} is painted: {texts:?}"))
    };
    for tab in ["Commit", "Stash", "History"] {
        assert!(at(tab).y < at("Staged").y, "{tab} tab above the changes");
    }
    assert!(at("1 file").x > at("Staged").x, "dim count after the group");
    assert!(at("a.rs").y < at("Amend").y);
    assert!(at("Amend").y < at("Commit Message").y);
    assert!(at("Commit Message").y < at("Commit and Push").y);
    assert!(
        at("Commit and Push").y > 600.0,
        "the buttons are at the bottom"
    );
}

#[test]
fn branch_shares_the_tab_row_only_when_a_short_name_fits() {
    let budget = |free, branch| BranchBudget {
        free,
        trailing: 30.0,
        branch,
    };
    assert!(branch_beside_tabs(budget(80.0, 40.0)), "full name fits");
    assert!(
        branch_beside_tabs(budget(94.0, 300.0)),
        "a long name truncates beside the tabs"
    );
    assert!(!branch_beside_tabs(budget(93.0, 300.0)), "too narrow: wrap");
    assert!(!branch_beside_tabs(budget(60.0, 40.0)), "short name: wrap");
}

/// The History tab and branch switcher rects of one Git header frame.
#[cfg(feature = "test-support")]
fn header_rects(width: f32, branch: &str) -> (egui::Rect, egui::Rect) {
    let context = ContextData {
        cwd: "/repo".into(),
        root: Some("/repo".into()),
        perforce: None,
        git_dirs: vec![],
        branch: branch.into(),
        changes: vec![],
        decorations: std::collections::HashMap::default(),
        stats: std::collections::HashMap::default(),
        error: None,
    };
    let compare = workspace_ops::CompareData {
        root: Some("/repo".into()),
        upstream: Some("origin/main".into()),
        ahead: 3,
        behind: 12,
        base: Some("origin/main".into()),
        files: vec![],
    };
    let theme = AppearanceConfig::default();
    let ctx = egui::Context::default();
    crate::appearance::install(&ctx);
    for _ in 0..2 {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 700.0),
                )),
                ..Default::default()
            },
            |ui| {
                let mut draft = String::new();
                let mut input = GitPanelInput {
                    context: &context,
                    prepared: None,
                    review_mode: ReviewMode::Native,
                    neovim_review: false,
                    theme: &theme,
                    tab: GitTab::Commit,
                    view_list: false,
                    commits: &[],
                    shelves: &[],
                    branches: &[],
                    commit_draft: &mut draft,
                    recent_messages: &[],
                    amend: &mut false,
                    signoff: &mut false,
                    collapse_generation: 0,
                    tree_open: true,
                    open_shortcut: "",
                    compare: Some(&compare),
                    base_ref: None,
                    focus_message: false,
                    commit_run: None,
                    commit_lock: CommitLock::default(),
                    commit_failure: None,
                    list_error: None,
                };
                git_panel(ui, &mut input);
            },
        );
        output.textures_delta.clear();
    }
    let target = |name: &str| -> egui::Rect {
        ctx.data(|data| data.get_temp(egui::Id::unique(("fixture-target", name))))
            .unwrap_or_else(|| panic!("{name} is recorded"))
    };
    (target("git-history"), target("git-branch"))
}

#[cfg(feature = "test-support")]
#[test]
fn narrow_git_header_never_paints_the_branch_over_the_tabs() {
    let long = "feature/a-very-long-branch-name-for-the-header";
    for width in [180.0, 217.0, 260.0, 320.0, 480.0] {
        for branch in ["master", long] {
            let (history, switcher) = header_rects(width, branch);
            assert!(
                !history.intersects(switcher.shrink(0.5)),
                "{width} {branch}: {history:?} {switcher:?}"
            );
            assert!(switcher.right() <= width, "{width} {branch}: {switcher:?}");
            assert!(switcher.left() >= 0.0, "{width} {branch}: {switcher:?}");
        }
    }
    // Wide enough: the branch stays on the tab row, the long name truncated.
    let (history, switcher) = header_rects(480.0, long);
    assert!((history.center().y - switcher.center().y).abs() < 1.0);
    // At the default sidebar width the branch side wraps under the tabs.
    let (history, switcher) = header_rects(217.0, long);
    assert!(
        switcher.top() >= history.bottom(),
        "{history:?} {switcher:?}"
    );
}

/// Paint the Commit tab with `draft`, one staged file unless `staged` is
/// false. Returns the outcome and the painted texts.
#[cfg(feature = "test-support")]
fn paint_commit_footer(
    ctx: &egui::Context,
    draft: &mut String,
    staged: bool,
    events: Vec<egui::Event>,
) -> (GitPanelOutcome, Vec<String>) {
    let root = PathBuf::from("/repo");
    let context = ContextData {
        cwd: root.clone(),
        root: Some(root.clone()),
        perforce: None,
        git_dirs: vec![],
        branch: "main".into(),
        changes: vec![terminator_git::Change {
            path: root.join("a.rs"),
            status: if staged { "M ".into() } else { " M".into() },
        }],
        decorations: std::collections::HashMap::default(),
        stats: std::collections::HashMap::default(),
        error: None,
    };
    let theme = AppearanceConfig::default();
    let mut outcome = GitPanelOutcome::default();
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(210.0, 700.0),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            let mut input = GitPanelInput {
                context: &context,
                prepared: None,
                review_mode: ReviewMode::Native,
                neovim_review: false,
                theme: &theme,
                tab: GitTab::Commit,
                view_list: false,
                commits: &[],
                shelves: &[],
                branches: &[],
                commit_draft: draft,
                recent_messages: &[],
                amend: &mut false,
                signoff: &mut false,
                collapse_generation: 0,
                tree_open: true,
                open_shortcut: "",
                compare: None,
                base_ref: None,
                focus_message: true,
                commit_run: None,
                commit_lock: CommitLock::default(),
                commit_failure: None,
                list_error: None,
            };
            outcome = git_panel(ui, &mut input);
        },
    );
    output.textures_delta.clear();
    let texts = crate::app::painted_text(&output.shapes)
        .into_iter()
        .map(|(_, text)| text)
        .collect();
    (outcome, texts)
}

#[cfg(feature = "test-support")]
fn command_enter() -> egui::Event {
    egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    }
}

#[cfg(feature = "test-support")]
#[test]
fn command_enter_in_the_message_commits_only_when_commit_can_run() {
    let ctx = egui::Context::default();
    crate::appearance::install(&ctx);
    let mut draft = "Fix the bar".to_string();
    paint_commit_footer(&ctx, &mut draft, true, vec![]);
    let (outcome, _) = paint_commit_footer(&ctx, &mut draft, true, vec![command_enter()]);
    assert!(outcome.commit);
    assert_eq!(draft, "Fix the bar", "Cmd+Enter adds no new line");
    let mut empty = String::new();
    let ctx = egui::Context::default();
    crate::appearance::install(&ctx);
    paint_commit_footer(&ctx, &mut empty, true, vec![]);
    let (outcome, _) = paint_commit_footer(&ctx, &mut empty, true, vec![command_enter()]);
    assert!(!outcome.commit, "no message, no commit");
}

#[cfg(feature = "test-support")]
#[test]
fn commit_footer_shows_the_subject_count_and_both_buttons_on_one_row() {
    let ctx = egui::Context::default();
    crate::appearance::install(&ctx);
    let mut draft = "Fix the bar".to_string();
    paint_commit_footer(&ctx, &mut draft, true, vec![]);
    let (_, texts) = paint_commit_footer(&ctx, &mut draft, true, vec![]);
    assert!(texts.iter().any(|text| text == "11/72"), "{texts:?}");
    let commit = diagnostics::current_rect(&ctx, "git-commit").unwrap();
    let push = diagnostics::current_rect(&ctx, "git-commit-push").unwrap();
    assert!((commit.center().y - push.center().y).abs() < 1.0);
    assert!(push.right() <= 210.0, "{push:?}");
}

#[test]
fn a_disabled_commit_names_what_is_missing() {
    assert_eq!(
        commit_block_reason(CommitBlockInput {
            running: false,
            staged: false,
            amend: false,
            draft: "msg"
        }),
        Some("Stage a file first")
    );
    assert_eq!(
        commit_block_reason(CommitBlockInput {
            running: false,
            staged: true,
            amend: false,
            draft: "  "
        }),
        Some("Write a commit message")
    );
    assert_eq!(
        commit_block_reason(CommitBlockInput {
            running: false,
            staged: true,
            amend: true,
            draft: ""
        }),
        None,
        "Amend keeps the last message"
    );
    assert_eq!(
        commit_block_reason(CommitBlockInput {
            running: true,
            staged: true,
            amend: false,
            draft: "msg"
        }),
        Some("Wait for the commit to finish")
    );
}
