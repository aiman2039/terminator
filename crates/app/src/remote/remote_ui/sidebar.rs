//! Remote project sidebar: SFTP file browser and read-only Git status.
use super::{OpenRemoteFileInput, View};
use crate::{App, Job, Tab, appearance, preferences::SidebarTool, transfers_ui::Pick};
use eframe::egui;
use std::time::{Duration, Instant};
use terminator_core::remote::{Change, Entry, Operation, RemoteProject};

/// Everything a sidebar pass asks the app to do once rendering is done.
#[derive(Default)]
struct SidebarActions {
    picks: Vec<Pick>,
    polls: Vec<String>,
    sign_in: bool,
    request: Option<Operation>,
    /// Remote path plus whether to force the SSH text editor.
    open: Option<(String, bool)>,
    /// Remote path plus whether to show the staged side.
    diff: Option<(String, bool)>,
}

/// One frame's view of a remote project while `view` is borrowed.
struct SidebarPass<'a> {
    view: &'a mut View,
    project_id: &'a str,
    refresh: bool,
    transfers: bool,
    failed: egui::Color32,
    upload: bool,
    actions: SidebarActions,
}

impl App {
    pub(crate) fn remote_sidebar(&mut self, ui: &mut egui::Ui, tool: SidebarTool) {
        let Some(project) = self.selected_remote_project().cloned() else {
            return;
        };
        if self.remote_sidebar_notice(ui, &project, tool) {
            return;
        }
        let git = tool == SidebarTool::Git;
        let transfers = !git && self.transfers_supported();
        let failed = appearance::color(&self.theme.status_failed);
        let (refresh, upload) = remote_toolbar(ui, &project.id, transfers);
        if refresh {
            self.remote_ui.host_key_declined.remove(&project.id);
        }
        let view = self.remote_ui.views.entry(project.id.clone()).or_default();
        let mut pass = SidebarPass {
            view,
            project_id: &project.id,
            refresh,
            transfers,
            failed,
            upload,
            actions: SidebarActions::default(),
        };
        pass.sign_in_row(ui);
        if git {
            pass.git(ui);
        } else {
            pass.files(ui);
        }
        let actions = pass.actions;
        if !git {
            self.drop_remote_uploads(ui, &project.id);
        }
        self.apply_sidebar_actions(&project, actions);
    }

    /// Header plus the tool states that show only a message. Returns `true`
    /// when nothing else should render.
    fn remote_sidebar_notice(
        &mut self,
        ui: &mut egui::Ui,
        project: &RemoteProject,
        tool: SidebarTool,
    ) -> bool {
        ui.strong(&project.name);
        ui.weak(project.label());
        if !self.remote_supported() {
            ui.label("The active service does not support SSH projects. Existing terminals are preserved.");
            return true;
        }
        if tool == SidebarTool::Info {
            ui.label("Terminals run through OpenSSH. File access uses SFTP. Git changes are made in the SSH terminal.");
            ui.separator();
            self.forwards_section(ui, project);
            return true;
        }
        if matches!(tool, SidebarTool::Problems | SidebarTool::Structure) {
            ui.label("Use tools in the SSH terminal for remote diagnostics and symbols.");
            return true;
        }
        false
    }

    fn drop_remote_uploads(&mut self, ui: &mut egui::Ui, project_id: &str) {
        if let Some(path) = self.remote_ui.views.get(project_id).map(|v| v.path.clone()) {
            self.drop_uploads(ui, project_id, &path);
        }
    }

    fn apply_sidebar_actions(&mut self, project: &RemoteProject, actions: SidebarActions) {
        let SidebarActions {
            picks,
            polls,
            sign_in,
            request,
            open,
            diff,
        } = actions;
        for pick in picks {
            self.start_remote_transfer(&project.id, pick);
        }
        self.poll_remote_transfers(&project.id, polls);
        if sign_in {
            self.select_project(project.id.clone());
            self.create(None);
        }
        if let Some(operation) = request {
            let _ = self.jobs.send(Job::RemoteRead {
                project: project.id.clone(),
                operation,
            });
        }
        if let Some((path, text)) = open {
            self.open_remote_file(OpenRemoteFileInput {
                project,
                path,
                line: None,
                split: None,
                text,
            });
        }
        if let Some((path, staged)) = diff {
            self.open_remote_diff(project, path, staged);
        }
    }

    fn open_remote_diff(&mut self, project: &RemoteProject, path: String, staged: bool) {
        let tab = Tab::Diff {
            cwd: project.identity_path(),
            path: project.identity_path().join(path),
            staged,
            remote: Some(project.id.clone()),
        };
        let after = self.editor_target(&project.id, None, None);
        self.place_gui_tab(project.id.clone(), tab, after);
    }
}

/// Refresh and upload buttons. Returns `(refresh, upload)` clicks.
fn remote_toolbar(ui: &mut egui::Ui, project_id: &str, transfers: bool) -> (bool, bool) {
    #[cfg(not(feature = "test-support"))]
    let _ = project_id;
    ui.horizontal(|ui| {
        let refresh =
            appearance::sidebar_action(ui, "RefreshCw", "Refresh remote project").clicked();
        let mut upload = false;
        if transfers {
            let response =
                appearance::sidebar_action(ui, "Upload", "Upload a file into this folder…");
            #[cfg(feature = "test-support")]
            crate::diagnostics::record(
                ui.ctx(),
                &format!("remote-upload:{project_id}"),
                response.rect,
            );
            upload = response.clicked();
        }
        (refresh, upload)
    })
    .inner
}

