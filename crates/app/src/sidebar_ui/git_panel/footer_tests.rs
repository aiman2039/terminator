//! The Git panel's progress and footer lines, painted.
#![cfg(feature = "test-support")]
use super::*;
use crate::workspace_ops::{BranchOp, SyncProgress};
use terminator_core::async_service::CancellationToken;

fn context(root: &Path, changes: &[(&str, &str)]) -> ContextData {
    ContextData {
        cwd: root.to_path_buf(),
        root: Some(root.to_path_buf()),
        perforce: None,
        git_dirs: vec![],
        branch: "main".into(),
        changes: changes
            .iter()
            .map(|(path, status)| terminator_git::Change {
                path: root.join(path),
                status: (*status).into(),
            })
            .collect(),
        decorations: std::collections::HashMap::default(),
        stats: std::collections::HashMap::default(),
        error: None,
    }
}

/// One paint of the Commit tab. Returns the outcome and the painted texts.
fn paint(
    ctx: &egui::Context,
    context: &ContextData,
    events: Vec<egui::Event>,
) -> (GitPanelOutcome, Vec<String>) {
    paint_with(ctx, context, None, events)
}

fn paint_with(
    ctx: &egui::Context,
    context: &ContextData,
    compare: Option<&workspace_ops::CompareData>,
    events: Vec<egui::Event>,
) -> (GitPanelOutcome, Vec<String>) {
    let theme = AppearanceConfig::default();
    let mut outcome = GitPanelOutcome::default();
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(360.0, 700.0),
        )),
        events,
        ..Default::default()
    };
    let mut output = ctx.run_ui(raw, |ui| {
        let mut draft = String::new();
        let mut input = GitPanelInput {
            context,
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
            compare,
            base_ref: None,
            focus_message: false,
            commit_run: None,
            commit_lock: CommitLock::default(),
            commit_failure: None,
            list_error: None,
        };
        outcome = git_panel(ui, &mut input);
    });
    output.textures_delta.clear();
    let texts = crate::app::painted_text(&output.shapes)
        .into_iter()
        .map(|(_, text)| text)
        .collect();
    (outcome, texts)
}

fn click(ctx: &egui::Context, context: &ContextData, name: &str) -> GitPanelOutcome {
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
    paint(ctx, context, vec![egui::Event::PointerMoved(pos)]);
    paint(ctx, context, vec![press(true)]);
    paint(ctx, context, vec![press(false)]).0
}

#[test]
fn a_running_fetch_shows_its_step_and_cancel_stops_it() {
    let ctx = egui::Context::default();
    crate::appearance::install(&ctx);
    let root = PathBuf::from("/repo");
    let context = context(&root, &[]);
    let cancel = CancellationToken::new();
    let progress = SyncProgress {
        root: root.clone(),
        op: BranchOp::Fetch,
        cancel: cancel.clone(),
    };
    super::sync_progress::publish(&ctx, Some(&progress));
    let (_, texts) = paint(&ctx, &context, vec![]);
    assert!(texts.iter().any(|text| text == "Fetching…"), "{texts:?}");
    click(&ctx, &context, "git-sync-cancel");
    assert!(cancel.is_cancelled());
    let (_, texts) = paint(&ctx, &context, vec![]);
    assert!(texts.iter().any(|text| text == "Stopping…"), "{texts:?}");
    // Another repository's sync does not show here.
    let other = self::context(Path::new("/other"), &[]);
    let (_, texts) = paint(&ctx, &other, vec![]);
    assert!(!texts.iter().any(|text| text == "Stopping…"), "{texts:?}");
    super::sync_progress::publish(&ctx, None);
    let (_, texts) = paint(&ctx, &context, vec![]);
    assert!(!texts.iter().any(|text| text == "Stopping…"), "{texts:?}");
}

fn has(texts: &[String], wanted: &str) -> bool {
    texts.iter().any(|text| text == wanted)
}

#[test]
fn the_readiness_line_counts_the_staged_set_only() {
    let root = PathBuf::from("/repo");
    let mut context = context(&root, &[("a.rs", "M "), ("b.rs", " M"), ("c.rs", "??")]);
    context.stats = [("a.rs", (2, 1)), ("b.rs", (7, 0)), ("c.rs", (9, 0))]
        .into_iter()
        .map(|(path, stat)| (root.join(path), stat))
        .collect();
    let ready = readiness::readiness(&context);
    assert_eq!(
        (ready.staged, ready.total, ready.lines),
        (1, 3, Some((2, 1)))
    );
    let ctx = egui::Context::default();
    crate::appearance::install(&ctx);
    let (_, texts) = paint(&ctx, &context, vec![]);
    for text in ["1 of 3 files staged", "+2", "−1"] {
        assert!(has(&texts, text), "{text}: {texts:?}");
    }
    // A file with staged and unstaged edits has summed counts: files only.
    context.changes[0].status = "MM".into();
    assert_eq!(readiness::readiness(&context).lines, None);
    let (_, texts) = paint(&ctx, &context, vec![]);
    assert!(has(&texts, "1 of 3 files staged") && !has(&texts, "+2"));
}

#[test]
fn nothing_staged_offers_stage_all() {
    let ctx = egui::Context::default();
    crate::appearance::install(&ctx);
    let root = PathBuf::from("/repo");
    let context = context(&root, &[("b.rs", " M"), ("c.rs", "??"), ("d.rs", "UU")]);
    let (_, texts) = paint(&ctx, &context, vec![]);
    assert!(has(&texts, "Nothing staged"), "{texts:?}");
    // Conflicts stay out of Stage all.
    assert!(has(&texts, "Stage all (2)"), "{texts:?}");
    assert!(click(&ctx, &context, "git-stage-all-link").stage_all);
}

#[test]
fn create_pr_shows_only_for_a_pushed_branch_ahead_of_the_default_without_a_pr() {
    let ctx = egui::Context::default();
    crate::appearance::install(&ctx);
    let root = PathBuf::from("/repo");
    let context = context(&root, &[]);
    let mut compare = workspace_ops::CompareData {
        upstream: Some("origin/topic".into()),
        pr_ahead: Some(2),
        ..Default::default()
    };
    let (_, texts) = paint_with(&ctx, &context, Some(&compare), vec![]);
    assert!(has(&texts, "Create PR"), "{texts:?}");
    compare.pr_ahead = Some(0);
    let (_, texts) = paint_with(&ctx, &context, Some(&compare), vec![]);
    assert!(!has(&texts, "Create PR"), "nothing to propose");
    compare.pr_ahead = Some(2);
    compare.upstream = None;
    let (_, texts) = paint_with(&ctx, &context, Some(&compare), vec![]);
    assert!(!has(&texts, "Create PR"), "not pushed");
    // A known pull request shows instead, for its repository only.
    compare.upstream = Some("origin/topic".into());
    let known = KnownPr {
        root: root.clone(),
        number: 12,
        url: "https://example.com/pr/12".into(),
    };
    readiness::publish(&ctx, Some(known));
    let (_, texts) = paint_with(&ctx, &context, Some(&compare), vec![]);
    assert!(
        has(&texts, "PR #12") && !has(&texts, "Create PR"),
        "{texts:?}"
    );
    let other = self::context(Path::new("/other"), &[]);
    let (_, texts) = paint_with(&ctx, &other, Some(&compare), vec![]);
    assert!(!has(&texts, "PR #12"), "{texts:?}");
}
