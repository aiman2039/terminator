//! SSH remote and VNC job dispatch.
use super::{MISROUTED, Services};
use crate::gui_services::jobs::{
    RemoteAddInput, RemoteAssetInput, RemoteTransferInput, VncFrameInput, VncPickTransferInput,
};
use crate::{Job, Update};
use anyhow::Result;
use terminator_core::async_service::CancellationToken;

impl Services {
    pub(super) async fn remote_job(
        &self,
        job: Job,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        match job {
            Job::RemoteAdd {
                project,
                probe,
                generation,
            } => {
                self.remote_add(RemoteAddInput {
                    project,
                    probe,
                    generation,
                    updates,
                })
                .await
            }
            Job::RemoteRead { project, operation } => {
                self.remote_read(project, operation, updates).await
            }
            Job::HostKey { ssh, keys } => self.host_key(ssh, keys, updates).await,
            Job::SshConfigHosts(generation) => {
                self.ssh_config_hosts(generation, cancel, updates).await
            }
            Job::Forwards(request) => self.forwards(request, updates).await,
            Job::SshControlStatus(targets) => {
                updates.push(Update::SshControlStatus(
                    crate::hosts_ui::check_controls(targets).await,
                ));
                Ok(())
            }
            Job::RemoteAsset {
                project,
                path,
                cache_root,
                after,
            } => {
                self.remote_asset(RemoteAssetInput {
                    project,
                    path,
                    cache_root,
                    after,
                    cancel,
                    updates,
                })
                .await
            }
            Job::RemoteTransfer { project, pick } => {
                self.remote_transfer(RemoteTransferInput {
                    project,
                    pick,
                    cancel,
                    updates,
                })
                .await
            }
            Job::RemoteTransferPoll { project, id } => {
                self.remote_transfer_poll(project, id, updates).await
            }
            _ => anyhow::bail!(MISROUTED),
        }
    }

    pub(super) async fn vnc_job(
        &self,
        job: Job,
        cancel: CancellationToken,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        match job {
            Job::VncPickTransfer {
                session,
                direction,
                remote,
            } => {
                self.vnc_pick_transfer(VncPickTransferInput {
                    session,
                    direction,
                    remote,
                    cancel,
                    updates,
                })
                .await
            }
            Job::VncTransferPoll { session, id } => {
                self.vnc_transfer_poll(session, id, updates).await
            }
            Job::VncConnectTab { tab_id, request } => {
                self.vnc_connect_tab(tab_id, request, updates).await
            }
            Job::VncFrame {
                session,
                since,
                regions,
                shm,
            } => {
                self.vnc_frame(VncFrameInput {
                    session,
                    since,
                    regions,
                    shm,
                    cancel,
                    updates,
                })
                .await
            }
            _ => anyhow::bail!(MISROUTED),
        }
    }
}
