//! Hosts sidebar: saved SSH projects grouped by folder and host, live
//! connection dots, and quick connect for `~/.ssh/config` aliases.
use crate::{App, Job, appearance};
use eframe::egui::{self, RichText};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
    time::{Duration, Instant},
};
use terminator_core::{
    Request,
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

    pub(crate) fn hosts_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Hosts");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if appearance::sidebar_action(ui, "Plus", "New SSH project").clicked() {
                    let generation = self.remote_ui.open_new();
                    let _ = self.jobs.send(Job::SshConfigHosts(generation));
                }
                if appearance::sidebar_action(ui, "RefreshCw", "Reload ~/.ssh/config").clicked() {
                    self.load_config_hosts();
                    self.remote_ui.hosts.status_polled = None;
                }
            });
        });
        ui.add_space(4.0);
        if !self.remote_supported() {
            ui.colored_label(
                appearance::color(&self.theme.status_waiting),
                "Restart the session service to enable SSH projects.",
            );
            self.restart_session_button(ui, true);
            return;
        }
        let managed = self.hosts_supported();
        if self.remote_ui.hosts.config_generation == 0 {
            self.load_config_hosts();
        }
        self.poll_host_status(ui.ctx());
        self.poll_forwards(ui.ctx());

        ui.horizontal(|ui| {
            ui.add(
                appearance::singleline(&mut self.preferences.hosts_filter)
                    .hint_text("Filter hosts…")
                    .desired_width((ui.available_width() - 28.0).max(60.0)),
            );
            if !self.preferences.hosts_filter.is_empty()
                && appearance::sidebar_action(ui, "X", "Clear filter").clicked()
            {
                self.preferences.hosts_filter.clear();
            }
        });
        if let Some(error) = self.remote_ui.hosts.error.clone() {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(appearance::color(&self.theme.status_failed), error);
                if appearance::sidebar_action(ui, "X", "Dismiss").clicked() {
                    self.remote_ui.hosts.error = None;
                }
            });
        }
        if !managed {
            ui.weak("Restart the session service to group, remove and quick-connect hosts.");
        }
        ui.add_space(6.0);

        let projects = self.state.remote_projects.clone();
        let filter = self.preferences.hosts_filter.clone();
        let groups = group_hosts(&projects, &filter);
        let any_group = groups.iter().any(|g| g.name.is_some());
        let config = self.remote_ui.hosts.config_hosts.clone();
        let quick_busy = self.remote_ui.hosts.quick.is_some();

        appearance::sidebar_scroll("hosts-sidebar").show(ui, |ui| {
            if projects.is_empty() {
                ui.weak("No saved SSH projects. Click + or quick-connect a config host below.");
            } else if groups.is_empty() {
                ui.weak("No matching hosts.");
            }
            for group in &groups {
                if any_group {
                    ui.label(RichText::new(group.name.unwrap_or("Ungrouped")).strong());
                }
                for (authority, members) in &group.hosts {
                    ui.weak(authority);
                    for project in members {
                        self.host_row(ui, project, managed);
                    }
                }
                ui.add_space(6.0);
            }

            ui.separator();
            ui.weak("Quick connect · ~/.ssh/config");
            let Some(config) = config else {
                ui.weak("Loading…");
                return;
            };
            let aliases = quick_aliases(&config, &projects, &filter);
            if aliases.is_empty() {
                ui.weak("No other config hosts.");
            }
            for host in aliases {
                ui.horizontal(|ui| {
                    ui.label(&host.alias).on_hover_text(&host.hostname);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let connecting = self
                            .remote_ui
                            .hosts
                            .quick
                            .as_ref()
                            .is_some_and(|q| q.ssh.host == host.alias);
                        if connecting {
                            ui.add(egui::Spinner::new());
                            return;
                        }
                        let response = ui
                            .add_enabled_ui(managed && !quick_busy, |ui| {
                                appearance::sidebar_action(
                                    ui,
                                    "Play",
                                    "Quick connect: add the home directory and open a terminal",
                                )
                            })
                            .inner;
                        if response.clicked() {
                            self.quick_connect(&host.alias);
                        }
                    });
                });
            }
        });
    }

    fn host_row(&mut self, ui: &mut egui::Ui, project: &RemoteProject, managed: bool) {
        let forwarding = self.forwarding_supported();
        let up = self.remote_ui.hosts.status.get(&project.id).copied();
        ui.horizontal(|ui| {
            let (color, tip) = match up {
                Some(true) => (
                    appearance::color(&self.theme.status_running),
                    "Connected: files, Git and new terminals reuse this connection",
                ),
                Some(false) => (ui.visuals().weak_text_color(), "Not connected"),
                None => (ui.visuals().weak_text_color(), "Checking connection…"),
            };
            ui.colored_label(color, "●").on_hover_text(tip);
            let selected = self.selected.as_deref() == Some(project.id.as_str());
            if ui
                .selectable_label(selected, &project.name)
                .on_hover_text(project.label())
                .clicked()
            {
                self.reveal_project(project.id.clone());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_enabled_ui(managed, |ui| {
                    if appearance::sidebar_action(ui, "Trash", "Remove SSH project").clicked() {
                        self.remote_ui.hosts.confirm_remove = Some(project.id.clone());
                    }
                    if appearance::sidebar_action(ui, "Pencil", "Set group").clicked() {
                        self.remote_ui.hosts.editing_group = Some((
                            project.id.clone(),
                            project.group.clone().unwrap_or_default(),
                        ));
                    }
                });
                if forwarding {
                    let open = self.remote_ui.hosts.forwards_open.contains(&project.id);
                    if appearance::selectable_icon(ui, "Globe", "Port forwards", open).clicked() {
                        let hosts = &mut self.remote_ui.hosts;
                        if open {
                            hosts.forwards_open.remove(&project.id);
                        } else {
                            hosts.forwards_open.insert(project.id.clone());
                        }
                    }
                }
                if appearance::sidebar_action(ui, "Terminal", "Open terminal").clicked() {
                    self.open_host_terminal(project.id.clone());
                }
            });
        });
        ui.indent(("host-root", &project.id), |ui| {
            ui.weak(&project.root);
            if !forwarding {
                return;
            }
            if self.remote_ui.hosts.forwards_open.contains(&project.id) {
                self.forwards_section(ui, project);
            } else {
                let active = self
                    .remote_ui
                    .hosts
                    .forwards
                    .active
                    .iter()
                    .filter(|a| a.project == project.id)
                    .count();
                if active > 0 {
                    ui.weak(format!(
                        "{active} port forward{} active",
                        if active == 1 { "" } else { "s" }
                    ));
                }
            }
        });
        if self.remote_ui.hosts.confirm_remove.as_deref() == Some(project.id.as_str()) {
            ui.horizontal_wrapped(|ui| {
                ui.label("Remove this SSH project? Close its terminals first.");
                if ui.button("Remove").clicked() {
                    self.remote_ui.hosts.confirm_remove = None;
                    self.remote_ui.hosts.status.remove(&project.id);
                    self.send(Request::RemoveRemoteProject {
                        project: project.id.clone(),
                    });
                }
                if ui.button("Cancel").clicked() {
                    self.remote_ui.hosts.confirm_remove = None;
                }
            });
        }
        let mut save = None;
        let mut cancel = false;
        if let Some((id, text)) = &mut self.remote_ui.hosts.editing_group
            && *id == project.id
        {
            ui.horizontal(|ui| {
                let response = ui.add(
                    appearance::singleline(text)
                        .hint_text("Group (blank for none)")
                        .desired_width((ui.available_width() - 120.0).max(60.0)),
                );
                let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button("Save").clicked() || enter {
                    save = Some(text.clone());
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        }
        if let Some(text) = save {
            match terminator_core::remote::normalize_group(Some(&text)) {
                Ok(group) => {
                    self.remote_ui.hosts.editing_group = None;
                    self.send(Request::SetRemoteProjectGroup {
                        project: project.id.clone(),
                        group,
                    });
                }
                Err(error) => self.remote_ui.hosts.error = Some(error.to_string()),
            }
        } else if cancel {
            self.remote_ui.hosts.editing_group = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn project(id: &str, host: &str, group: Option<&str>) -> RemoteProject {
        RemoteProject {
            id: id.into(),
            name: id.into(),
            ssh: SshOptions {
                host: host.into(),
                ..SshOptions::default()
            },
            root: format!("/srv/{id}"),
            layout: serde_json::Value::Null,
            group: group.map(Into::into),
            forwards: Vec::new(),
        }
    }
    fn names(groups: &[HostGroup<'_>]) -> Vec<(Option<String>, String, Vec<String>)> {
        groups
            .iter()
            .flat_map(|g| {
                g.hosts.iter().map(|(host, projects)| {
                    (
                        g.name.map(Into::into),
                        host.clone(),
                        projects.iter().map(|p| p.name.clone()).collect(),
                    )
                })
            })
            .collect()
    }

    #[test]
    fn hosts_group_by_folder_then_host_with_ungrouped_last() {
        let projects = vec![
            project("web", "beta", None),
            project("db", "alpha", Some("prod")),
            project("api", "alpha", Some("prod")),
            project("ci", "gamma", Some("Lab")),
        ];
        assert_eq!(
            names(&group_hosts(&projects, "")),
            vec![
                (Some("Lab".into()), "gamma".into(), vec!["ci".into()]),
                (
                    Some("prod".into()),
                    "alpha".into(),
                    vec!["api".into(), "db".into()]
                ),
                (None, "beta".into(), vec!["web".into()]),
            ]
        );
    }

    #[test]
    fn host_filter_matches_name_host_root_and_group() {
        let projects = vec![
            project("web", "beta", None),
            project("db", "alpha", Some("Prod")),
        ];
        let only = |filter| {
            names(&group_hosts(&projects, filter))
                .into_iter()
                .flat_map(|(_, _, p)| p)
                .collect::<Vec<_>>()
        };
        assert_eq!(only("PROD"), ["db"]);
        assert_eq!(only(" beta "), ["web"]);
        assert_eq!(only("/srv/db"), ["db"]);
        assert!(only("nothing").is_empty());
    }

    #[test]
    fn quick_connect_lists_only_unsaved_aliases() {
        let config = vec![
            ConfigHost {
                alias: "alpha".into(),
                hostname: "alpha.example".into(),
                ..ConfigHost::default()
            },
            ConfigHost {
                alias: "beta".into(),
                hostname: "beta.example".into(),
                ..ConfigHost::default()
            },
        ];
        let projects = vec![project("db", "alpha", None)];
        let aliases: Vec<_> = quick_aliases(&config, &projects, "")
            .into_iter()
            .map(|h| h.alias.as_str())
            .collect();
        assert_eq!(aliases, ["beta"]);
        assert!(quick_aliases(&config, &projects, "zzz").is_empty());
    }
}
