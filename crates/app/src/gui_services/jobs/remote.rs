//! Remote-project jobs.
use super::super::submit::Services;
use crate::{After, Update};
use anyhow::Result;
use std::{fs, path::PathBuf};
use terminator_core::async_service::CancellationToken;
use terminator_core::{Request, Response};

impl Services {
    pub(in crate::gui_services) async fn remote_add(
        &self,
        project: terminator_core::remote::RemoteProject,
        probe: bool,
        generation: u64,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let client = self.client();
        let request = if probe {
            Request::ProbeRemoteProject {
                project: project.clone(),
            }
        } else {
            Request::AddRemoteProject {
                project: project.clone(),
            }
        };
        let result = match client.rpc(request).await {
            Ok(Response::RemoteProject(terminator_core::remote::Reply::Probe { root })) => {
                Ok(crate::remote_ui::FormResult::Probe(root))
            }
            Ok(Response::Ok) => match client.rpc(Request::Snapshot).await {
                Ok(Response::State(state)) => Ok(crate::remote_ui::FormResult::Added(state)),
                Ok(_) => Err("Expected project snapshot".into()),
                Err(e) => Err(format!("{e:#}")),
            },
            Ok(_) => Err("Unexpected SSH project response".into()),
            Err(e) => Err(format!("{e:#}")),
        };
        updates.push(Update::RemoteAdded(project, generation, result));
        Ok(())
    }

    pub(in crate::gui_services) async fn remote_read(
        &self,
        project: String,
        operation: terminator_core::remote::Operation,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let client = self.client();
        let result = match client
            .rpc(Request::RemoteProject {
                project: project.clone(),
                operation: operation.clone(),
            })
            .await
        {
            Ok(Response::RemoteProject(reply)) => Ok(reply),
            Ok(_) => Err("Unexpected SSH project response".into()),
            Err(e) => Err(format!("{e:#}")),
        };
        updates.push(Update::RemoteReply(project, operation, result));
        Ok(())
    }

    pub(in crate::gui_services) async fn remote_asset(
        &self,
        project: String,
        path: String,
        cache_root: PathBuf,
        after: After,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let client = self.client();
        terminator_core::remote::validate_relative(&path)?;
        let reply = client
            .rpc(Request::RemoteProject {
                project: project.clone(),
                operation: terminator_core::remote::Operation::Read { path: path.clone() },
            })
            .await?;
        let Response::RemoteProject(terminator_core::remote::Reply::File {
            path: actual,
            base64,
        }) = reply
        else {
            anyhow::bail!("Unexpected remote file response")
        };
        anyhow::ensure!(actual == path, "Remote file response identity changed");
        let target = cache_root.join(&path);
        let saved = target.clone();
        self.fs()
            .run(&cancel, move || {
                use base64::Engine;
                let bytes = base64::engine::general_purpose::STANDARD.decode(base64)?;
                anyhow::ensure!(
                    bytes.len() <= terminator_core::remote::MAX_FILE_BYTES,
                    "Remote preview exceeds 2 MiB"
                );
                if let Some(parent) = saved.parent() {
                    fs::create_dir_all(parent)?;
                }
                terminator_core::atomic_write(&saved, &bytes)?;
                let mut permissions = fs::metadata(&saved)?.permissions();
                permissions.set_readonly(true);
                fs::set_permissions(&saved, permissions)?;
                Ok(())
            })
            .await?;
        updates.push(Update::RemoteAsset(project, target, after));
        Ok(())
    }
}
