//! Remote project controls and data. Local refresh/watch services are never used.
mod fields;
mod host_key;
mod properties;
mod sidebar;
pub(crate) use fields::{blank_project, config_host_summary};

use crate::{App, Job, appearance};
use eframe::egui;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    time::Instant,
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
    ssh::{ConfigHost, HostKeyScan, SshError, SshFailure, SshOptions},
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
    /// The host in SSH Properties. A non-empty id edits that saved project.
    pub draft: Option<RemoteProject>,
    pub error: Option<String>,
    /// The classified SSH failure behind `error`, for its fix button.
    pub failure: Option<SshFailure>,
    pub busy: bool,
    /// Open a terminal once the host is added or saved.
    pub connect_after: bool,
    /// `ssh -G` for the typed host, so the page shows what the config sets.
    pub probe: crate::vnc_ui::ssh_probe::SshProbe,
    pub resolved: Option<crate::vnc_ui::ssh_probe::SshHostConfig>,
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
    /// The import page, while open.
    pub import: Option<crate::hosts_ui::import::HostImport>,
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
    /// Opens SSH Properties on `draft`; returns the config-load generation.
    pub(crate) fn open_draft(&mut self, draft: RemoteProject) -> u64 {
        self.config_generation = self.next_config_generation();
        self.config_hosts = None;
        self.open = true;
        self.active = None;
        self.error = None;
        self.failure = None;
        self.busy = false;
        self.connect_after = draft.id.is_empty();
        self.resolved = None;
        self.draft = Some(draft);
        self.config_generation
    }
    pub(crate) fn close(&mut self) {
        self.open = false;
        self.active = None;
        self.busy = false;
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
