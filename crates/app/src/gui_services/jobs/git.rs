//! Git diff, worktree, and commit-log/blame jobs.
use super::super::submit::Services;
use crate::{Tab, Update, diff, worktree_ui};
use anyhow::{Context, Result};
use std::sync::Arc;
use terminator_core::async_service::CancellationToken;
use terminator_core::{Request, Response};

impl Services {
    pub(in crate::gui_services) async fn diff_action(
        &self,
        tab: Tab,
        request: Box<diff::actions::Request>,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        anyhow::ensure!(
            !matches!(
                &tab,
                Tab::Diff {
                    remote: Some(_),
                    ..
                }
            ),
            "Remote Git diffs are read-only; use the SSH terminal to change Git state"
        );
        let undo = matches!(request.as_ref(), diff::actions::Request::Undo(_));
        let staged = matches!(&tab, Tab::Diff { staged: true, .. });
        let result = diff::actions::execute(self, *request, staged, &cancel)
            .await
            .map_err(|error| format!("{error:#}"));
        updates.push(Update::DiffAction(tab, undo, result));
        Ok(())
    }

    /// The `HEAD` version of a file, or `None` outside Git, for untracked
    /// files, and for binary or non-UTF-8 content.
    pub(in crate::gui_services) async fn git_base_text(
        &self,
        path: &std::path::Path,
    ) -> Option<String> {
        let dir = path.parent()?;
        let name = path.file_name()?.to_string_lossy().into_owned();
        let bytes = terminator_git::run(
            self.processes(),
            self.fs(),
            dir,
            vec!["show".into(), format!("HEAD:./{name}").into()],
            terminator_core::CommandOptions {
                stdout_limit: 4 * 1024 * 1024,
                ..Default::default()
            },
        )
        .await
        .ok()?;
        terminator_git::check_text(&bytes).then(|| String::from_utf8_lossy(&bytes).into_owned())
    }

    pub(in crate::gui_services) async fn diff_context(
        &self,
        tab: Tab,
        view: diff::DiffView,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let client = self.client();
        let context = view.context;
        if let Tab::Diff {
            cwd, path, staged, ..
        } = &tab
        {
            let result = if matches!(
                &tab,
                Tab::Diff {
                    remote: Some(_),
                    ..
                }
            ) {
                diff::remote_document(self, client, &tab, context, &cancel).await
            } else {
                diff::document_with_context(
                    self,
                    cwd.clone(),
                    path.clone(),
                    *staged,
                    context,
                    &cancel,
                )
                .await
            };
            // Expanded ranges and the whitespace mode survive a refresh.
            let result = match result {
                Ok(doc) if view.whitespace || !view.revealed.is_empty() => {
                    let (path, staged) = (path.clone(), *staged);
                    self.cpu()
                        .run(&cancel, move || Ok(doc.rebuild(&path, staged, &view)))
                        .await
                }
                other => other,
            }
            .map_err(|e| format!("{e:#}"));
            updates.push(Update::Diff(tab.key(), result));
        }
        Ok(())
    }

    pub(in crate::gui_services) async fn diff_expand(
        &self,
        tab: Tab,
        doc: Arc<diff::DiffDocument>,
        (gap, part): (usize, diff::GapPart),
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        if let Tab::Diff { path, staged, .. } = &tab {
            let path = path.clone();
            let staged = *staged;
            let result = self
                .cpu()
                .run(&cancel, move || doc.expand_gap(&path, gap, part, staged))
                .await
                .map_err(|error| format!("{error:#}"));
            updates.push(Update::Diff(tab.key(), result));
        }
        Ok(())
    }

    pub(in crate::gui_services) async fn diff(
        &self,
        tab: Tab,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let client = self.client();
        if let Tab::Diff {
            cwd, path, staged, ..
        } = &tab
        {
            let result = if matches!(
                &tab,
                Tab::Diff {
                    remote: Some(_),
                    ..
                }
            ) {
                diff::remote_document(self, client, &tab, 6, &cancel).await
            } else {
                diff::document_async(self, cwd.clone(), path.clone(), *staged, &cancel).await
            }
            .map_err(|e| format!("{e:#}"));
            updates.push(Update::Diff(tab.key(), result));
        }
        Ok(())
    }

    pub(in crate::gui_services) async fn create_worktree(
        &self,
        draft: worktree_ui::WorktreeDraft,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let client = self.client();
        anyhow::ensure!(
            matches!(
                client
                    .rpc(Request::WorktreeAdd {
                        project: draft.source,
                        path: draft.dest.clone(),
                        branch: Some(draft.branch.trim().to_owned()).filter(|s| !s.is_empty()),
                        start: draft.start.trim().to_owned()
                    })
                    .await?,
                Response::Ok
            ),
            "Unexpected worktree creation response"
        );
        let canonical = self
            .fs()
            .run(&cancel, move || Ok(draft.dest.canonicalize()?))
            .await?;
        let Response::State(state) = client.rpc(Request::Snapshot).await? else {
            anyhow::bail!("Expected worktree snapshot")
        };
        let project = state
            .projects
            .iter()
            .find(|p| p.path == canonical)
            .context("Created worktree missing from inventory")?
            .id
            .clone();
        updates.push(Update::WorktreeCreated(state, project, draft.open_terminal));
        Ok(())
    }

    pub(in crate::gui_services) async fn git_view(
        &self,
        tab: Tab,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let key = tab.key();
        let task = tab.clone();
        let view = self
            .fs()
            .run(&cancel, move || Ok(crate::GitView::load(&task)))
            .await
            .unwrap_or_else(|error| crate::GitView::failed(&tab, format!("{error:#}")));
        updates.push(Update::GitView(key, view));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn git_base_reads_head_and_skips_untracked_files() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        let git = |args: &[&str]| {
            terminator_git::run_blocking(&repo, args, terminator_core::CommandOptions::default())
                .unwrap();
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "base@example.invalid"]);
        git(&["config", "user.name", "Base"]);
        git(&["config", "commit.gpgsign", "false"]);
        std::fs::write(repo.join("src/lib.rs"), "committed\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-qm", "base"]);
        std::fs::write(repo.join("src/lib.rs"), "edited\n").unwrap();
        std::fs::write(repo.join("src/new.rs"), "fresh\n").unwrap();
        let (updates, _) = std::sync::mpsc::channel();
        let (service, _owner) = crate::gui_services::Services::new(
            terminator_core::Paths::at(dir.path().join("state")),
            eframe::egui::Context::default(),
            updates,
        )
        .unwrap();
        assert_eq!(
            service
                .git_base_text(&repo.join("src/lib.rs"))
                .await
                .as_deref(),
            Some("committed\n")
        );
        assert_eq!(service.git_base_text(&repo.join("src/new.rs")).await, None);
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("x.rs"), "x").unwrap();
        assert_eq!(
            service.git_base_text(&outside.path().join("x.rs")).await,
            None
        );
    }
}
