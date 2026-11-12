//! Git diff, worktree, and commit-log/blame jobs.
use super::super::submit::Services;
use crate::diff::ExpandGapInput;
use crate::{Tab, Update, diff, worktree_ui};
use anyhow::{Context, Result};
use std::sync::Arc;
use terminator_core::async_service::CancellationToken;
use terminator_core::{Request, Response};

pub(crate) struct DiffActionInput<'a> {
    pub(in crate::gui_services) tab: Tab,
    pub(in crate::gui_services) request: Box<diff::actions::Request>,
    pub(in crate::gui_services) cancel: CancellationToken,
    pub(in crate::gui_services) updates: &'a mut Vec<Update>,
}

pub(crate) struct DiffContextInput<'a> {
    pub(in crate::gui_services) tab: Tab,
    pub(in crate::gui_services) view: diff::DiffView,
    pub(in crate::gui_services) cancel: CancellationToken,
    pub(in crate::gui_services) updates: &'a mut Vec<Update>,
}

pub(in crate::gui_services) struct DiffExpandInput<'a> {
    pub(in crate::gui_services) tab: Tab,
    pub(in crate::gui_services) doc: Arc<diff::DiffDocument>,
    pub(in crate::gui_services) gap: usize,
    pub(in crate::gui_services) part: diff::GapPart,
    pub(in crate::gui_services) cancel: CancellationToken,
    pub(in crate::gui_services) updates: &'a mut Vec<Update>,
}
impl Services {
    pub(in crate::gui_services) async fn diff_action(
        &self,
        input: DiffActionInput<'_>,
    ) -> Result<()> {
        let DiffActionInput {
            tab,
            request,
            cancel,
            updates,
        } = input;
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
        let result = diff::actions::execute(diff::actions::ExecuteInput {
            service: self,
            request: *request,
            staged,
            cancel: &cancel,
        })
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
            terminator_git::RunInput {
                cwd: dir,
                args: vec!["show".into(), format!("HEAD:./{name}").into()],
                options: terminator_core::CommandOptions {
                    stdout_limit: 4 * 1024 * 1024,
                    ..Default::default()
                },
            },
        )
        .await
        .ok()?;
        terminator_git::check_text(&bytes).then(|| String::from_utf8_lossy(&bytes).into_owned())
    }

    /// The canonical worktree root of `path`'s repository, found the way
    /// the gutter Stage job finds it: `rev-parse --show-toplevel` in its
    /// folder, then canonicalized.
    pub(in crate::gui_services) async fn git_toplevel(
        &self,
        path: &std::path::Path,
        cancel: &CancellationToken,
    ) -> Option<std::path::PathBuf> {
        let bytes = terminator_git::run(
            self.processes(),
            self.fs(),
            terminator_git::RunInput {
                cwd: path.parent()?,
                args: vec!["rev-parse".into(), "--show-toplevel".into()],
                options: terminator_core::CommandOptions::default(),
            },
        )
        .await
        .ok()?;
        let root = std::path::PathBuf::from(std::str::from_utf8(&bytes).ok()?.trim());
        self.fs()
            .run(cancel, move || anyhow::Ok(root.canonicalize()?))
            .await
            .ok()
    }

    pub(in crate::gui_services) async fn diff_context(
        &self,
        input: DiffContextInput<'_>,
    ) -> Result<()> {
        let DiffContextInput {
            tab,
            view,
            cancel,
            updates,
        } = input;
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
                diff::remote_document(diff::RemoteDocumentInput {
                    service: self,
                    client,
                    tab: &tab,
                    context,
                    cancel: &cancel,
                })
                .await
            } else {
                diff::document_with_context(diff::DocumentWithContextInput {
                    service: self,
                    cwd: cwd.clone(),
                    path: path.clone(),
                    staged: *staged,
                    context,
                    cancel: &cancel,
                })
                .await
            };
            // Expanded ranges and the compare options survive a refresh.
            let result = match result {
                Ok(doc) if view.needs_rebuild() => {
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
        input: DiffExpandInput<'_>,
    ) -> Result<()> {
        let DiffExpandInput {
            tab,
            doc,
            gap,
            part,
            cancel,
            updates,
        } = input;
        if let Tab::Diff { path, staged, .. } = &tab {
            let path = path.clone();
            let staged = *staged;
            let result = self
                .cpu()
                .run(&cancel, move || {
                    doc.expand_gap(ExpandGapInput {
                        path: &path,
                        gap,
                        part,
                        staged,
                    })
                })
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
                diff::remote_document(diff::RemoteDocumentInput {
                    service: self,
                    client,
                    tab: &tab,
                    context: 6,
                    cancel: &cancel,
                })
                .await
            } else {
                diff::document_async(diff::DocumentAsyncInput {
                    service: self,
                    cwd: cwd.clone(),
                    path: path.clone(),
                    staged: *staged,
                    cancel: &cancel,
                })
                .await
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
        let mut draft = draft;
        // A pull request task starts at its head, fetched now (on Create)
        // into FETCH_HEAD only: no branch or ref changes.
        if let Some((root, head)) = draft.issue.head_to_fetch(&draft.start) {
            draft.start = self
                .fs()
                .run(&cancel, move || {
                    crate::git::task_issue::fetch_head(&root, &head)
                })
                .await
                .context("Could not fetch the pull request head")?;
        }
        let mut start = draft.task_start();
        let revision = draft.start.clone();
        let source_id = draft.source.clone();
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
            .run(&cancel, move || anyhow::Ok(draft.dest.canonicalize()?))
            .await?;
        let Response::State(state) = client.rpc(Request::Snapshot).await? else {
            anyhow::bail!("Expected worktree snapshot")
        };
        let project = checkout_project(&state, &canonical)
            .context("Created worktree missing from inventory")?;
        // On the file pool: the commit the task starts at (read-only Git),
        // then the `.worktreeinclude` copy into the new checkout.
        if let Some(source) = state.projects.iter().find(|p| p.id == source_id) {
            let (source, checkout) = (source.path.clone(), canonical.clone());
            let (base, report) = self
                .fs()
                .run(&cancel, move || {
                    let base = crate::task_changes::task_base(&source, &checkout, &revision);
                    let report = crate::worktree_include::copy_included(&source, &checkout);
                    anyhow::Ok((base, report))
                })
                .await
                .unwrap_or_default();
            start.base = base.map(|base| (canonical.clone(), base));
            start.setup_note = report.summary();
        }
        updates.push(Update::WorktreeCreated(state, project, start));
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
            .run(&cancel, move || anyhow::Ok(crate::GitView::load(&task)))
            .await
            .unwrap_or_else(|error| crate::GitView::failed(&tab, format!("{error:#}")));
        updates.push(Update::GitView(key, view));
        Ok(())
    }

    /// A Commit Log tab's filtered load or Go to Hash. Git runs locally,
    /// also for logs that a daemon could serve: no daemon request is made.
    pub(in crate::gui_services) async fn git_log(
        &self,
        (tab, query): (Tab, crate::git_log::LogQuery),
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let key = tab.key();
        let Tab::CommitLog { cwd } = &tab else {
            let log = query.failed("Not a Commit Log tab".into());
            updates.push(Update::GitView(key, crate::GitView::Log(log)));
            return Ok(());
        };
        let git = JobGit::new(self, &cancel);
        let (cwd, task) = (cwd.clone(), query.clone());
        let log = self
            .fs()
            .run(&cancel, move || {
                anyhow::Ok(crate::git_log::query_log(&git, &cwd, &task))
            })
            .await
            .unwrap_or_else(|error| query.failed(format!("{error:#}")));
        updates.push(Update::GitView(key, crate::GitView::Log(log)));
        Ok(())
    }
}

/// Git for a Commit Log job. When a newer query replaces the job, its
/// cancellation kills the Git process group at once.
pub(in crate::gui_services) struct JobGit {
    processes: terminator_core::async_process::Processes,
    runtime: tokio::runtime::Handle,
    cancel: CancellationToken,
}

impl JobGit {
    /// Call on a runtime thread; use the result on a file-pool thread.
    pub(in crate::gui_services) fn new(service: &Services, cancel: &CancellationToken) -> Self {
        Self {
            processes: service.processes().clone(),
            runtime: tokio::runtime::Handle::current(),
            cancel: cancel.clone(),
        }
    }
}

impl crate::git_log::LogGit for JobGit {
    fn git(
        &self,
        dir: &std::path::Path,
        args: &[&str],
        options: terminator_core::CommandOptions,
    ) -> std::result::Result<Vec<u8>, String> {
        let command = terminator_core::git::command(dir, args);
        // A file-pool thread, not a runtime thread: it may wait here.
        self.runtime
            .block_on(
                self.processes
                    .run_cancellable(command, options, &self.cancel),
            )
            .map(|output| output.stdout)
            .map_err(|error| error.to_string())
    }
}

/// The project of the checkout at `path`. A removed worktree at the same
/// path cannot be selected, so it never matches.
fn checkout_project(state: &terminator_core::State, path: &std::path::Path) -> Option<String> {
    crate::services::openable_projects(state)
        .find(|project| project.path == path)
        .map(|project| project.id.clone())
}

#[cfg(test)]
mod tests {
    /// A removed worktree stays in the inventory with its old path. A new
    /// checkout at that path must open the new project, which can be
    /// selected, not the removed one.
    #[test]
    fn a_new_checkout_at_a_removed_worktree_path_opens_the_new_project() {
        let project = |id: &str| terminator_core::Project {
            id: id.into(),
            name: id.into(),
            path: "/work/task".into(),
            layout: serde_json::Value::Null,
        };
        let registration = |id: &str, removed| terminator_core::worktrees::Registration {
            project_id: id.into(),
            common_dir: "/work/repo/.git".into(),
            path: "/work/task".into(),
            created: 0,
            removed,
        };
        let state = terminator_core::State {
            projects: vec![project("removed"), project("checkout")],
            worktrees: vec![
                registration("removed", true),
                registration("checkout", false),
            ],
            ..Default::default()
        };
        let path = std::path::Path::new("/work/task");
        assert_eq!(
            super::checkout_project(&state, path).as_deref(),
            Some("checkout")
        );
    }

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
