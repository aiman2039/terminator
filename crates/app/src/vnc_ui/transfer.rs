//! Native file pickers run on the worker pool; network transfers run in the daemon.
use super::*;
use terminator_core::vnc::{TransferDirection, Transport};
pub(super) fn panel(app: &mut App, ui: &mut egui::Ui, sid: &str, view_only: bool) {
    let mut pick = None;
    let mut poll = None;
    {
        let view = app.vnc_views.entry(sid.into()).or_default();
        if view.draft_options.transport != Transport::Ssh {
            return;
        }
        egui::CollapsingHeader::new("Files (SFTP)").show(ui,|ui|{
            ui.weak("Uses the SSH host. Files are limited to 1 GiB. Existing destinations are never replaced.");
            ui.label("Remote file path");ui.add(appearance::singleline(&mut view.transfer_remote).hint_text("/home/user/file.txt").desired_width(f32::INFINITY));
            let busy=view.transfer_status.as_ref().is_some_and(|s|!s.done);
            ui.add_enabled_ui(!view_only&&!busy,|ui|{
                ui.horizontal(|ui|{
                    if ui.button("Upload file…").clicked(){pick=Some(TransferDirection::Upload);}
                    if ui.button("Download file…").clicked(){pick=Some(TransferDirection::Download);}
                });
            });
            if let Some(status)=&view.transfer_status{
                ui.label(if let Some(error)=&status.error{format!("Transfer failed: {error}")}else if status.done{format!("Transfer complete: {} bytes",status.bytes)}else{format!("Transferring: {} bytes",status.bytes)});
                if !status.done&&view.transfer_poll.is_none_or(|at|at.elapsed()>=Duration::from_millis(500)){poll=Some(status.id.clone());view.transfer_poll=Some(Instant::now());}
            }
        });
    }
    if let Some(direction) = pick {
        let remote = app
            .vnc_views
            .get(sid)
            .map_or_else(String::new, |v| v.transfer_remote.clone());
        if !remote.starts_with('/') {
            app.error =
                Some("Enter an absolute remote file path before starting a transfer".into());
            return;
        }
        let _ = app.jobs.send(Job::VncPickTransfer {
            session: sid.into(),
            direction,
            remote,
        });
    }
    if let Some(id) = poll {
        let _ = app.jobs.send(Job::VncTransferPoll {
            session: sid.into(),
            id,
        });
    }
}
