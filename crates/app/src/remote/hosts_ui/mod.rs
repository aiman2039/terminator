//! SSH hosts: the Hosts sidebar lists saved SSH projects in collapsible
//! groups with live connection dots; the SSH page in the main pane (like
//! the VNC page) finds, adds, edits and imports hosts.
pub(crate) mod import;
mod page;
mod rows;
mod sidebar;
#[cfg(test)]
mod tests;

use crate::{App, Job};
use eframe::egui;
use std::{
    cmp::Reverse,
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
    time::{Duration, Instant},
};
use terminator_core::{
    remote::RemoteProject,
    ssh::{ConfigHost, SshOptions},
};

/// How often visible host dots re-run `ssh -O check`.
pub(crate) const STATUS_INTERVAL: Duration = Duration::from_secs(10);
/// Bound on one `ssh -O check`; a master that does not answer is down.
const CHECK_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Default)]
pub(crate) struct HostsState {
    /// The SSH page fills the main pane.
    pub page_open: bool,
    /// The page's address bar: filters cards, or names a new host.
    pub address: String,
    pub page_error: Option<String>,
    /// The card the keyboard acts on.
    pub selected_card: Option<String>,
    /// Show only this group's cards.
    pub group_filter: Option<String>,
    /// Rows instead of cards.
    pub list_view: bool,
    /// `~/.ssh/config` aliases for the import page; `None` until loaded.
    pub config_hosts: Option<Vec<ConfigHost>>,
    /// The config load this sidebar waits for; 0 before the first request.
    pub config_generation: u64,
    /// Project id to whether its control master answered.
    pub status: HashMap<String, bool>,
    pub status_pending: bool,
    pub status_polled: Option<Instant>,
    pub confirm_remove: Option<String>,
    /// Keyboard selection (a project id) while the filter field has focus.
    pub cursor: Option<String>,
    /// Project id and the group name being typed.
    pub editing_group: Option<(String, String)>,
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
    /// Sorted by name, or most recently connected first.
    pub projects: Vec<&'a RemoteProject>,
}

impl HostGroup<'_> {
    /// Key for [`Preferences::hosts_collapsed`](crate::preferences::Preferences).
    pub(crate) fn key(&self) -> &str {
        self.name.unwrap_or_default()
    }
}

pub(super) fn matches(project: &RemoteProject, filter: &str) -> bool {
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

/// Named groups alphabetically, then ungrouped projects. Within a group,
/// projects sort by name or, with `recent_first`, by last connection.
/// `filter` matches name, host, user, root or group.
pub(crate) fn group_hosts<'a>(
    projects: &'a [RemoteProject],
    filter: &str,
    recent_first: bool,
) -> Vec<HostGroup<'a>> {
    let filter = filter.trim().to_lowercase();
    let mut groups: BTreeMap<(bool, String, &'a str), Vec<&'a RemoteProject>> = BTreeMap::new();
    for project in projects.iter().filter(|p| matches(p, &filter)) {
        let name = project.group.as_deref().unwrap_or_default();
        groups
            .entry((project.group.is_none(), name.to_lowercase(), name))
            .or_default()
            .push(project);
    }
    groups
        .into_iter()
        .map(|((ungrouped, _, name), mut projects)| {
            projects.sort_by_cached_key(|p| {
                let recent = if recent_first { p.last_connected } else { 0 };
                (Reverse(recent), p.name.to_lowercase(), p.id.clone())
            });
            HostGroup {
                name: (!ungrouped).then_some(name),
                projects,
            }
        })
        .collect()
}

/// `host`, `user@host`, `host:port` or `user@host:port` typed into the
/// filter, as SSH options. `None` unless the options validate.
pub(crate) fn parse_target(text: &str) -> Option<SshOptions> {
    let text = text.trim();
    let (user, rest) = text.split_once('@').unwrap_or(("", text));
    let (host, port) = match rest.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => {
            (host, Some(port.parse::<u16>().ok().filter(|p| *p > 0)?))
        }
        _ => (rest, None),
    };
    let options = SshOptions {
        host: host.to_owned(),
        username: user.to_owned(),
        port,
        ..SshOptions::default()
    };
    options.validate().ok().map(|()| options)
}

/// Config aliases no saved project connects to by name, filtered like
/// [`group_hosts`].
pub(crate) fn unsaved_aliases<'a>(
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
}
