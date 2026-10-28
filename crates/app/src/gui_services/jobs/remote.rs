//! Remote-project jobs.
use super::super::submit::Services;
use crate::{After, Update};
use anyhow::Result;
use std::{fs, path::PathBuf};
use terminator_core::async_service::CancellationToken;
use terminator_core::{Request, Response};

pub(crate) struct RemoteAddInput<'a> {
    pub(in crate::gui_services) project: terminator_core::remote::RemoteProject,
    pub(in crate::gui_services) probe: bool,
    pub(in crate::gui_services) generation: u64,
    pub(in crate::gui_services) updates: &'a mut Vec<Update>,
}

pub(crate) struct RemoteTransferInput<'a> {
    pub(in crate::gui_services) project: String,
    pub(in crate::gui_services) pick: crate::transfers_ui::Pick,
    pub(in crate::gui_services) cancel: CancellationToken,
    pub(in crate::gui_services) updates: &'a mut Vec<Update>,
}

pub(crate) struct RemoteAssetInput<'a> {
    pub(in crate::gui_services) project: String,
    pub(in crate::gui_services) path: String,
    pub(in crate::gui_services) cache_root: PathBuf,
    pub(in crate::gui_services) after: After,
    pub(in crate::gui_services) cancel: CancellationToken,
    pub(in crate::gui_services) updates: &'a mut Vec<Update>,
}

impl Services {
    pub(in crate::gui_services) async fn remote_add(
        &self,
        input: RemoteAddInput<'_>,
    ) -> Result<()> {
        let RemoteAddInput {
            project,
            probe,
            generation,
            updates,
        } = input;
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
                Ok(_) => Err(String::from("Expected project snapshot").into()),
                Err(e) => Err(crate::remote_ui::Failure::from_error(&e)),
            },
            Ok(_) => Err(String::from("Unexpected SSH project response").into()),
            Err(e) => Err(crate::remote_ui::Failure::from_error(&e)),
        };
        updates.push(Update::RemoteAdded(project, generation, result));
        Ok(())
    }

    pub(in crate::gui_services) async fn forwards(
        &self,
        request: Request,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let key = crate::forwards_ui::forward_key(&request);
        let result = match self.client().rpc(request).await {
            Ok(Response::Forwards(status)) => Ok(status),
            Ok(_) => Err(String::from("Unexpected port forward response").into()),
            Err(e) => Err(crate::remote_ui::Failure::from_error(&e)),
        };
        updates.push(Update::Forwards(key, result));
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
            Ok(_) => Err(String::from("Unexpected SSH project response").into()),
            Err(e) => Err(crate::remote_ui::Failure::from_error(&e)),
        };
        updates.push(Update::RemoteReply(project, operation, result));
        Ok(())
    }

    pub(in crate::gui_services) async fn remote_transfer(
        &self,
        input: RemoteTransferInput<'_>,
    ) -> Result<()> {
        let RemoteTransferInput {
            project,
            pick,
            cancel,
            updates,
        } = input;
        let picked = self
            .fs()
            .run(&cancel, move || anyhow::Ok(pick.resolve()))
            .await;
        let result = match picked {
            Ok(Some(operation)) => self.remote_transfer_rpc(&project, operation).await,
            Ok(None) => Ok(None),
            Err(e) => Err(format!("{e:#}").into()),
        };
        updates.push(Update::RemoteTransfer(project, None, result));
        Ok(())
    }

    pub(in crate::gui_services) async fn remote_transfer_poll(
        &self,
        project: String,
        id: String,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let operation = terminator_core::remote::Operation::TransferStatus { id: id.clone() };
        let result = self.remote_transfer_rpc(&project, operation).await;
        updates.push(Update::RemoteTransfer(project, Some(id), result));
        Ok(())
    }

    async fn remote_transfer_rpc(
        &self,
        project: &str,
        operation: terminator_core::remote::Operation,
    ) -> Result<Option<terminator_core::remote::TransferStatus>, crate::remote_ui::Failure> {
        match self
            .client()
            .rpc(Request::RemoteProject {
                project: project.to_owned(),
                operation,
            })
            .await
        {
            Ok(Response::RemoteProject(terminator_core::remote::Reply::Transfer(status))) => {
                Ok(Some(status))
            }
            Ok(_) => Err(String::from("Unexpected SSH transfer response").into()),
            Err(e) => Err(crate::remote_ui::Failure::from_error(&e)),
        }
    }

    pub(in crate::gui_services) async fn host_key(
        &self,
        ssh: terminator_core::ssh::SshOptions,
        keys: Option<Vec<terminator_core::ssh::HostKey>>,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let client = self.client();
        if let Some(keys) = keys {
            let result = match client
                .rpc(Request::TrustHostKey {
                    ssh: ssh.clone(),
                    keys,
                })
                .await
            {
                Ok(Response::Ok) => Ok(()),
                Ok(_) => Err("Unexpected host key response".into()),
                Err(e) => Err(format!("{e:#}")),
            };
            updates.push(Update::HostKeyTrusted(ssh, result));
        } else {
            let result = match client.rpc(Request::ScanHostKey { ssh: ssh.clone() }).await {
                Ok(Response::HostKeys(scan)) => Ok(scan),
                Ok(_) => Err("Unexpected host key response".into()),
                Err(e) => Err(format!("{e:#}")),
            };
            updates.push(Update::HostKeyScanned(ssh, result));
        }
        Ok(())
    }

    /// `ssh -G` runs once per alias, so this stays off the GUI thread.
    pub(in crate::gui_services) async fn ssh_config_hosts(
        &self,
        generation: u64,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let hosts = self
            .cpu()
            .run(&cancel, || anyhow::Ok(terminator_core::ssh::config_hosts()))
            .await?;
        updates.push(Update::SshConfigHosts(generation, hosts));
        Ok(())
    }

    pub(in crate::gui_services) async fn remote_asset(
        &self,
        input: RemoteAssetInput<'_>,
    ) -> Result<()> {
        let RemoteAssetInput {
            project,
            path,
            cache_root,
            after,
            cancel,
            updates,
        } = input;
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
