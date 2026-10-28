//! Remote project controls and data. Local refresh/watch services are never used.
use crate::{App, Job, Tab, appearance, preferences::SidebarTool};
use eframe::egui;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(crate) struct OpenRemoteFileInput<'a> {
    pub(crate) project: &'a RemoteProject,
    pub(crate) path: String,
    pub(crate) line: Option<u32>,
    pub(crate) split: Option<&'a str>,
    pub(crate) text: bool,
}

use terminator_core::{
    Project, Request,
    remote::{Change, Entry, Operation, RemoteProject, Reply},
    ssh::{ConfigHost, HostKey, HostKeyScan, SshError, SshFailure, SshOptions},
};

pub(crate) enum FormResult {
    Probe(String),
    Added(Box<terminator_core::State>),
}
/// A remote operation's error text plus its SSH classification, when the
/// daemon supplied one (`ssh-hostkeys-v1`).
#[derive(Clone, Debug)]
pub(crate) struct Failure {
    pub text: String,
    pub ssh: Option<SshFailure>,
}
impl From<String> for Failure {
    fn from(text: String) -> Self {
        Self { text, ssh: None }
    }
}
impl Failure {
    pub(crate) fn from_error(error: &(dyn std::error::Error + 'static)) -> Self {
        let ssh = SshError::find(error).map(|e| e.failure.clone());
        let text = match &ssh {
            Some(SshFailure::AuthFailed) => {
                "SSH sign-in required. Open an SSH terminal to sign in; files and Git then reuse its connection.".into()
            }
            Some(SshFailure::UnknownHostKey) => "This SSH host's key is not trusted yet.".into(),
            Some(SshFailure::ChangedHostKey) => {
                "The SSH host key changed. Nothing was trusted automatically.".into()
            }
            _ => terminator_core::ErrorChain(error).to_string(),
        };
        Self { text, ssh }
    }
}
/// What to run again after the user trusts a host key.
#[derive(Clone)]
pub(crate) enum Retry {
    Form {
        project: Box<RemoteProject>,
        probe: bool,
        generation: u64,
    },
    Project(String),
    /// Hosts sidebar quick connect: probe the login directory again.
    Quick(Box<RemoteProject>),
}
pub(crate) struct HostKeyPrompt {
    pub ssh: SshOptions,
    pub retry: Retry,
    pub changed: bool,
    pub scan: Option<HostKeyScan>,
    pub error: Option<String>,
    pub busy: bool,
}
#[derive(Default)]
pub(crate) struct RemoteUi {
    pub active: Option<RemoteProject>,
    pub open: bool,
    pub draft: Option<RemoteProject>,
    pub error: Option<String>,
    pub busy: bool,
    pub views: HashMap<String, View>,
    pub host_key: Option<HostKeyPrompt>,
    /// Projects whose host-key prompt was dismissed; polling must not
    /// reopen it until the user refreshes.
    pub host_key_declined: HashSet<String>,
    /// `~/.ssh/config` aliases for the open dialog; `None` while loading.
    pub config_hosts: Option<Vec<ConfigHost>>,
    /// Identifies the dialog opening that `config_hosts` belongs to.
    pub config_generation: u64,
    /// Allocates config-load generations for the dialog and Hosts sidebar.
    pub config_seq: u64,
    pub hosts: crate::hosts_ui::HostsState,
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
    /// A batch call failed authentication: a terminal must sign in first.
    pub sign_in: bool,
    pub files_loading: bool,
    pub git_loading: bool,
    pub files_polled: Option<Instant>,
    pub git_polled: Option<Instant>,
    /// Uploads and downloads (`ssh-transfer-v1`).
    pub transfers: crate::transfers_ui::Transfers,
}
impl RemoteUi {
    /// Returns the generation to load SSH config hosts for.
    pub(crate) fn next_config_generation(&mut self) -> u64 {
        self.config_seq = self.config_seq.wrapping_add(1).max(1);
        self.config_seq
    }
    pub(crate) fn open_new(&mut self) -> u64 {
        self.config_generation = self.next_config_generation();
        self.config_hosts = None;
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
            group: None,
            forwards: Vec::new(),
        });
        self.config_generation
    }
}
fn config_host_summary(host: &ConfigHost) -> String {
    let mut text = String::new();
    if !host.user.is_empty() {
        text.push_str(&host.user);
        text.push('@');
    }
    text.push_str(&host.hostname);
    if let Some(port) = host.port {
        text.push_str(&format!(":{port}"));
    }
    if let Some(jump) = &host.proxyjump {
        text.push_str(&format!(" via {jump}"));
    }
    text
}
/// "From SSH config" picker. Choosing an alias leaves user and port blank so
/// OpenSSH applies the configured `User`, `Port`, `ProxyJump` and keys.
fn config_host_picker(ui: &mut egui::Ui, hosts: Option<&[ConfigHost]>, draft: &mut RemoteProject) {
    let selected = hosts.and_then(|h| h.iter().find(|h| h.alias == draft.ssh.host));
    let text = match (hosts, selected) {
        (None, _) => "Loading…".to_owned(),
        (Some([]), _) => "No hosts in ~/.ssh/config".to_owned(),
        (Some(_), Some(host)) => host.alias.clone(),
        (Some(_), None) => "Choose a host…".to_owned(),
    };
    let mut chosen = None;
    ui.horizontal(|ui| {
        ui.label("From SSH config");
        ui.add_enabled_ui(hosts.is_some_and(|h| !h.is_empty()), |ui| {
            egui::ComboBox::from_id_salt("ssh-config-host")
                .selected_text(text)
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    for host in hosts.unwrap_or_default() {
                        let label = format!("{}  ·  {}", host.alias, config_host_summary(host));
                        if ui
                            .selectable_label(host.alias == draft.ssh.host, label)
                            .clicked()
                        {
                            chosen = Some(host.alias.clone());
                        }
                    }
                });
        });
    });
    if let Some(alias) = chosen {
        if draft.name.trim().is_empty() || draft.name == draft.ssh.host {
            draft.name.clone_from(&alias);
        }
        draft.ssh.host = alias;
        draft.ssh.username.clear();
        draft.ssh.port = None;
    }
}
pub(crate) fn location_label(session: &terminator_core::Session) -> String {
    session.remote.as_ref().map_or_else(
        || session.cwd.display().to_string(),
        |r| format!("ssh://{}{}", r.ssh.authority(), r.cwd),
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
            ui.weak("Each terminal connects to this host. Files and Git stay remote and reuse a signed-in terminal's connection. Passwords and 2FA are entered in the SSH terminal. Text editing uses Neovim on the SSH host.");
            if !supported{ui.colored_label(ui.visuals().warn_fg_color,"Restart the session service to enable SSH projects.");}
            if let Some(error)=&self.remote_ui.error{ui.colored_label(ui.visuals().error_fg_color,error);}
            let hosts=self.remote_ui.config_hosts.as_deref();
            let Some(draft)=&mut self.remote_ui.draft else{return;};
            config_host_picker(ui,hosts,draft);
            let config=hosts.and_then(|h|h.iter().find(|h|h.alias==draft.ssh.host)).cloned();
            let user_hint=config.as_ref().filter(|h|!h.user.is_empty()).map_or_else(||"From SSH config or local user".to_owned(),|h|h.user.clone());
            for (label,value,hint) in [("Name",&mut draft.name,""),("SSH host or config alias",&mut draft.ssh.host,""),("SSH user (optional)",&mut draft.ssh.username,user_hint.as_str()),("Remote project path",&mut draft.root,"")]{
                ui.label(label);ui.add(appearance::singleline(value).hint_text(hint).desired_width(f32::INFINITY));
            }
            let port_hint=config.as_ref().and_then(|h|h.port).map_or_else(||"config".to_owned(),|p|p.to_string());
            ui.horizontal(|ui|{ui.label("SSH port (blank uses SSH config)");appearance::optional_port(ui,&mut draft.ssh.port,&port_hint);});
            if let Some(host)=&config{ui.weak(format!("SSH config: {}",config_host_summary(host)));}
            for (label,value) in [("Identity file (optional)",&mut draft.ssh.identity_file),("Known hosts file (optional)",&mut draft.ssh.known_hosts_file)]{
                ui.label(label);let mut text=value.as_ref().map_or_else(String::new,|p|p.to_string_lossy().into_owned());
                if ui.add(appearance::singleline(&mut text).desired_width(f32::INFINITY)).changed(){*value=(!text.trim().is_empty()).then(||PathBuf::from(text.trim()));}
            }
            ui.weak("Unknown host keys are shown for confirmation before they are trusted; changed keys are refused. The project is not mounted or copied onto this computer.");
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
            ui.separator();
            self.forwards_section(ui, &project);
            return;
        }
        if matches!(tool, SidebarTool::Problems | SidebarTool::Structure) {
            ui.label("Use tools in the SSH terminal for remote diagnostics and symbols.");
            return;
        }
        let git = tool == SidebarTool::Git;
        let transfers = !git && self.transfers_supported();
        let failed = appearance::color(&self.theme.status_failed);
        let mut picks = Vec::new();
        let mut polls = Vec::new();
        let mut sign_in = false;
        let mut request = None;
        let mut open = None;
        let mut diff = None;
        {
            let mut upload = false;
            let refresh = ui
                .horizontal(|ui| {
                    let refresh =
                        appearance::sidebar_action(ui, "RefreshCw", "Refresh remote project")
                            .clicked();
                    if transfers {
                        let response = appearance::sidebar_action(
                            ui,
                            "Upload",
                            "Upload a file into this folder…",
                        );
                        #[cfg(feature = "test-support")]
                        crate::diagnostics::record(
                            ui.ctx(),
                            &format!("remote-upload:{}", project.id),
                            response.rect,
                        );
                        upload = response.clicked();
                    }
                    refresh
                })
                .inner;
            if refresh {
                self.remote_ui.host_key_declined.remove(&project.id);
            }
            let view = self.remote_ui.views.entry(project.id.clone()).or_default();
            if view.sign_in {
                ui.horizontal(|ui| {
                    if ui.button("Open SSH terminal").clicked() {
                        sign_in = true;
                    }
                    ui.weak("Sign in there; this view retries afterwards.");
                });
            }
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
                if upload {
                    picks.push(crate::transfers_ui::Pick::Upload {
                        remote_dir: view.path.clone(),
                    });
                }
                polls = view.transfers.panel(ui, failed);
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
                            response.context_menu(|ui| {
                                if entry.directory {
                                    if ui
                                        .add_enabled(
                                            transfers,
                                            egui::Button::new("Upload into this folder…"),
                                        )
                                        .clicked()
                                    {
                                        picks.push(crate::transfers_ui::Pick::Upload {
                                            remote_dir: entry.path.clone(),
                                        });
                                        ui.close();
                                    }
                                    return;
                                }
                                if ui.button("Open in SSH editor").clicked() {
                                    open = Some((entry.path.clone(), true));
                                    ui.close();
                                }
                                if ui
                                    .add_enabled(transfers, egui::Button::new("Download…"))
                                    .clicked()
                                {
                                    picks.push(crate::transfers_ui::Pick::Download {
                                        path: entry.path.clone(),
                                        name: entry.name.clone(),
                                    });
                                    ui.close();
                                }
                            });
                        }
                    });
            }
        }
        if !git
            && let Some(path) = self
                .remote_ui
                .views
                .get(&project.id)
                .map(|v| v.path.clone())
        {
            self.drop_uploads(ui, &project.id, &path);
        }
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
                project: &project,
                path,
                line: None,
                split: None,
                text,
            });
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

    pub(crate) fn open_remote_file(&mut self, input: OpenRemoteFileInput<'_>) {
        let OpenRemoteFileInput {
            project,
            path,
            line,
            split,
            text,
        } = input;
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
        result: Result<Reply, Failure>,
    ) {
        let Some(ssh) = self
            .state
            .remote_projects
            .iter()
            .find(|p| p.id == project)
            .map(|p| p.ssh.clone())
        else {
            return;
        };
        if let Err(failure) = &result {
            self.prompt_for_failure(&project, ssh, failure);
        }
        let view = self.remote_ui.views.entry(project).or_default();
        match &result {
            Ok(_) => view.sign_in = false,
            Err(failure) => view.sign_in = failure.ssh == Some(SshFailure::AuthFailed),
        }
        let result = result.map_err(|failure| failure.text);
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
impl App {
    /// Offers the host-key dialog for a host-key failure in project `id`.
    fn prompt_for_failure(&mut self, id: &str, ssh: SshOptions, failure: &Failure) {
        if matches!(
            failure.ssh,
            Some(SshFailure::UnknownHostKey | SshFailure::ChangedHostKey)
        ) && !self.remote_ui.host_key_declined.contains(id)
        {
            self.prompt_host_key(
                ssh,
                Retry::Project(id.to_owned()),
                failure.ssh == Some(SshFailure::ChangedHostKey),
            );
        }
    }
    /// A transfer's classified SSH failure: offer the host-key dialog or ask
    /// for a terminal sign-in.
    pub(crate) fn remote_ssh_failure(&mut self, id: &str, failure: &Failure) {
        let Some(ssh) = self
            .state
            .remote_projects
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.ssh.clone())
        else {
            return;
        };
        self.prompt_for_failure(id, ssh, failure);
        if failure.ssh == Some(SshFailure::AuthFailed) {
            self.remote_ui
                .views
                .entry(id.to_owned())
                .or_default()
                .sign_in = true;
        }
    }
    fn host_keys_supported(&self) -> bool {
        self.connected
            && self
                .state
                .capabilities
                .iter()
                .any(|c| c == terminator_core::ssh::HOST_KEYS_CAPABILITY)
    }
    /// Opens the trust dialog and scans the host's keys off the GUI thread.
    /// One prompt at a time; repeated poll failures do not stack.
    pub(crate) fn prompt_host_key(&mut self, ssh: SshOptions, retry: Retry, changed: bool) {
        if !self.host_keys_supported() || self.remote_ui.host_key.is_some() {
            return;
        }
        self.remote_ui.host_key = Some(HostKeyPrompt {
            ssh: ssh.clone(),
            retry,
            changed,
            scan: None,
            error: None,
            busy: true,
        });
        let _ = self.jobs.send(Job::HostKey { ssh, keys: None });
    }
    pub(crate) fn apply_host_key_scan(
        &mut self,
        ssh: &SshOptions,
        result: Result<HostKeyScan, String>,
    ) {
        let Some(prompt) = self.remote_ui.host_key.as_mut().filter(|p| p.ssh == *ssh) else {
            return;
        };
        prompt.busy = false;
        match result {
            Ok(scan) => {
                prompt.scan = Some(scan);
                prompt.error = None;
            }
            Err(error) => prompt.error = Some(error),
        }
    }
    pub(crate) fn apply_host_key_trust(&mut self, ssh: &SshOptions, result: Result<(), String>) {
        if self
            .remote_ui
            .host_key
            .as_ref()
            .is_none_or(|p| p.ssh != *ssh)
        {
            return;
        }
        if let Err(error) = result {
            if let Some(prompt) = &mut self.remote_ui.host_key {
                prompt.busy = false;
                prompt.error = Some(error);
            }
            return;
        }
        let Some(prompt) = self.remote_ui.host_key.take() else {
            return;
        };
        self.info = Some("SSH host key trusted.".into());
        match prompt.retry {
            Retry::Form {
                project,
                probe,
                generation,
            } => {
                if !self.remote_ui.open || generation != self.selection_generation {
                    return;
                }
                self.remote_ui.busy = true;
                self.remote_ui.error = None;
                self.remote_ui.active = Some((*project).clone());
                let _ = self.jobs.send(Job::RemoteAdd {
                    project: *project,
                    probe,
                    generation,
                });
            }
            Retry::Quick(project) => self.start_quick_connect(*project),
            Retry::Project(id) => {
                self.remote_ui.host_key_declined.remove(&id);
                if let Some(view) = self.remote_ui.views.get_mut(&id) {
                    view.files_loading = false;
                    view.pending_path = None;
                    view.files_polled = None;
                    view.git_loading = false;
                    view.git_polled = None;
                    view.file_error = None;
                    view.git_error = None;
                }
            }
        }
    }
    fn dismiss_host_key(&mut self) {
        if let Some(prompt) = self.remote_ui.host_key.take()
            && let Retry::Project(id) = prompt.retry
        {
            self.remote_ui.host_key_declined.insert(id);
        }
    }
    /// First-connect trust confirmation. A changed key is only explained;
    /// removing the old key stays a manual, explicit user action.
    pub(crate) fn host_key_dialog(&mut self, ctx: &egui::Context) {
        let Some(prompt) = &self.remote_ui.host_key else {
            return;
        };
        let failed = appearance::color(&self.theme.status_failed);
        let host = prompt
            .scan
            .as_ref()
            .map_or_else(|| prompt.ssh.host.clone(), |scan| scan.host.clone());
        let changed = prompt.changed;
        let busy = prompt.busy;
        let error = prompt.error.clone();
        let keys: Vec<HostKey> = prompt
            .scan
            .as_ref()
            .map(|s| s.keys.clone())
            .unwrap_or_default();
        let remove = format!(
            "ssh-keygen -R {}{}",
            terminator_core::quote(&host),
            prompt
                .ssh
                .known_hosts_file
                .as_ref()
                .map(|path| format!(" -f {}", terminator_core::quote(&path.to_string_lossy())))
                .unwrap_or_default()
        );
        let mut open = true;
        let mut trust = false;
        let mut cancel = false;
        self.popups
            .window(
                ctx,
                if changed {
                    "SSH host key changed"
                } else {
                    "Trust SSH host?"
                },
            )
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                if changed {
                    ui.label(
                        egui::RichText::new(format!(
                            "The host key for {host} does not match the one in known_hosts."
                        ))
                        .strong()
                        .color(failed),
                    );
                    ui.label(
                        egui::RichText::new(
                            "Someone may be intercepting the connection, or the host was reinstalled. Terminator will not replace the key.",
                        )
                        .color(failed),
                    );
                } else {
                    ui.label(format!("First connection to {host}."));
                    ui.weak("Compare these fingerprints with ones from the host's administrator before trusting it.");
                }
                ui.add_space(4.0);
                if busy && keys.is_empty() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Reading host keys…");
                    });
                }
                if !keys.is_empty() {
                    if changed {
                        ui.label("The host now presents:");
                    }
                    for key in &keys {
                        ui.label(
                            egui::RichText::new(format!("{}  {}", key.key_type, key.fingerprint))
                                .monospace(),
                        );
                    }
                }
                if changed {
                    ui.add_space(4.0);
                    ui.label("If you trust the change, remove the old key in a terminal, then reconnect:");
                    ui.label(egui::RichText::new(&remove).monospace());
                }
                if let Some(error) = &error {
                    ui.colored_label(failed, error);
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if changed {
                        cancel = ui.button("Close").clicked();
                    } else {
                        let response =
                            ui.add_enabled(!busy && !keys.is_empty(), egui::Button::new("Trust and connect"));
                        // Only once clickable, so a fixture click waits for the scan.
                        #[cfg(feature = "test-support")]
                        if !busy && !keys.is_empty() {
                            crate::diagnostics::record(ui.ctx(), "ssh-hostkey-trust", response.rect);
                        }
                        trust = response.clicked();
                        cancel = ui.button("Cancel").clicked();
                    }
                });
            });
        if trust && let Some(prompt) = &mut self.remote_ui.host_key {
            prompt.busy = true;
            prompt.error = None;
            let _ = self.jobs.send(Job::HostKey {
                ssh: prompt.ssh.clone(),
                keys: Some(keys),
            });
        } else if cancel || !open {
            self.dismiss_host_key();
        }
    }
}
