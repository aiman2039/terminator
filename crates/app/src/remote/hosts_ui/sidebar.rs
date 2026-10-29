//! Hosts sidebar rendering: header, address bar, collapsible groups and
//! quick connect. Host rows live in `rows`.
use super::{HostCursor, HostGroup, group_hosts, parse_target, quick_aliases};
use crate::{App, appearance};
use eframe::egui;
use terminator_core::{
    remote::RemoteProject,
    ssh::{ConfigHost, SshFailure},
};

/// Data shared by the scrollable part of the Hosts sidebar.
pub(super) struct HostListing<'a> {
    pub(super) projects: &'a [RemoteProject],
    pub(super) groups: &'a [HostGroup<'a>],
    pub(super) aliases: &'a [&'a ConfigHost],
    pub(super) config_loaded: bool,
    pub(super) filtering: bool,
    pub(super) managed: bool,
    pub(super) quick_busy: bool,
}

/// The fix offered next to a quick-connect error.
#[derive(Clone, Copy)]
enum QuickFix {
    /// Open Properties so the user sets a path and signs in in a terminal.
    SetUp,
    HostKey,
    Retry,
}

impl QuickFix {
    fn for_failure(failure: &SshFailure) -> Option<Self> {
        match failure {
            SshFailure::AuthFailed => Some(Self::SetUp),
            SshFailure::UnknownHostKey | SshFailure::ChangedHostKey => Some(Self::HostKey),
            SshFailure::Unreachable => Some(Self::Retry),
            SshFailure::Other(_) => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::SetUp => "Set up…",
            Self::HostKey => "Review host key",
            Self::Retry => "Retry",
        }
    }
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

