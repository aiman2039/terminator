//! Hosts sidebar rendering: header, filter and collapsible groups. Host
//! rows live in `rows`; adding and editing happen on the SSH page.
use super::{HostGroup, group_hosts};
use crate::{App, appearance};
use eframe::egui;
use terminator_core::remote::RemoteProject;

/// Data shared by the scrollable part of the Hosts sidebar.
pub(super) struct HostListing<'a> {
    pub(super) projects: &'a [RemoteProject],
    pub(super) groups: &'a [HostGroup<'a>],
    pub(super) filtering: bool,
    pub(super) managed: bool,
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
        self.poll_host_status(ui.ctx());
        self.poll_forwards(ui.ctx());

        let projects = self.state.remote_projects.clone();
        let filter = self.preferences.hosts_filter.clone();
        let groups = group_hosts(&projects, &filter, self.preferences.hosts_recent_first);
        let listing = HostListing {
            projects: &projects,
            groups: &groups,
            filtering: !filter.trim().is_empty(),
            managed,
        };
        self.hosts_filter_bar(ui, &listing);
        self.hosts_error_row(ui);
        if !managed {
            ui.weak("Restart the session service to group and remove hosts.");
        }
        ui.add_space(6.0);
        appearance::sidebar_scroll("hosts-sidebar").show(ui, |ui| {
            self.host_groups(ui, &listing);
        });
    }

    fn hosts_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.strong("Hosts");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if appearance::sidebar_action(ui, "Plus", "New SSH host").clicked() {
                    self.new_ssh_host();
                }
                if appearance::sidebar_action(ui, "LayoutDashboard", "Show all hosts").clicked() {
                    self.show_ssh_page();
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

    /// Project ids the keyboard can select, in display order.
    fn cursor_entries(&self, listing: &HostListing<'_>) -> Vec<String> {
        let mut entries = Vec::new();
        for group in listing.groups {
            if !self.group_collapsed(group, listing) {
                entries.extend(group.projects.iter().map(|p| p.id.clone()));
            }
        }
        entries
    }

    /// Filter field: arrows move the selection, Enter opens a terminal on
    /// the selected or first match.
    fn hosts_filter_bar(&mut self, ui: &mut egui::Ui, listing: &HostListing<'_>) {
        let entries = self.cursor_entries(listing);
        let mut submit = false;
        ui.horizontal(|ui| {
            let field = ui.add(
                appearance::singleline(&mut self.preferences.hosts_filter)
                    .hint_text("Filter hosts…")
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
            let chosen = self
                .remote_ui
                .hosts
                .cursor
                .clone()
                .filter(|c| entries.contains(c))
                .or_else(|| entries.first().cloned());
            if let Some(id) = chosen {
                self.open_host_terminal(id);
            }
        }
    }

    fn move_host_cursor(&mut self, entries: &[String], down: bool) {
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

    fn group_collapsed(&self, group: &HostGroup<'_>, listing: &HostListing<'_>) -> bool {
        !listing.filtering
            && listing.groups.iter().any(|g| g.name.is_some())
            && self.preferences.hosts_collapsed.contains(group.key())
    }

    fn host_groups(&mut self, ui: &mut egui::Ui, listing: &HostListing<'_>) {
        if listing.projects.is_empty() {
            ui.weak("No saved SSH hosts. Click + to add one.");
        } else if listing.groups.is_empty() {
            ui.weak("No matching hosts.");
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
}
