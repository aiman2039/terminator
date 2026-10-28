//! Hosts sidebar: saved SSH projects grouped by folder and host, live
//! connection dots, and quick connect for `~/.ssh/config` aliases.
mod sidebar;
#[cfg(test)]
mod tests;

use crate::{App, Job};
use eframe::egui;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
    time::{Duration, Instant},
};
use terminator_core::{
    remote::{HOME_ROOT, RemoteProject},
    ssh::{ConfigHost, SshOptions},
};

/// How often visible host dots re-run `ssh -O check`.
pub(crate) const STATUS_INTERVAL: Duration = Duration::from_secs(10);
/// Bound on one `ssh -O check`; a master that does not answer is down.
const CHECK_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Default)]
pub(crate) struct HostsState {
    /// `~/.ssh/config` aliases; `None` until the first load finishes.
    pub config_hosts: Option<Vec<ConfigHost>>,
    /// The config load this sidebar waits for; 0 before the first request.
    pub config_generation: u64,
    /// Project id to whether its control master answered.
    pub status: HashMap<String, bool>,
    pub status_pending: bool,
    pub status_polled: Option<Instant>,
    pub confirm_remove: Option<String>,
    /// Project id and the group name being typed.
    pub editing_group: Option<(String, String)>,
    /// Quick connect in flight: probing the login directory, then adding.
    pub quick: Option<RemoteProject>,
    pub error: Option<String>,
    /// Daemon forward status (`ssh-forwarding-v1`).
    pub forwards: terminator_core::remote::ForwardStatus,
    pub forwards_pending: bool,
    pub forwards_polled: Option<Instant>,
    /// (project, local port) start or stop requests in flight.
    pub forward_busy: HashSet<(String, u16)>,
    /// Last forward error per project.
    pub forward_errors: HashMap<String, String>,
    /// Projects whose forwards are expanded in the Hosts sidebar.
    pub forwards_open: HashSet<String>,
    pub forward_draft: Option<crate::forwards_ui::ForwardDraft>,
}

pub(crate) struct HostGroup<'a> {
    /// `None` collects projects without a group.
    pub name: Option<&'a str>,
    /// Host authority (`user@host:port`) and its projects, sorted by name.
    pub hosts: Vec<(String, Vec<&'a RemoteProject>)>,
}

fn matches(project: &RemoteProject, filter: &str) -> bool {
    filter.is_empty()
        || [
            project.name.as_str(),
            project.ssh.host.as_str(),
            project.ssh.username.as_str(),
            project.root.as_str(),
            project.group.as_deref().unwrap_or_default(),
        ]
        .iter()
        .any(|field| field.to_lowercase().contains(filter))
}

/// Named groups alphabetically, then ungrouped projects; hosts and projects
/// sorted within each. `filter` matches name, host, user, root or group.
pub(crate) fn group_hosts<'a>(projects: &'a [RemoteProject], filter: &str) -> Vec<HostGroup<'a>> {
    let filter = filter.trim().to_lowercase();
    type Hosts<'a> = BTreeMap<(String, String), Vec<&'a RemoteProject>>;
    let mut groups: BTreeMap<(bool, String, &'a str), Hosts<'a>> = BTreeMap::new();
    for project in projects.iter().filter(|p| matches(p, &filter)) {
        let name = project.group.as_deref().unwrap_or_default();
        let authority = project.ssh.authority();
        groups
            .entry((project.group.is_none(), name.to_lowercase(), name))
            .or_default()
            .entry((authority.to_lowercase(), authority))
            .or_default()
            .push(project);
    }
    groups
        .into_iter()
        .map(|((ungrouped, _, name), hosts)| HostGroup {
            name: (!ungrouped).then_some(name),
            hosts: hosts
                .into_iter()
                .map(|((_, authority), mut projects)| {
                    projects.sort_by_cached_key(|p| (p.name.to_lowercase(), p.id.clone()));
                    (authority, projects)
                })
                .collect(),
        })
        .collect()
}

/// Config aliases no saved project connects to by name, filtered like
/// [`group_hosts`].
pub(crate) fn quick_aliases<'a>(
    hosts: &'a [ConfigHost],
    projects: &[RemoteProject],
    filter: &str,
) -> Vec<&'a ConfigHost> {
    let filter = filter.trim().to_lowercase();
    hosts
        .iter()
        .filter(|h| !projects.iter().any(|p| p.ssh.host == h.alias))
        .filter(|h| {
            filter.is_empty()
                || h.alias.to_lowercase().contains(&filter)
                || h.hostname.to_lowercase().contains(&filter)
        })
        .collect()
}

/// Runs `ssh -O check` for each socket, sharing one check per socket.
/// A missing socket is down without spawning anything.
pub(crate) async fn check_controls(targets: Vec<(String, PathBuf)>) -> Vec<(String, bool)> {
    let mut checked: HashMap<PathBuf, bool> = HashMap::new();
    let mut results = Vec::with_capacity(targets.len());
    for (id, control) in targets {
        let up = if let Some(up) = checked.get(&control) {
            *up
        } else {
            let up = check_control(&control).await;
            checked.insert(control, up);
            up
        };
        results.push((id, up));
    }
    results
}

