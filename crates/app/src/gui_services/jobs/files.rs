//! Filesystem jobs: target resolution, project opening, workspace ops, search.
use super::super::submit::Services;
use crate::{Update, search, services, workspace_ops};
use anyhow::{Context, Result};
use std::path::PathBuf;
use terminator_core::async_service::CancellationToken;
use terminator_core::{Request, Response};

pub(crate) struct ResolveTargetInput<'a> {
    pub(in crate::gui_services) key: String,
    pub(in crate::gui_services) text: String,
    pub(in crate::gui_services) cwd: PathBuf,
    pub(in crate::gui_services) cancel: CancellationToken,
    pub(in crate::gui_services) updates: &'a mut Vec<Update>,
}

pub(crate) struct OpenProjectInput<'a> {
    pub(in crate::gui_services) path: PathBuf,
    pub(in crate::gui_services) generation: u64,
    pub(in crate::gui_services) cancel: CancellationToken,
    pub(in crate::gui_services) updates: &'a mut Vec<Update>,
}

pub(crate) struct WorkspaceOpInput<'a> {
    pub(in crate::gui_services) root: PathBuf,
    pub(in crate::gui_services) op: workspace_ops::Op,
    pub(in crate::gui_services) cancel: CancellationToken,
    pub(in crate::gui_services) updates: &'a mut Vec<Update>,
}

pub(crate) struct SearchInput<'a> {
    pub(in crate::gui_services) id: u64,
    pub(in crate::gui_services) root: PathBuf,
    pub(in crate::gui_services) query: search::Query,
    pub(in crate::gui_services) show_ignored: bool,
    pub(in crate::gui_services) cancel: CancellationToken,
    pub(in crate::gui_services) updates: &'a mut Vec<Update>,
}

impl Services {
    pub(in crate::gui_services) async fn resolve_target(
        &self,
        input: ResolveTargetInput<'_>,
    ) -> Result<()> {
        let ResolveTargetInput {
            key,
            text,
            cwd,
            cancel,
            updates,
        } = input;
        let target = self
            .fs()
            .run(&cancel, move || {
                anyhow::Ok(services::resolve_target(&text, &cwd).ok())
            })
            .await?;
        updates.push(Update::ResolvedTarget(key, target));
        Ok(())
    }

    pub(in crate::gui_services) async fn open_project(
        &self,
        input: OpenProjectInput<'_>,
    ) -> Result<()> {
        let OpenProjectInput {
            path,
            generation,
            cancel,
            updates,
        } = input;
        let client = self.client();
        let path = self
            .fs()
            .run(&cancel, move || {
                let path = path.canonicalize()?;
                anyhow::ensure!(path.is_dir(), "Project must be a directory");
                Ok(path)
            })
            .await?;
        let Response::State(state) = client.rpc(Request::Snapshot).await? else {
            anyhow::bail!("Expected project inventory")
        };
        let lookup = path.clone();
        let (state, project) = self
            .fs()
            .run(&cancel, move || {
                let project =
                    services::project_for_directory(&state.projects, &lookup).map(|p| p.id.clone());
                anyhow::Ok((state, project))
            })
            .await?;
        let (state, project) = if let Some(project) = project {
            (state, project)
        } else {
            client
                .rpc(Request::AddProject { path: path.clone() })
                .await?;
            let Response::State(state) = client.rpc(Request::Snapshot).await? else {
                anyhow::bail!("Expected project inventory")
            };
            self.fs()
                .run(&cancel, move || {
                    let project = services::project_for_directory(&state.projects, &path)
                        .context("Opened project missing from inventory")?
                        .id
                        .clone();
                    anyhow::Ok((state, project))
                })
                .await?
        };
        updates.push(Update::OpenedProject(state, project, generation));
        Ok(())
    }

    pub(in crate::gui_services) async fn workspace_op(
        &self,
        input: WorkspaceOpInput<'_>,
    ) -> Result<()> {
        let WorkspaceOpInput {
            root,
            op,
            cancel,
            updates,
        } = input;
        let reply = op.clone();
        let result = match op {
            workspace_ops::Op::Perforce(op) => {
                crate::perforce::perform(crate::perforce::PerformInput {
                    service: self,
                    cwd: &root,
                    op,
                    cancel: &cancel,
                })
                .await
            }
            op => {
                self.fs()
                    .run(&cancel, move || workspace_ops::perform(&root, op))
                    .await
            }
        }
        .map_err(|error| format!("{error:#}"));
        updates.push(Update::Workspace(reply, result));
        Ok(())
    }

    pub(in crate::gui_services) async fn search(&self, input: SearchInput<'_>) -> Result<()> {
        let SearchInput {
            id,
            root,
            query,
            show_ignored,
            cancel,
            updates,
        } = input;
        let result = self
            .fs()
            .run(&cancel, move || {
                anyhow::Ok(search::run(&root, &query, show_ignored))
            })
            .await;
        let result = match result {
            Ok(inner) => inner,
            Err(error) => Err(format!("{error:#}")),
        };
        updates.push(Update::Search(id, result));
        Ok(())
    }
}
