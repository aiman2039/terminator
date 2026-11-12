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
            compare: None,
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