async fn check_control(control: &std::path::Path) -> bool {
    if !control.exists() {
        return false;
    }
    let mut command = tokio::process::Command::from(terminator_core::ssh::check_control(control));
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let Ok(mut child) = command.spawn() else {
        return false;
    };
    matches!(
        tokio::time::timeout(CHECK_TIMEOUT, child.wait()).await,
        Ok(Ok(status)) if status.success()
    )
}

impl App {
    fn hosts_supported(&self) -> bool {
        self.connected
            && self
                .state
                .capabilities
                .iter()
                .any(|c| c == terminator_core::remote::HOSTS_CAPABILITY)
    }

    fn load_config_hosts(&mut self) {
        let generation = self.remote_ui.next_config_generation();
        self.remote_ui.hosts.config_generation = generation;
        let _ = self.jobs.send(Job::SshConfigHosts(generation));
    }

    /// Polls control masters while this sidebar is drawn; hidden sidebars
    /// never poll.
    fn poll_host_status(&mut self, ctx: &egui::Context) {
        let hosts = &mut self.remote_ui.hosts;
        ctx.request_repaint_after(STATUS_INTERVAL);
        if hosts.status_pending
            || hosts
                .status_polled
                .is_some_and(|at| at.elapsed() < STATUS_INTERVAL)
        {
            return;
        }
        let targets: Vec<_> = self
            .state
            .remote_projects
            .iter()
            .filter_map(|p| Some((p.id.clone(), self.paths.ssh_control(&p.ssh)?)))
            .collect();
        hosts.status_polled = Some(Instant::now());
        if targets.is_empty() {
            hosts.status.clear();
            return;
        }
        hosts.status_pending = true;
        let _ = self.jobs.send(Job::SshControlStatus(targets));
    }

    pub(crate) fn apply_host_status(&mut self, results: Vec<(String, bool)>) {
        let hosts = &mut self.remote_ui.hosts;
        hosts.status_pending = false;
        if !results.is_empty() {
            hosts.status = results.into_iter().collect();
        }
    }

    pub(crate) fn open_host_terminal(&mut self, id: String) {
        self.select_project(id);
        self.create(None);
    }

    /// Probes the alias's login directory, then adds a project rooted there
    /// and opens a terminal in it.
    pub(crate) fn quick_connect(&mut self, alias: &str) {
        let project = RemoteProject {
            id: String::new(),
            name: alias.to_owned(),
            ssh: SshOptions {
                host: alias.to_owned(),
                ..SshOptions::default()
            },
            root: HOME_ROOT.to_owned(),
            layout: serde_json::Value::Null,
            group: None,
            forwards: Vec::new(),
        };
        self.start_quick_connect(project);
    }

    pub(crate) fn start_quick_connect(&mut self, project: RemoteProject) {
        self.remote_ui.hosts.error = None;
        self.remote_ui.hosts.quick = Some(project.clone());
        let _ = self.jobs.send(Job::RemoteAdd {
            project,
            probe: true,
            generation: self.selection_generation,
        });
    }

    /// Advances the quick connect in flight for `project`.
    pub(crate) fn apply_quick_connect(
        &mut self,
        project: &RemoteProject,
        generation: u64,
        result: Result<crate::remote_ui::FormResult, crate::remote_ui::Failure>,
    ) {
        if self.remote_ui.hosts.quick.as_ref() != Some(project) {
            return;
        }
        self.remote_ui.hosts.quick = None;
        match result {
            Ok(crate::remote_ui::FormResult::Probe(root)) => {
                let mut added = project.clone();
                added.root = root;
                self.remote_ui.hosts.quick = Some(added.clone());
                let _ = self.jobs.send(Job::RemoteAdd {
                    project: added,
                    probe: false,
                    generation,
                });
            }
            Ok(crate::remote_ui::FormResult::Added(state)) => {
                let id = state
                    .remote_projects
                    .iter()
                    .find(|p| p.ssh == project.ssh && p.root == project.root)
                    .map(|p| p.id.clone());
                self.apply_state(*state);
                if let Some(id) = id {
                    self.open_host_terminal(id);
                }
            }
            Err(failure) => {
                if let Some(
                    ssh @ (terminator_core::ssh::SshFailure::UnknownHostKey
                    | terminator_core::ssh::SshFailure::ChangedHostKey),
                ) = &failure.ssh
                {
                    let changed = *ssh == terminator_core::ssh::SshFailure::ChangedHostKey;
                    self.prompt_host_key(
                        project.ssh.clone(),
                        crate::remote_ui::Retry::Quick(Box::new(project.clone())),
                        changed,
                    );
                }
                self.remote_ui.hosts.error = Some(failure.text);
            }
        }
    }
}
