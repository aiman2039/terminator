//! The Git panel's PR tab: the pull request of the current branch, read
//! only. Opt-in through Settings → Fetch PR metadata. `gh` or `glab` runs
//! off the GUI thread as a replaceable background read with the quick cap,
//! never a daemon request, and only when the tab shows or Refresh is
//! clicked. No merge or other change is offered here.
mod fetch;
#[cfg(test)]
mod tests;
mod view;

use crate::merge::{Done, take_done};
use crate::*;
pub(crate) use fetch::PrState;
use std::path::{Path, PathBuf};
use terminator_core::async_service::{CancellationToken, OperationContext, Policy};

/// The PR tab's data and its load in flight.
#[derive(Default)]
pub(crate) struct PrPanel {
    /// The repository root and branch the shown state belongs to.
    key: Option<(PathBuf, String)>,
    state: Option<PrState>,
    pending: Option<Done<PrState>>,
}

impl App {
    /// The pull request of `root` (on `branch`, when given) that the GUI
    /// already read: the PR tab's last answer or the focused terminal's
    /// metadata. Never starts a read. `(number, url)`.
    pub(crate) fn known_pr(&self, root: &Path, branch: Option<&str>) -> Option<(u64, String)> {
        let branch_matches = |known: &str| branch.is_none_or(|branch| branch == known);
        let panel = &self.git_extras.pull_request;
        if let (Some((key_root, key_branch)), Some(PrState::Ready(info))) =
            (&panel.key, &panel.state)
            && key_root == root
            && branch_matches(key_branch)
        {
            return Some((info.number, info.url.clone()));
        }
        let metadata = self.metadata.as_ref()?;
        let pr = metadata.pull_request.as_ref()?;
        (metadata.root.as_deref() == Some(root)
            && metadata.branch.as_deref().is_some_and(branch_matches))
        .then(|| (pr.number, pr.url.clone()))
    }

    /// Shares the known pull request of the shown repository with the Git
    /// panel (its Create PR link). The app calls it each frame.
    pub(crate) fn publish_known_pr(&self, ctx: &egui::Context) {
        let known = self.context.as_ref().and_then(|context| {
            let root = context.root.as_deref()?;
            let (number, url) = self.known_pr(root, Some(&context.branch))?;
            Some(sidebar_ui::KnownPr {
                root: root.to_path_buf(),
                number,
                url,
            })
        });
        sidebar_ui::publish_known_pr(ctx, known);
    }

    /// Starts a load of the pull request of `root` on `branch`. A newer
    /// load replaces the one that still runs.
    fn load_pull_request(&mut self, root: PathBuf, branch: String) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let refused = tx.clone();
        let service = self.services.clone();
        let work_root = root.clone();
        let submitted = self.services.handle().submit(
            OperationContext::new(
                "git",
                format!("pull-request:{}", root.display()),
                Policy::ReplaceableRead,
            ),
            CancellationToken::new(),
            async move {
                let token = CancellationToken::new();
                let state = service
                    .fs()
                    .run(&token, move || anyhow::Ok(fetch::fetch(&work_root, None)))
                    .await
                    .unwrap_or_else(|error| PrState::Failed(format!("{error:#}")));
                let _ = tx.send(Ok(state));
                Ok(Vec::new())
            },
        );
        if let Err(error) = submitted {
            let _ = refused.send(Err(format!("Could not start the read: {error}")));
        }
        let panel = &mut self.git_extras.pull_request;
        panel.key = Some((root, branch));
        panel.state = Some(PrState::Loading);
        panel.pending = Some(rx);
    }

    /// Takes a finished load.
    fn poll_pull_request(&mut self, ctx: &egui::Context) {
        let panel = &mut self.git_extras.pull_request;
        let Some(rx) = panel.pending.as_mut() else {
            return;
        };
        match take_done(rx) {
            Some(result) => {
                panel.pending = None;
                panel.state = Some(result.unwrap_or_else(PrState::Failed));
            }
            None => ctx.request_repaint_after(std::time::Duration::from_millis(200)),
        }
    }

    /// The PR tab body, painted under the Git panel header while the PR
    /// tab is open in a local Git repository.
    pub(crate) fn pull_request_tab(&mut self, ui: &mut egui::Ui) {
        if self.git.tab != sidebar_ui::GitTab::PullRequest {
            return;
        }
        if !self.state.settings.pr_metadata {
            ui.weak("Turn on Settings → Fetch PR metadata to see the pull request here.");
            return;
        }
        let Some(context) = self.context.as_ref() else {
            return;
        };
        let (Some(root), None) = (context.root.clone(), context.perforce.as_ref()) else {
            return;
        };
        if self.selected_remote_project().is_some() {
            ui.weak("The PR tab reads local repositories only.");
            return;
        }
        let key = (root.clone(), context.branch.clone());
        self.poll_pull_request(ui.ctx());
        let stale = self.git_extras.pull_request.key.as_ref() != Some(&key);
        let refresh = view::refresh_row(ui, self.git_extras.pull_request.pending.is_some());
        if stale || refresh {
            self.load_pull_request(key.0, key.1);
        }
        let Some(state) = self.git_extras.pull_request.state.clone() else {
            return;
        };
        if let Some(pick) = view::body(ui, &state, &self.theme) {
            self.pull_request_pick(ui, &root, pick);
        }
    }

    fn pull_request_pick(&mut self, ui: &egui::Ui, root: &std::path::Path, pick: view::Pick) {
        let Some(project) = self.selected.clone() else {
            return;
        };
        match pick {
            view::Pick::Open(url) => {
                let _ = self.jobs.send(Job::Browser(url));
            }
            view::Pick::OpenInTab(url) => {
                if let Err(error) = self.open_browser_url(&project, &url, None) {
                    self.error.set(format!("Could not open {url}: {error:#}"));
                }
            }
            view::Pick::Copy(url) => self.copy_with_note(ui.ctx(), url, "Link"),
            view::Pick::File(path, line) => self.open_file(crate::app::OpenFileInput {
                path: root.join(path),
                line,
                split: None,
                external: false,
            }),
        }
    }
}
