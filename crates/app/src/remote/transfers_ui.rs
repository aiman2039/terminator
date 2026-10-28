//! SSH project uploads and downloads. Native pickers run on the worker
//! pool; the transfer itself runs in the daemon and is polled every 500 ms.
use crate::{App, Job, appearance, remote_ui::Failure};
use eframe::egui;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use terminator_core::remote::{Operation, TransferStatus};

const POLL: Duration = Duration::from_millis(500);
/// Finished rows kept in a project's explorer.
const KEEP: usize = 6;

/// How a transfer's local side is chosen.
#[derive(Clone, Debug)]
pub(crate) enum Pick {
    /// Pick a local file to upload into the project directory.
    Upload { remote_dir: String },
    /// Pick a new local file to save the project file `path` to.
    Download { path: String, name: String },
    /// A file dropped on the explorer; no picker.
    Dropped { local: PathBuf, remote_dir: String },
}
impl Pick {
    /// Shows the native picker, if any. `None` when the user cancelled.
    pub(crate) fn resolve(self) -> Option<Operation> {
        match self {
            Self::Dropped { local, remote_dir } => Some(Operation::Upload { local, remote_dir }),
            Self::Upload { remote_dir } => rfd::FileDialog::new()
                .set_title("Upload to SSH project")
                .pick_file()
                .map(|local| Operation::Upload { local, remote_dir }),
            Self::Download { path, name } => rfd::FileDialog::new()
                .set_title("Save download to a new file")
                .set_file_name(name)
                .save_file()
                .map(|local| Operation::Download { path, local }),
        }
    }
}

#[derive(Default)]
pub(crate) struct Transfers {
    pub list: Vec<TransferStatus>,
    pub polled: Option<Instant>,
    /// A transfer that could not start.
    pub error: Option<String>,
}
impl Transfers {
    /// Inserts or replaces `status`. Returns whether it just finished.
    fn record(&mut self, status: TransferStatus) -> bool {
        let finished = status.done;
        if let Some(existing) = self.list.iter_mut().find(|s| s.id == status.id) {
            let was = existing.done;
            *existing = status;
            return finished && !was;
        }
        self.list.push(status);
        while self.list.len() > KEEP {
            let Some(index) = self.list.iter().position(|s| s.done) else {
                break;
            };
            self.list.remove(index);
        }
        finished
    }
    /// Draws progress rows; returns transfers due for a status poll.
    pub(crate) fn panel(&mut self, ui: &mut egui::Ui, failed: egui::Color32) -> Vec<String> {
        let mut dismiss_error = false;
        if let Some(error) = &self.error {
            ui.horizontal(|ui| {
                dismiss_error = appearance::sidebar_action(ui, "X", "Dismiss").clicked();
                ui.colored_label(failed, error);
            });
        }
        if dismiss_error {
            self.error = None;
        }
        let mut dismiss = None;
        for status in &self.list {
            ui.horizontal(|ui| {
                if status.done && appearance::sidebar_action(ui, "X", "Dismiss").clicked() {
                    dismiss = Some(status.id.clone());
                }
                let arrow = if status.upload { "↑" } else { "↓" };
                let size = crate::session_info::format_bytes(status.bytes);
                if let Some(error) = &status.error {
                    ui.colored_label(failed, format!("{arrow} {}: {error}", status.name));
                } else if status.done {
                    ui.label(format!(
                        "{arrow} {} {} ({size})",
                        status.name,
                        if status.upload {
                            "uploaded"
                        } else {
                            "downloaded"
                        }
                    ));
                } else if let Some(total) = status.total.filter(|t| *t > 0) {
                    // Display ratio only.
                    let fraction = status.bytes as f32 / total as f32;
                    ui.add(
                        egui::ProgressBar::new(fraction.clamp(0.0, 1.0))
                            .text(format!(
                                "{arrow} {} {size} / {}",
                                status.name,
                                crate::session_info::format_bytes(total)
                            ))
                            .desired_width(ui.available_width()),
                    );
                } else {
                    ui.spinner();
                    ui.label(format!("{arrow} {} {size}", status.name));
                }
            });
        }
        if let Some(id) = dismiss {
            self.list.retain(|s| s.id != id);
        }
        let active: Vec<String> = self
            .list
            .iter()
            .filter(|s| !s.done)
            .map(|s| s.id.clone())
            .collect();
        if active.is_empty() {
            return active;
        }
        ui.ctx().request_repaint_after(POLL);
        if self.polled.is_some_and(|at| at.elapsed() < POLL) {
            return Vec::new();
        }
        self.polled = Some(Instant::now());
        active
    }
}

