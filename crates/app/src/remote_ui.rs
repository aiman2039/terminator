//! Remote project controls and data. Local refresh/watch services are never used.
use crate::{App, Job, Tab, appearance, preferences::SidebarTool};
use eframe::egui;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use terminator_core::{
    Project, Request,
    remote::{Change, Entry, Operation, RemoteProject, Reply},
};

pub(crate) enum FormResult {
    Probe(String),
    Added(Box<terminator_core::State>),
}
#[derive(Default)]
pub(crate) struct RemoteUi {
    pub active: Option<RemoteProject>,
    pub open: bool,
    pub draft: Option<RemoteProject>,
    pub error: Option<String>,
    pub busy: bool,
    pub views: HashMap<String, View>,
}
#[derive(Default)]
pub(crate) struct View {
    pub pending_path: Option<String>,
    pub path: String,
    pub entries: Vec<Entry>,
    pub branch: String,
    pub changes: Vec<Change>,
    pub file_error: Option<String>,
    pub git_error: Option<String>,
    pub files_loading: bool,
    pub git_loading: bool,
    pub files_polled: Option<Instant>,
    pub git_polled: Option<Instant>,
}
impl RemoteUi {
    pub(crate) fn open_new(&mut self) {
        self.open = true;
        self.active = None;
        self.error = None;
        self.busy = false;
        self.draft = Some(RemoteProject {
            id: String::new(),
            name: String::new(),
            root: String::from("/home"),
            ssh: terminator_core::ssh::SshOptions::default(),
            layout: serde_json::Value::Null,
        });
    }
}
pub(crate) fn location_label(session: &terminator_core::Session) -> String {
    session.remote.as_ref().map_or_else(
        || session.cwd.display().to_string(),
        |r| {
            format!(
                "ssh://{}@{}:{}{}",
                r.ssh.username, r.ssh.host, r.ssh.port, r.cwd
            )
        },
    )
}
pub(crate) fn snapshot_root(data: &Path, project: &str) -> PathBuf {
    data.join("remote-files").join(project)
}
pub(crate) fn is_snapshot(data: &Path, path: &Path) -> bool {
    path.starts_with(data.join("remote-files"))
}
pub(crate) fn projection(remote: &RemoteProject) -> Project {
    Project {
        id: remote.id.clone(),
        name: format!("SSH · {}", remote.name),
        path: remote.identity_path(),
        layout: remote.layout.clone(),
    }
}
impl App {
    pub(crate) fn remote_file_source(&self, path: &Path) -> Option<(RemoteProject, String)> {
        for project in &self.state.remote_projects {
            let cache = snapshot_root(&self.paths.data, &project.id);
            let identity = project.identity_path();
            if let Ok(relative) = path
                .strip_prefix(&cache)
                .or_else(|_| path.strip_prefix(&identity))
            {
                let relative = relative.to_str()?.to_owned();
                if terminator_core::remote::validate_relative(&relative).is_ok() {
                    return Some((project.clone(), relative));
                }
            }
        }
        None
    }
    pub(crate) fn remote_file_label(&self, path: &Path) -> Option<String> {
        self.remote_file_source(path)
            .map(|(p, relative)| format!("{}/{}", p.label().trim_end_matches('/'), relative))
    }
    pub(crate) fn selected_remote_project(&self) -> Option<&RemoteProject> {
        self.selected
            .as_ref()
            .and_then(|id| self.state.remote_projects.iter().find(|p| &p.id == id))
    }
    pub(crate) fn remote_supported(&self) -> bool {
        self.connected
            && self
                .state
                .capabilities
                .iter()
                .any(|c| c == terminator_core::remote::CAPABILITY)
    }
    pub(crate) fn project_location_label(&self, id: &str) -> String {
        self.state
            .remote_projects
            .iter()
            .find(|p| p.id == id)
            .map_or_else(
                || {
                    self.state
                        .projects
                        .iter()
                        .find(|p| p.id == id)
                        .map_or_else(String::new, |p| p.path.display().to_string())
                },
                RemoteProject::label,
            )
    }
    pub(crate) fn remote_project_dialog(&mut self, ctx: &egui::Context) {
        if !self.remote_ui.open {
            return;
        }
        let supported = self.remote_supported();
        let mut submit = false;
        let mut probe = false;
        let mut cancel = false;
        egui::Window::new("SSH project").id(egui::Id::unique("ssh-project-dialog")).default_width(440.0).resizable(true).show(ctx,|ui|{
            ui.weak("Each terminal connects to this host. Files and Git stay remote. OpenSSH keys or an agent are required. Text editing uses Neovim on the SSH host.");
            if !supported{ui.colored_label(ui.visuals().warn_fg_color,"Restart the session service to enable SSH projects.");}
            if let Some(error)=&self.remote_ui.error{ui.colored_label(ui.visuals().error_fg_color,error);}
            let Some(draft)=&mut self.remote_ui.draft else{return;};
            for (label,value) in [("Name",&mut draft.name),("SSH host or config alias",&mut draft.ssh.host),("SSH user (optional)",&mut draft.ssh.username),("Remote project path",&mut draft.root)]{
                ui.label(label);ui.add(appearance::singleline(value).desired_width(f32::INFINITY));
            }
            ui.horizontal(|ui|{ui.label("SSH port");ui.add(egui::DragValue::new(&mut draft.ssh.port).range(1..=65535));});
            for (label,value) in [("Identity file (optional)",&mut draft.ssh.identity_file),("Known hosts file (optional)",&mut draft.ssh.known_hosts_file)]{
                ui.label(label);let mut text=value.as_ref().map_or_else(String::new,|p|p.to_string_lossy().into_owned());
                if ui.add(appearance::singleline(&mut text).desired_width(f32::INFINITY)).changed(){*value=(!text.trim().is_empty()).then(||PathBuf::from(text.trim()));}
            }
            ui.weak("Host keys are checked strictly. The project is not mounted or copied onto this computer.");
            ui.add_enabled_ui(supported&&!self.remote_ui.busy,|ui|{ui.horizontal(|ui|{probe=ui.button("Test connection").clicked();submit=ui.button("Add SSH project").clicked();});});
            cancel=ui.button("Cancel").clicked();
        });
        if cancel {
            self.remote_ui.open = false;
            self.remote_ui.active = None;
            self.selection_generation = self.selection_generation.wrapping_add(1);
            return;
        }
        if (probe || submit)
            && let Some(project) = self.remote_ui.draft.clone()
        {
            match project.validate() {
                Ok(()) => {
                    self.remote_ui.busy = true;
                    self.remote_ui.active = Some(project.clone());
                    let _ = self.jobs.send(Job::RemoteAdd {
                        project,
                        probe,
                        generation: self.selection_generation,
                    });
                }
                Err(error) => self.remote_ui.error = Some(error.to_string()),
            }
        }
    }
    pub(crate) fn remote_sidebar(&mut self, ui: &mut egui::Ui, tool: SidebarTool) {
        let Some(project) = self.selected_remote_project().cloned() else {
            return;
        };
        ui.strong(&project.name);
        ui.weak(project.label());
        if !self.remote_supported() {
            ui.label("The active service does not support SSH projects. Existing terminals are preserved.");
            return;
        }
        if tool == SidebarTool::Info {
            ui.label("Terminals run through OpenSSH. File access uses SFTP. Git changes are made in the SSH terminal.");
            return;
        }
        if matches!(tool, SidebarTool::Problems | SidebarTool::Structure) {
            ui.label("Use tools in the SSH terminal for remote diagnostics and symbols.");
            return;
        }
        let git = tool == SidebarTool::Git;
        let mut request = None;
        let mut open = None;
        let mut diff = None;
        {
            let view = self.remote_ui.views.entry(project.id.clone()).or_default();
            let refresh =
                appearance::sidebar_action(ui, "RefreshCw", "Refresh remote project").clicked();
            if git {
                if refresh
                    || (!view.git_loading
                        && view
                            .git_polled
                            .is_none_or(|at| at.elapsed() >= Duration::from_secs(3)))
                {
                    view.git_loading = true;
                    view.git_polled = Some(Instant::now());
                    request = Some(Operation::GitStatus);
                }
                if let Some(error) = &view.git_error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                ui.label(format!("Branch: {}", view.branch));
                ui.weak("Remote diffs are read-only. Use the SSH terminal to stage or discard.");
                egui::ScrollArea::vertical()
                    .id_salt(("remote-git", &project.id))
                    .show(ui, |ui| {
                        for change in &view.changes {
                            ui.horizontal(|ui| {
                                let x = change.status.as_bytes().first().copied().unwrap_or(b' ');
                                let y = change.status.as_bytes().get(1).copied().unwrap_or(b' ');
                                let response = ui.selectable_label(
                                    false,
                                    format!("{}  {}", change.status, change.path),
                                );
                                #[cfg(feature = "test-support")]
                                crate::diagnostics::record(
                                    ui.ctx(),
                                    &format!("remote-git:{}:{}", project.id, change.path),
                                    response.rect,
                                );
                                if response.clicked() {
                                    diff = Some((change.path.clone(), y == b' ' && x != b'?'));
                                }
                                if appearance::sidebar_action(ui, "FileCode", "Open remote file")
                                    .clicked()
                                {
                                    open = Some((change.path.clone(), false));
                                }
                            });
                        }
                        if view.changes.is_empty() && !view.git_loading && view.git_error.is_none()
                        {
                            ui.weak("Working tree is clean.");
                        }
                    });
            } else {
                ui.horizontal(|ui| {
                    if ui.button("↑ Project root").clicked() {
                        view.path.clear();
                        view.files_loading = false;
                        view.pending_path = None;
                        view.files_polled = None;
                    }
                    ui.label(&view.path);
                });
                if refresh
                    || (!view.files_loading
                        && view
                            .files_polled
                            .is_none_or(|at| at.elapsed() >= Duration::from_secs(3)))
                {
                    view.files_loading = true;
                    view.pending_path = Some(view.path.clone());
                    view.files_polled = Some(Instant::now());
                    request = Some(Operation::List {
                        path: view.path.clone(),
                    });
                }
                if let Some(error) = &view.file_error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                let entries = view.entries.clone();
                egui::ScrollArea::vertical()
                    .id_salt(("remote-files", &project.id))
                    .show(ui, |ui| {
                        for entry in entries {
                            let label = format!(
                                "{} {}",
                                if entry.directory { "▸" } else { "" },
                                entry.name
                            );
                            let response = ui.selectable_label(false, label);
                            #[cfg(feature = "test-support")]
                            crate::diagnostics::record(
                                ui.ctx(),
                                &format!("remote-file:{}:{}", project.id, entry.path),
                                response.rect,
                            );
                            if response.clicked() {
                                if entry.directory {
                                    view.path.clone_from(&entry.path);
                                    view.entries.clear();
                                    view.files_loading = false;
                                    view.pending_path = None;
                                    view.files_polled = None;
                                } else {
                                    open = Some((entry.path.clone(), false));
                                }
                            }
                            if !entry.directory {
                                response.context_menu(|ui| {
                                    if ui.button("Open in SSH editor").clicked() {
                                        open = Some((entry.path.clone(), true));
                                        ui.close();
                                    }
                                });
                            }
                        }
                    });
            }
        }
        if let Some(operation) = request {
            let _ = self.jobs.send(Job::RemoteRead {
                project: project.id.clone(),
                operation,
            });
        }
        if let Some((path, text)) = open {
            self.open_remote_file(&project, path, None, None, text);
        }
        if let Some((path, staged)) = diff {
            let tab = Tab::Diff {
                cwd: project.identity_path(),
                path: project.identity_path().join(path),
                staged,
                remote: Some(project.id.clone()),
            };
            let after = self.editor_target(&project.id, None, None);
            self.place_gui_tab(project.id, tab, after);
        }
    }
    pub(crate) fn open_remote_file(
        &mut self,
        project: &RemoteProject,
        path: String,
        line: Option<u32>,
        split: Option<&str>,
        text: bool,
    ) {
        if !self.remote_supported() {
            self.error = Some("Restart the session service to open SSH project files".into());
            return;
        }
        if let Err(error) = terminator_core::remote::validate_relative(&path) {
            self.error = Some(error.to_string());
            return;
        }
        let after = self.editor_target(&project.id, None, split);
        let file = Path::new(&path);
        let preview = !text
            && (crate::image_preview::supported(file)
                || crate::browser::supported_file(file)
                || crate::player::supported(file)
                || file
                    .extension()
                    .is_some_and(|ext| ext == "md" || ext == "markdown"));
        if preview {
            let _ = self.jobs.send(Job::RemoteAsset {
                project: project.id.clone(),
                path,
                cache_root: snapshot_root(&self.paths.data, &project.id),
                after,
            });
        } else {
            let _ = self.jobs.send(Job::rpc(
                Request::Create {
                    project: project.id.clone(),
                    cwd: None,
                    file: Some(path.into()),
                    line,
                    column: None,
                    editor: true,
                },
                after,
            ));
        }
    }
    pub(crate) fn apply_remote_reply(
        &mut self,
        project: String,
        operation: Operation,
        result: Result<Reply, String>,
    ) {
        if !self.state.remote_projects.iter().any(|p| p.id == project) {
            return;
        }
        let view = self.remote_ui.views.entry(project).or_default();
        match (&operation, result) {
            (
                Operation::List { path },
                Ok(Reply::Directory {
                    path: actual,
                    entries,
                }),
            ) if view.pending_path.as_ref() == Some(path) && path == &actual => {
                view.files_loading = false;
                view.pending_path = None;
                view.files_polled = Some(Instant::now());
                if path == &view.path {
                    view.entries = entries;
                    view.file_error = None;
                }
            }
            (Operation::List { path }, Err(error)) if view.pending_path.as_ref() == Some(path) => {
                view.files_loading = false;
                view.pending_path = None;
                view.files_polled = Some(Instant::now());
                view.file_error = Some(error);
            }
            (Operation::GitStatus, Ok(Reply::Git { branch, changes })) => {
                view.git_loading = false;
                view.git_polled = Some(Instant::now());
                view.branch = branch;
                view.changes = changes;
                view.git_error = None;
            }
            (Operation::GitStatus, Err(error)) => {
                view.git_loading = false;
                view.git_polled = Some(Instant::now());
                view.git_error = Some(error);
            }
            _ => {}
        }
    }
}
