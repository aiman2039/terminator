//! Native file pickers run on the worker pool; network transfers run in the daemon.
use super::*;
use terminator_core::vnc::{TransferDirection, Transport};
pub(super) struct PanelInput<'a> {
    pub(super) app: &'a mut App,
    pub(super) ui: &'a mut egui::Ui,
    pub(super) sid: &'a str,
    pub(super) view_only: bool,
}

pub(super) fn panel(input: PanelInput<'_>) {
    let PanelInput {
        app,
        ui,
        sid,
        view_only,
    } = input;
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
            ui.label("Upload folder for files dropped on the screen");ui.add(appearance::singleline(&mut view.upload_dir).hint_text("/home/user/Desktop").desired_width(f32::INFINITY));
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
            local: None,
        });
    }
    if let Some(id) = poll {
        let _ = app.jobs.send(Job::VncTransferPoll {
            session: sid.into(),
            id,
        });
    }
}

pub(super) struct DropInput<'a> {
    pub(super) app: &'a mut App,
    pub(super) ui: &'a egui::Ui,
    pub(super) sid: &'a str,
    /// The remote screen's area in the pane.
    pub(super) area: egui::Rect,
}

/// Files dragged over the remote screen of a writable SSH session upload
/// over SFTP into the view's upload folder. One file at a time, like the
/// Files panel; SFTP paths must be absolute, so the folder must be set.
pub(super) fn drop_upload(input: DropInput<'_>) {
    let DropInput { app, ui, sid, area } = input;
    let Some(view) = app.vnc_views.get(sid) else {
        return;
    };
    let busy = view.transfer_status.as_ref().is_some_and(|s| !s.done);
    if view.draft_options.transport != Transport::Ssh || busy {
        return;
    }
    let over = ui.rect_contains_pointer(area) || ui.input(|i| i.pointer.hover_pos().is_none());
    let hovering = ui.input(|i| !i.raw.hovered_files.is_empty());
    if hovering && over {
        let text = if view.upload_dir.starts_with('/') {
            format!("Drop to upload to {}", view.upload_dir)
        } else {
            "Set an upload folder in Files (SFTP) to drop files here".to_owned()
        };
        paint_drop_hint(ui, area, &text);
    }
    if !over || ui.input(|i| i.raw.dropped_files.is_empty()) {
        return;
    }
    // Taken, like the explorer does, so no other pane also handles them.
    let dropped: Vec<std::path::PathBuf> = ui.ctx().input_mut(|i| {
        std::mem::take(&mut i.raw.dropped_files)
            .into_iter()
            .map(|file| file.path().to_path_buf())
            .filter(|path| !path.as_os_str().is_empty())
            .collect()
    });
    let Some(local) = dropped.first() else {
        return;
    };
    match remote_for_drop(&view.upload_dir, local) {
        Ok(remote) => {
            if dropped.len() > 1 {
                app.error = Some("Uploading the first file; drop one file at a time".into());
            }
            let _ = app.jobs.send(Job::VncPickTransfer {
                session: sid.into(),
                direction: TransferDirection::Upload,
                remote,
                local: Some(local.clone()),
            });
        }
        Err(error) => app.error = Some(error.into()),
    }
}

fn paint_drop_hint(ui: &egui::Ui, area: egui::Rect, text: &str) {
    let visuals = ui.visuals();
    let painter = ui.painter();
    painter.rect_filled(area, 6.0, visuals.selection.bg_fill.gamma_multiply(0.25));
    painter.rect_stroke(
        area,
        6.0,
        visuals.selection.stroke,
        egui::StrokeKind::Inside,
    );
    painter.text(
        area.center(),
        egui::Align2::CENTER_CENTER,
        text,
        egui::FontId::proportional(15.0),
        visuals.strong_text_color(),
    );
}

/// Remote path for a dropped file: `<folder>/<file name>`.
pub(super) fn remote_for_drop(
    folder: &str,
    local: &std::path::Path,
) -> Result<String, &'static str> {
    let folder = folder.trim();
    if !folder.starts_with('/') {
        return Err("Set an absolute upload folder in Files (SFTP) before dropping files");
    }
    let folder = folder.trim_end_matches('/');
    let name = local
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty() && !n.contains('/'))
        .ok_or("The dropped item has no file name")?;
    Ok(format!("{folder}/{name}"))
}
