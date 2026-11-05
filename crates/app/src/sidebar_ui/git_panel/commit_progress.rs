//! The commit area while a commit job runs, and the last failure's output.
use super::*;

/// The step a commit job runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CommitPhase {
    Committing,
    Pushing,
}

/// The panel controls that a commit running in its repository disables,
/// as the hover text that says why, or `None` when they work.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CommitLock {
    /// Staging and Discard: the commit runs in this worktree.
    pub(crate) index: Option<&'static str>,
    /// Shelve, Unshelve, and Drop: the commit runs in a worktree of this
    /// repository.
    pub(crate) stash: Option<&'static str>,
    /// The branch switch: the commit or its push runs in this worktree.
    pub(crate) head: Option<&'static str>,
}

/// The commit job in flight, as the commit area shows it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CommitProgress<'a> {
    pub(crate) phase: CommitPhase,
    /// Cancel was pressed; the job is stopping Git.
    pub(crate) cancelling: bool,
    /// The repository the job runs in.
    pub(crate) root: &'a Path,
}

impl CommitProgress<'_> {
    /// `Committing…`, `Pushing…`, or `Cancelling…`. Names the repository
    /// when the panel shows another one.
    pub(crate) fn label(&self, shown: Option<&Path>) -> String {
        let verb = match (self.cancelling, self.phase) {
            (true, _) => "Cancelling",
            (false, CommitPhase::Committing) => "Committing",
            (false, CommitPhase::Pushing) => "Pushing",
        };
        match self.root.file_name().filter(|_| shown != Some(self.root)) {
            Some(name) => format!("{verb} in {}…", name.to_string_lossy()),
            None => format!("{verb}…"),
        }
    }
}

impl GitPanelPass<'_, '_> {
    /// Spinner, the step, and Cancel, which stops Git and its hooks.
    pub(super) fn commit_progress(&mut self, ui: &mut egui::Ui) {
        let Some(progress) = self.input.commit_run else {
            return;
        };
        let label = progress.label(self.input.context.root.as_deref());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.spinner();
            ui.label(label)
                .on_hover_text(progress.root.display().to_string());
            let cancel = ui
                .add_enabled(!progress.cancelling, egui::Button::new("Cancel"))
                .on_hover_text("Stop Git and its hooks");
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "git-commit-cancel", cancel.rect);
            if cancel.clicked() {
                self.outcome.cancel_commit = true;
            }
        });
    }

    /// The last failure: its summary, then the end of the hook and Git
    /// output.
    pub(super) fn commit_failure(&mut self, ui: &mut egui::Ui) {
        let Some(failure) = self.input.commit_failure else {
            return;
        };
        let (summary, output) = failure.split_once('\n').unwrap_or((failure, ""));
        let color = appearance::color(&self.input.theme.status_failed);
        ui.horizontal(|ui| {
            let dismiss = appearance::sidebar_action(ui, "X", "Dismiss");
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "git-commit-failure-dismiss", dismiss.rect);
            if dismiss.clicked() {
                self.outcome.dismiss_commit_failure = true;
            }
            ui.add(egui::Label::new(RichText::new(summary).color(color)).wrap());
        });
        if output.is_empty() {
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("git-commit-failure")
            .max_height(140.0)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(output).monospace().small()).wrap());
            });
    }
}