impl App {
    pub(crate) fn transfers_supported(&self) -> bool {
        self.connected
            && self
                .state
                .capabilities
                .iter()
                .any(|c| c == terminator_core::remote::TRANSFER_CAPABILITY)
    }
    pub(crate) fn start_remote_transfer(&mut self, project: &str, pick: Pick) {
        if !self.transfers_supported() {
            self.error = Some("Restart the session service to transfer SSH project files".into());
            return;
        }
        let _ = self.jobs.send(Job::RemoteTransfer {
            project: project.to_owned(),
            pick,
        });
    }
    pub(crate) fn poll_remote_transfers(&mut self, project: &str, ids: Vec<String>) {
        for id in ids {
            let _ = self.jobs.send(Job::RemoteTransferPoll {
                project: project.to_owned(),
                id,
            });
        }
    }
    /// Uploads files dropped on the window while this project's explorer is
    /// shown into its current directory. Consumes the drop.
    pub(crate) fn drop_uploads(&mut self, ui: &egui::Ui, project: &str, remote_dir: &str) {
        if !self.transfers_supported() {
            return;
        }
        let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        if hovering {
            let rect = ui.max_rect();
            ui.painter().rect_stroke(
                rect,
                4.0,
                egui::Stroke::new(2.0, ui.visuals().selection.stroke.color),
                egui::StrokeKind::Inside,
            );
            ui.ctx().request_repaint();
        }
        let dropped = ui
            .ctx()
            .input_mut(|i| std::mem::take(&mut i.raw.dropped_files));
        for file in dropped {
            self.start_remote_transfer(
                project,
                Pick::Dropped {
                    local: file.path().to_path_buf(),
                    remote_dir: remote_dir.to_owned(),
                },
            );
        }
    }
    pub(crate) fn apply_remote_transfer(
        &mut self,
        project: String,
        id: Option<String>,
        result: Result<Option<TransferStatus>, Failure>,
    ) {
        let failure = match &result {
            Ok(Some(status)) if status.done => status.ssh.clone().map(|ssh| {
                let error = anyhow::Error::from(ssh);
                Failure::from_error(&error)
            }),
            Err(failure) => Some(failure.clone()),
            _ => None,
        };
        let view = self.remote_ui.views.entry(project.clone()).or_default();
        match result {
            Ok(None) => {}
            Ok(Some(mut status)) => {
                if let Some(failure) = &failure {
                    status.error = Some(failure.text.clone());
                }
                let refresh =
                    status.upload && status.error.is_none() && status.remote_dir == view.path;
                if view.transfers.record(status) && refresh {
                    view.files_polled = None;
                }
            }
            Err(failure) => match id
                .as_ref()
                .and_then(|id| view.transfers.list.iter_mut().find(|s| &s.id == id))
            {
                Some(status) => {
                    status.done = true;
                    status.error = Some(failure.text);
                }
                None => view.transfers.error = Some(failure.text),
            },
        }
        if let Some(failure) = failure {
            self.remote_ssh_failure(&project, &failure);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn status(id: &str, done: bool) -> TransferStatus {
        TransferStatus {
            id: id.into(),
            upload: true,
            name: "f".into(),
            remote_dir: String::new(),
            done,
            bytes: 0,
            total: None,
            error: None,
            ssh: None,
        }
    }
    #[test]
    fn a_transfer_finishes_once_and_old_finished_rows_are_dropped() {
        let mut transfers = Transfers::default();
        assert!(!transfers.record(status("a", false)));
        assert!(transfers.record(status("a", true)));
        assert!(!transfers.record(status("a", true)));
        for i in 0..KEEP {
            transfers.record(status(&format!("x{i}"), false));
        }
        assert_eq!(transfers.list.len(), KEEP);
        assert!(transfers.list.iter().all(|s| s.id != "a"));
    }
}