        let projects = self.state.remote_projects.clone();
        let filter = self.preferences.hosts_filter.clone();
        let groups = group_hosts(&projects, &filter, self.preferences.hosts_recent_first);
        let config = self.remote_ui.hosts.config_hosts.clone();
        let aliases = config
            .as_deref()
            .map(|c| quick_aliases(c, &projects, &filter))
            .unwrap_or_default();
        let listing = HostListing {
            projects: &projects,
            groups: &groups,
            aliases: &aliases,
            config_loaded: config.is_some(),
            filtering: !filter.trim().is_empty(),
            managed,
            quick_busy: self.remote_ui.hosts.quick.is_some(),
        };
        self.hosts_address_bar(ui, &listing);
        self.hosts_error_row(ui);
        if !managed {
            ui.weak("Restart the session service to group, remove and quick-connect hosts.");
        }
        ui.add_space(6.0);
        appearance::sidebar_scroll("hosts-sidebar").show(ui, |ui| {
            self.host_groups(ui, &listing);
            self.quick_connect_section(ui, &listing);
        });
    }

    fn hosts_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Hosts");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if appearance::sidebar_action(ui, "Plus", "New SSH host").clicked() {
                    self.new_ssh_host();
                }
                if appearance::sidebar_action(ui, "FileKey", "Import hosts from ~/.ssh/config")
                    .clicked()
                {
                    self.open_ssh_import();
                }
                if appearance::sidebar_action(ui, "RefreshCw", "Reload ~/.ssh/config").clicked() {
                    self.load_config_hosts();
                    self.remote_ui.hosts.status_polled = None;
                }
                let recent = self.preferences.hosts_recent_first;
                let tip = if recent {
                    "Sorted by last connection; click to sort by name"
                } else {
                    "Sorted by name; click to sort by last connection"
                };
                if appearance::selectable_icon(appearance::SelectableIconInput {
                    ui,
                    icon: "History",
                    tip,
                    selected: recent,
                })
                .clicked()
                {
                    self.preferences.hosts_recent_first = !recent;
                }
            });
        });
    }

    /// Rows the keyboard can select, in display order.
    fn cursor_entries(&self, listing: &HostListing<'_>) -> Vec<HostCursor> {
        let mut entries = Vec::new();
        for group in listing.groups {
            if !self.group_collapsed(group, listing) {
                entries.extend(
                    group
                        .projects
                        .iter()
                        .map(|p| HostCursor::Project(p.id.clone())),
                );
            }
        }
        if listing.managed {
            entries.extend(
                listing
                    .aliases
                    .iter()
                    .map(|h| HostCursor::Alias(h.alias.clone())),
            );
        }
        entries
    }

    /// Filter field that also connects: arrows move the selection, Enter
    /// opens the selected or first match, or quick-connects a typed
    /// `user@host:port`.
    fn hosts_address_bar(&mut self, ui: &mut egui::Ui, listing: &HostListing<'_>) {
        let entries = self.cursor_entries(listing);
        let mut submit = false;
        ui.horizontal(|ui| {
            let field = ui.add(
                appearance::singleline(&mut self.preferences.hosts_filter)
                    .hint_text("Filter, or connect to user@host…")
                    .desired_width((ui.available_width() - 28.0).max(60.0)),
            );
            if field.changed() {
                self.remote_ui.hosts.cursor = None;
            }
            if field.has_focus() {
                let step = ui.input_mut(|i| {
                    if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                        Some(true)
                    } else if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                        Some(false)
                    } else {
                        None
                    }
                });
                if let Some(down) = step {
                    self.move_host_cursor(&entries, down);
                }
            }
            submit = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if !self.preferences.hosts_filter.is_empty()
                && appearance::sidebar_action(ui, "X", "Clear filter").clicked()
            {
                self.preferences.hosts_filter.clear();
                self.remote_ui.hosts.cursor = None;
            }
        });
        if submit {
            self.submit_host_address(&entries, listing);
        }
    }

    fn move_host_cursor(&mut self, entries: &[HostCursor], down: bool) {
        let last = entries.len().saturating_sub(1);
        let hosts = &mut self.remote_ui.hosts;
        let next = match hosts
            .cursor
            .as_ref()
            .and_then(|c| entries.iter().position(|e| e == c))
        {
            Some(index) if down => index.saturating_add(1).min(last),
            Some(index) => index.saturating_sub(1),
            None if down => 0,
            None => last,
        };
        hosts.cursor = entries.get(next).cloned();
    }

    fn submit_host_address(&mut self, entries: &[HostCursor], listing: &HostListing<'_>) {
        let typed = self.preferences.hosts_filter.trim().to_owned();
        let chosen = self
            .remote_ui
            .hosts
            .cursor
            .clone()
            .filter(|c| entries.contains(c))
            .or_else(|| {
                (!typed.is_empty())
                    .then(|| entries.first().cloned())
                    .flatten()
            });
        match chosen {
            Some(HostCursor::Project(id)) => self.open_host_terminal(id),
            Some(HostCursor::Alias(alias)) if !listing.quick_busy => self.quick_connect(&alias),
            Some(HostCursor::Alias(_)) => {}
            None if listing.managed && !listing.quick_busy => {
                if let Some(ssh) = parse_target(&typed) {
                    self.quick_connect_to(ssh);
                }
            }
            None => {}
        }
    }

    fn hosts_error_row(&mut self, ui: &mut egui::Ui) {
        let Some(error) = self.remote_ui.hosts.error.clone() else {
            return;
        };
        let failure = self.remote_ui.hosts.failure.clone();
        let fix = failure
            .as_ref()
            .and_then(|(ssh, project)| Some((QuickFix::for_failure(ssh)?, project.clone())));
        let mut chosen = None;
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(appearance::color(&self.theme.status_failed), error);
            if let Some((fix, _)) = &fix
                && ui.small_button(fix.label()).clicked()
            {
                chosen = Some(*fix);
            }
            if appearance::sidebar_action(ui, "X", "Dismiss").clicked() {
                self.remote_ui.hosts.error = None;
                self.remote_ui.hosts.failure = None;
            }
        });
        if let (Some(fix), Some((ssh, project))) = (chosen, failure) {
            self.apply_quick_fix(fix, ssh, project);
        }
    }

    fn apply_quick_fix(&mut self, fix: QuickFix, ssh: SshFailure, project: RemoteProject) {
        self.remote_ui.hosts.error = None;
        self.remote_ui.hosts.failure = None;
        match fix {
            QuickFix::SetUp => {
                let mut draft = project;
                if draft.root == terminator_core::remote::HOME_ROOT {
                    draft.root.clear();
                }
                self.open_ssh_properties(draft);
                self.remote_ui.error = Some(
                    "Password or 2FA sign-in happens in a terminal. Set the remote path, then add the host."
                        .into(),
                );
            }
            QuickFix::HostKey => self.prompt_host_key(
                project.ssh.clone(),
                crate::remote_ui::Retry::Quick(Box::new(project)),
                ssh == SshFailure::ChangedHostKey,
            ),
            QuickFix::Retry => self.start_quick_connect(project),
        }
    }

    fn group_collapsed(&self, group: &HostGroup<'_>, listing: &HostListing<'_>) -> bool {
        !listing.filtering
            && listing.groups.iter().any(|g| g.name.is_some())
            && self.preferences.hosts_collapsed.contains(group.key())
    }

    fn host_groups(&mut self, ui: &mut egui::Ui, listing: &HostListing<'_>) {
        if listing.projects.is_empty() {
            ui.weak(
                "No saved SSH hosts. Click +, import from ~/.ssh/config, or type user@host above.",
            );
        } else if listing.groups.is_empty() {
            ui.weak("No matching hosts. Press Enter to connect to what you typed.");
        }
        let any_group = listing.groups.iter().any(|g| g.name.is_some());
        for group in listing.groups {
            let collapsed = self.group_collapsed(group, listing);
            if any_group && self.group_header(ui, group, collapsed).clicked() {
                let key = group.key().to_owned();
                if collapsed {
                    self.preferences.hosts_collapsed.remove(&key);
                } else {
                    self.preferences.hosts_collapsed.insert(key);
                }
            }
            if collapsed {
                continue;
            }
            for project in &group.projects {
                self.host_row(ui, project, listing.managed);
            }
            ui.add_space(4.0);
        }
    }

    fn group_header(
        &self,
        ui: &mut egui::Ui,
        group: &HostGroup<'_>,
        collapsed: bool,
    ) -> egui::Response {
        let count = group.projects.len().to_string();
        appearance::row(appearance::RowInput {
            tint: ui.visuals().weak_text_color(),
            ui,
            label: group.name.unwrap_or("Ungrouped"),
            icon: if collapsed {
                "ChevronRight"
            } else {
                "ChevronDown"
            },
            selected: false,
            height: 24.0,
            trailing: &count,
        })
    }

    fn quick_connect_section(&mut self, ui: &mut egui::Ui, listing: &HostListing<'_>) {
        ui.separator();
        ui.weak("Quick connect · ~/.ssh/config");
        if !listing.config_loaded {
            ui.weak("Loading…");
            return;
        }
        if listing.aliases.is_empty() {
            ui.weak("No other config hosts.");
        }
        for host in listing.aliases {
            let selected = self.remote_ui.hosts.cursor.as_ref()
                == Some(&HostCursor::Alias(host.alias.clone()));
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(&host.alias).selectable(false))
                    .on_hover_text(crate::remote_ui::config_host_summary(host));
                if selected {
                    ui.weak("↵");
                }
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
}
