//! Filesystem jobs: target resolution, project opening, workspace ops, search.
use super::super::submit::Services;
use crate::{Update, search, services, workspace_ops};
use anyhow::{Context, Result};
use std::path::PathBuf;
use terminator_core::async_service::CancellationToken;
use terminator_core::{Request, Response};

impl Services {
    pub(in crate::gui_services) async fn resolve_target(
        &self,
        key: String,
        text: String,
        cwd: PathBuf,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let target = self
            .fs()
            .run(&cancel, move || {
                Ok(services::resolve_target(&text, &cwd).ok())
            })
            .await?;
        updates.push(Update::ResolvedTarget(key, target));
        Ok(())
    }

    pub(in crate::gui_services) async fn open_project(
        &self,
        path: PathBuf,
        generation: u64,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
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
                Ok((state, project))
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
                    Ok((state, project))
                })
                .await?
        };
        updates.push(Update::OpenedProject(state, project, generation));
        Ok(())
    }

    pub(in crate::gui_services) async fn workspace_op(
        &self,
        root: PathBuf,
        op: workspace_ops::Op,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let reply = op.clone();
        let result = match op {
            workspace_ops::Op::Perforce(op) => {
                crate::perforce::perform(self, &root, op, &cancel).await
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

    pub(in crate::gui_services) async fn search(
        &self,
        id: u64,
        root: PathBuf,
        query: search::Query,
        show_ignored: bool,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let result = self
            .fs()
            .run(&cancel, move || {
                Ok(search::run(&root, &query, show_ignored))
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
