//! Replies of the cancellable commit and push job.

use crate::*;

impl App {
    /// `Pushing` is progress: the commit landed and the panel shows it while
    /// the push runs. Any other reply ends the run. A landed commit reloads
    /// branch lists and Git views; a cancel is a notice; a failure keeps its
    /// output in the Git panel and names it in the status bar.
    pub(super) fn handle_commit_reply(&mut self, result: Result<workspace_ops::Report, String>) {
        if matches!(result, Ok(workspace_ops::Report::Pushing)) {
            if self.git.commit_pushing() {
                self.reload_after_commit();
            }
            return;
        }
        let result = result.map(|report| match report {
            workspace_ops::Report::Message(text) => text,
            _ => "Committed".into(),
        });
        if self.git.finish_commit(&result) {
            self.reload_after_commit();
        }
        self.refresh_request = None;
        match result {
            Ok(text) => self.info = Some(text),
            Err(notice) if workspace_ops::commit_cancelled(&notice) => self.info = Some(notice),
            Err(error) => {
                let failure = self.git.commit_failure.as_deref().unwrap_or(&error);
                let summary = failure.lines().next().unwrap_or_default().to_owned();
                self.report_status_error(summary);
            }
        }
    }

    /// A commit landed: status, branch lists, and Git views reload.
    fn reload_after_commit(&mut self) {
        self.refresh_request = None;
        self.git.list_root = None;
        // History and blame changed; open views reload.
        self.git_views.clear();
    }
}
