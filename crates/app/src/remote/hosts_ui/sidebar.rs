//! Hosts sidebar rendering: filter, grouped host rows, and quick connect.
use super::{HostGroup, group_hosts, quick_aliases};
use crate::{App, Job, appearance};
use eframe::egui::{self, RichText};
use terminator_core::{Request, remote::RemoteProject, ssh::ConfigHost};

/// Data shared by the scrollable part of the Hosts sidebar.
struct HostListing<'a> {
    projects: &'a [RemoteProject],
    groups: &'a [HostGroup<'a>],
    filter: &'a str,
    config: Option<&'a [ConfigHost]>,
    managed: bool,
    quick_busy: bool,
}

impl App {
    pub(crate) fn hosts_sidebar(&mut self, ui: &mut egui::Ui) {
        self.hosts_header(ui);
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
        self.hosts_filter_row(ui);
        self.hosts_error_row(ui);
        if !managed {
            ui.weak("Restart the session service to group, remove and quick-connect hosts.");
        }
        ui.add_space(6.0);

        let projects = self.state.remote_projects.clone();
        let filter = self.preferences.hosts_filter.clone();
        let groups = group_hosts(&projects, &filter);
        let config = self.remote_ui.hosts.config_hosts.clone();
        let listing = HostListing {
            projects: &projects,
            groups: &groups,
            filter: &filter,
            config: config.as_deref(),
            managed,
            quick_busy: self.remote_ui.hosts.quick.is_some(),
        };
        appearance::sidebar_scroll("hosts-sidebar").show(ui, |ui| {
            self.host_groups(ui, &listing);
            self.quick_connect_section(ui, &listing);
        });
    }

    fn hosts_header(&mut self, ui: &mut egui::Ui) {
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
    }

    fn hosts_filter_row(&mut self, ui: &mut egui::Ui) {
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
    }

    fn hosts_error_row(&mut self, ui: &mut egui::Ui) {
        let Some(error) = self.remote_ui.hosts.error.clone() else {
            return;
        };
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(appearance::color(&self.theme.status_failed), error);
            if appearance::sidebar_action(ui, "X", "Dismiss").clicked() {
                self.remote_ui.hosts.error = None;
            }
        });
    }

    fn host_groups(&mut self, ui: &mut egui::Ui, listing: &HostListing<'_>) {
        if listing.projects.is_empty() {
            ui.weak("No saved SSH projects. Click + or quick-connect a config host below.");
        } else if listing.groups.is_empty() {
            ui.weak("No matching hosts.");
        }
        let any_group = listing.groups.iter().any(|g| g.name.is_some());
        for group in listing.groups {
            if any_group {
                ui.label(RichText::new(group.name.unwrap_or("Ungrouped")).strong());
            }
            for (authority, members) in &group.hosts {
                ui.weak(authority);
                for project in members {
                    self.host_row(ui, project, listing.managed);
                }
            }
            ui.add_space(6.0);
        }
    }

    fn quick_connect_section(&mut self, ui: &mut egui::Ui, listing: &HostListing<'_>) {
        ui.separator();
        ui.weak("Quick connect · ~/.ssh/config");
        let Some(config) = listing.config else {
            ui.weak("Loading…");
            return;
        };
        let aliases = quick_aliases(config, listing.projects, listing.filter);
        if aliases.is_empty() {
            ui.weak("No other config hosts.");
        }
        for host in aliases {
            ui.horizontal(|ui| {
                ui.label(&host.alias).on_hover_text(&host.hostname);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.quick_connect_button(ui, host, listing);
                });
            });
        }
    }

    fn quick_connect_button(
        &mut self,
        ui: &mut egui::Ui,
        host: &ConfigHost,
        listing: &HostListing<'_>,
    ) {
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
            .add_enabled_ui(listing.managed && !listing.quick_busy, |ui| {
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
    }

    fn host_row(&mut self, ui: &mut egui::Ui, project: &RemoteProject, managed: bool) {
        let forwarding = self.forwarding_supported();
        ui.horizontal(|ui| {
            self.host_status_and_name(ui, project);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.host_row_actions(ui, project, managed);
            });
        });
        ui.indent(("host-root", &project.id), |ui| {
            ui.weak(&project.root);
            if forwarding {
                self.host_forwards_summary(ui, project);
            }
        });
        self.host_remove_confirm(ui, project);
        self.host_group_editor(ui, project);
    }

    fn host_status_and_name(&mut self, ui: &mut egui::Ui, project: &RemoteProject) {
        let up = self.remote_ui.hosts.status.get(&project.id).copied();
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
    }

    fn host_row_actions(&mut self, ui: &mut egui::Ui, project: &RemoteProject, managed: bool) {
        let forwarding = self.forwarding_supported();
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
    }

    fn host_forwards_summary(&mut self, ui: &mut egui::Ui, project: &RemoteProject) {
        if self.remote_ui.hosts.forwards_open.contains(&project.id) {
            self.forwards_section(ui, project);
            return;
        }
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

    fn host_remove_confirm(&mut self, ui: &mut egui::Ui, project: &RemoteProject) {
        if self.remote_ui.hosts.confirm_remove.as_deref() != Some(project.id.as_str()) {
            return;
        }
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

    fn host_group_editor(&mut self, ui: &mut egui::Ui, project: &RemoteProject) {
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
