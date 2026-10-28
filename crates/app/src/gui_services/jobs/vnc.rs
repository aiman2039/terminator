//! VNC transfer, connection, and frame jobs.
use super::super::submit::Services;
use crate::Update;
use anyhow::Result;
use terminator_core::async_service::CancellationToken;
use terminator_core::{Request, Response};

pub(crate) struct VncPickTransferInput<'a> {
    pub(in crate::gui_services) session: String,
    pub(in crate::gui_services) direction: terminator_core::vnc::TransferDirection,
    pub(in crate::gui_services) remote: String,
    pub(in crate::gui_services) cancel: CancellationToken,
    pub(in crate::gui_services) updates: &'a mut Vec<Update>,
}

pub(crate) struct VncFrameInput<'a> {
    pub(in crate::gui_services) session: String,
    pub(in crate::gui_services) since: u64,
    pub(in crate::gui_services) regions: bool,
    pub(in crate::gui_services) cancel: CancellationToken,
    pub(in crate::gui_services) updates: &'a mut Vec<Update>,
}

impl Services {
    pub(in crate::gui_services) async fn vnc_pick_transfer(
        &self,
        input: VncPickTransferInput<'_>,
    ) -> Result<()> {
        let VncPickTransferInput {
            session,
            direction,
            remote,
            cancel,
            updates,
        } = input;
        let client = self.client();
        let upload = direction == terminator_core::vnc::TransferDirection::Upload;
        let picked = self
            .fs()
            .run(&cancel, move || {
                let dialog = rfd::FileDialog::new().set_title(if upload {
                    "Upload via SFTP"
                } else {
                    "Save SFTP download to a new file"
                });
                anyhow::Ok(if upload {
                    dialog.pick_file()
                } else {
                    dialog.save_file()
                })
            })
            .await;
        let result = match picked {
            Ok(Some(local)) => {
                let id = terminator_core::id();
                let request = Request::VncTransfer {
                    session: session.clone(),
                    transfer: terminator_core::vnc::VncTransfer {
                        id: id.clone(),
                        direction,
                        local,
                        remote,
                    },
                };
                match client.rpc(request).await {
                    Ok(Response::Ok) => Ok(Some(terminator_core::vnc::VncTransferStatus {
                        id,
                        done: false,
                        bytes: 0,
                        error: None,
                    })),
                    Ok(_) => Err("Unexpected SFTP transfer response".into()),
                    Err(e) => Err(format!("{e}", e = terminator_core::ErrorChain(&e))),
                }
            }
            Ok(None) => Ok(None),
            Err(e) => Err(format!("{e:#}")),
        };
        updates.push(Update::VncTransfer(session, result));
        Ok(())
    }

    pub(in crate::gui_services) async fn vnc_transfer_poll(
        &self,
        session: String,
        id: String,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let client = self.client();
        let result = match client
            .rpc(Request::VncTransferStatus {
                session: session.clone(),
                id,
            })
            .await
        {
            Ok(Response::VncTransferStatus(status)) => Ok(Some(status)),
            Ok(_) => Err("Unexpected SFTP transfer status".into()),
            Err(e) => Err(format!("{e}", e = terminator_core::ErrorChain(&e))),
        };
        updates.push(Update::VncTransfer(session, result));
        Ok(())
    }

    pub(in crate::gui_services) async fn vnc_connect_tab(
        &self,
        tab_id: String,
        request: Request,
        updates: &mut Vec<Update>,
    ) -> Result<()> {
        let client = self.client();
        let result = match client.rpc(request).await {
            Ok(Response::VncCreated(session)) => {
                if let Ok(Response::State(state)) = client.rpc(Request::Snapshot).await {
                    updates.push(Update::State(state));
                }
                Ok(session)
            }
            Ok(_) => Err("Unexpected VNC connection response".into()),
            Err(err) => Err(format!("{err}", err = terminator_core::ErrorChain(&err))),
        };
        updates.push(Update::VncTabConnected(tab_id, result));
        Ok(())
    }

    pub(in crate::gui_services) async fn vnc_frame(&self, input: VncFrameInput<'_>) -> Result<()> {
        let VncFrameInput {
            session,
            since,
            regions,
            cancel,
            updates,
        } = input;
        let client = self.client();
        let result = match client
            .rpc(Request::VncFrame {
                session: session.clone(),
                since,
                regions,
            })
            .await
        {
            Ok(Response::VncFrame(frame)) => self
                .cpu()
                .run(&cancel, move || crate::vnc_ui::decode_frame(*frame))
                .await
                .map(Some)
                .map_err(|e| format!("{e:#}")),
            Ok(Response::Unchanged) => Ok(None),
            Ok(_) => Err("Unexpected VNC frame response".into()),
            Err(err) => Err(format!("{err}", err = terminator_core::ErrorChain(&err))),
        };
        updates.push(Update::VncFrame(session, result));
        Ok(())
    }
}
