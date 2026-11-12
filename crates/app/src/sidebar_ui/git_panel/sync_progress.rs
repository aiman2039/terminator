//! The Fetch, Pull or Push in flight in the shown repository: a spinner,
//! its step, and Cancel (Ctrl-C to Git, then a kill after 5 seconds).
use super::*;
use crate::workspace_ops::SyncProgress;

fn progress_id() -> egui::Id {
    egui::Id::unique("git-sync-progress")
}

/// Shares the sync in flight in the shown repository with the panel. The
/// app calls it each frame.
pub(crate) fn publish(ctx: &egui::Context, progress: Option<&SyncProgress>) {
    ctx.data_mut(|data| {
        if let Some(progress) = progress {
            data.insert_temp(progress_id(), progress.clone());
        } else {
            data.remove::<SyncProgress>(progress_id());
        }
    });
}

/// The published sync of `root`, if any.
pub(super) fn running(ctx: &egui::Context, root: Option<&Path>) -> Option<SyncProgress> {
    ctx.data(|data| data.get_temp::<SyncProgress>(progress_id()))
        .filter(|progress| Some(progress.root.as_path()) == root)
}

/// The lock of the sync menu items: a running commit, else a running sync.
pub(super) fn sync_lock(ui: &egui::Ui, input: &GitPanelInput<'_>) -> Option<&'static str> {
    input.commit_lock.head.or_else(|| {
        running(ui.ctx(), input.context.root.as_deref()).map(|_| crate::workspace_ops::SYNC_BUSY)
    })
}

/// The progress line with Cancel; nothing when no sync runs here.
pub(super) fn sync_progress(ui: &mut egui::Ui, root: Option<&Path>) {
    let Some(run) = running(ui.ctx(), root) else {
        return;
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.spinner();
        ui.label(run.line());
        let stopping = run.cancel.is_cancelled();
        let cancel = ui
            .add_enabled(!stopping, egui::Button::new("Cancel"))
            .on_hover_text("Stop Git: Ctrl-C, then kill after 5 seconds");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "git-sync-cancel", cancel.rect);
        if cancel.clicked() {
            run.cancel.cancel();
        }
    });
}