fn poll_due(loading: bool, polled: Option<Instant>) -> bool {
    !loading && polled.is_none_or(|at| at.elapsed() >= Duration::from_secs(3))
}

impl SidebarPass<'_> {
    fn sign_in_row(&mut self, ui: &mut egui::Ui) {
        if !self.view.sign_in {
            return;
        }
        ui.horizontal(|ui| {
            if ui.button("Open SSH terminal").clicked() {
                self.actions.sign_in = true;
            }
            ui.weak("Sign in there; this view retries afterwards.");
        });
    }

    fn git(&mut self, ui: &mut egui::Ui) {
        let view = &mut *self.view;
        if self.refresh || poll_due(view.git_loading, view.git_polled) {
            view.git_loading = true;
            view.git_polled = Some(Instant::now());
            self.actions.request = Some(Operation::GitStatus);
        }
        if let Some(error) = &view.git_error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        ui.label(format!("Branch: {}", view.branch));
        ui.weak("Remote diffs are read-only. Use the SSH terminal to stage or discard.");
        let project_id = self.project_id;
        let actions = &mut self.actions;
        egui::ScrollArea::vertical()
            .id_salt(("remote-git", project_id))
            .show(ui, |ui| {
                for change in &view.changes {
                    ui.horizontal(|ui| {
                        git_change_row(GitChangeRowInput {
                            ui,
                            change,
                            project_id,
                            actions,
                        })
                    });
                }
                if view.changes.is_empty() && !view.git_loading && view.git_error.is_none() {
                    ui.weak("Working tree is clean.");
                }
            });
    }

    fn files(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("↑ Project root").clicked() {
                self.view.path.clear();
                self.reset_listing();
            }
            ui.label(&self.view.path);
        });
        let view = &mut *self.view;
        if self.refresh || poll_due(view.files_loading, view.files_polled) {
            view.files_loading = true;
            view.pending_path = Some(view.path.clone());
            view.files_polled = Some(Instant::now());
            self.actions.request = Some(Operation::List {
                path: view.path.clone(),
            });
        }
        if let Some(error) = &view.file_error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        if self.upload {
            self.actions.picks.push(Pick::Upload {
                remote_dir: view.path.clone(),
            });
        }
        self.actions.polls = view.transfers.panel(ui, self.failed);
        let entries = view.entries.clone();
        egui::ScrollArea::vertical()
            .id_salt(("remote-files", self.project_id))
            .show(ui, |ui| {
                for entry in entries {
                    self.file_row(ui, &entry);
                }
            });
    }

    /// Forgets the in-flight listing so the next frame reloads it.
    fn reset_listing(&mut self) {
        self.view.files_loading = false;
        self.view.pending_path = None;
        self.view.files_polled = None;
    }

    fn file_row(&mut self, ui: &mut egui::Ui, entry: &Entry) {
        let label = format!("{} {}", if entry.directory { "▸" } else { "" }, entry.name);
        let response = ui.selectable_label(false, label);
        #[cfg(feature = "test-support")]
        crate::diagnostics::record(
            ui.ctx(),
            &format!("remote-file:{}:{}", self.project_id, entry.path),
            response.rect,
        );
        if response.clicked() {
            if entry.directory {
                self.view.path.clone_from(&entry.path);
                self.view.entries.clear();
                self.reset_listing();
            } else {
                self.actions.open = Some((entry.path.clone(), false));
            }
        }
        let transfers = self.transfers;
        response.context_menu(|ui| {
            file_menu(FileMenuInput {
                ui,
                entry,
                transfers,
                actions: &mut self.actions,
            })
        });
    }
}

struct GitChangeRowInput<'a> {
    ui: &'a mut egui::Ui,
    change: &'a Change,
    project_id: &'a str,
    actions: &'a mut SidebarActions,
}

fn git_change_row(input: GitChangeRowInput<'_>) {
    let GitChangeRowInput {
        ui,
        change,
        project_id,
        actions,
    } = input;
    #[cfg(not(feature = "test-support"))]
    let _ = project_id;
    let x = change.status.as_bytes().first().copied().unwrap_or(b' ');
    let y = change.status.as_bytes().get(1).copied().unwrap_or(b' ');
    let response = ui.selectable_label(false, format!("{}  {}", change.status, change.path));
    #[cfg(feature = "test-support")]
    crate::diagnostics::record(
        ui.ctx(),
        &format!("remote-git:{project_id}:{}", change.path),
        response.rect,
    );
    if response.clicked() {
        actions.diff = Some((change.path.clone(), y == b' ' && x != b'?'));
    }
    if appearance::sidebar_action(ui, "FileCode", "Open remote file").clicked() {
        actions.open = Some((change.path.clone(), false));
    }
}

struct FileMenuInput<'a> {
    ui: &'a mut egui::Ui,
    entry: &'a Entry,
    transfers: bool,
    actions: &'a mut SidebarActions,
}

fn file_menu(input: FileMenuInput<'_>) {
    let FileMenuInput {
        ui,
        entry,
        transfers,
        actions,
    } = input;
    if entry.directory {
        if ui
            .add_enabled(transfers, egui::Button::new("Upload into this folder…"))
            .clicked()
        {
            actions.picks.push(Pick::Upload {
                remote_dir: entry.path.clone(),
            });
            ui.close();
        }
        return;
    }
    if ui.button("Open in SSH editor").clicked() {
        actions.open = Some((entry.path.clone(), true));
        ui.close();
    }
    if ui
        .add_enabled(transfers, egui::Button::new("Download…"))
        .clicked()
    {
        actions.picks.push(Pick::Download {
            path: entry.path.clone(),
            name: entry.name.clone(),
        });
        ui.close();
    }
}
