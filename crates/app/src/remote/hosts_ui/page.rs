//! The SSH page in the main pane, like the VNC page: an address bar that
//! finds or adds a host, and saved hosts as cards or rows. New host,
//! Properties and import open on this page; the Hosts sidebar only lists
//! hosts.
use super::{matches, parse_target, rows::subtitle};
use crate::vnc_ui::{
    CardAction,
    book_keys::{CARD_GAP, grid_key_action},
    card::{
        CARD_HEIGHT, CARD_WIDTH, CardContent, PaintCardInput, ROW_HEIGHT, new_host_card,
        paint_card, paint_row,
    },
    form,
};
use crate::{App, appearance};
use eframe::egui::{self, RichText};
use terminator_core::{Request, remote::RemoteProject};

impl App {
    /// Show the SSH page in the main pane, replacing any other center view.
    pub(crate) fn show_ssh_page(&mut self) {
        self.hide_center_overlay();
        self.remote_ui.hosts.page_open = true;
    }

    /// Whether the main pane shows the SSH page (hosts, Properties or
    /// import).
    pub(crate) fn ssh_page_visible(&self) -> bool {
        self.remote_ui.hosts.page_open || self.remote_ui.open || self.remote_ui.import.is_some()
    }

    /// The main pane while the SSH page is open.
    pub(crate) fn ssh_page_center(&mut self, ui: &mut egui::Ui) {
        let style = appearance::CenterCardStyle::new(&self.theme);
        let (mut close, dismissed) = appearance::center_card(ui, style, |ui| {
            let mut close = false;
            ui.set_min_size(ui.available_size());
            ui.horizontal(|ui| {
                ui.label(RichText::new("SSH").size(appearance::FONT_TITLE).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close = appearance::sidebar_action(ui, "X", "Close").clicked();
                });
            });
            ui.add_space(8.0);
            close |= self.ssh_discard_bar(ui);
            if self.remote_ui.open {
                self.ssh_properties_body(ui);
            } else if self.remote_ui.import.is_some() {
                self.ssh_import_body(ui);
            } else if self.remote_supported() {
                self.ssh_address_book(ui);
            } else {
                ui.colored_label(
                    appearance::color(&self.theme.status_waiting),
                    "Restart the session service to enable SSH projects.",
                );
            }
            close
        });
        // A margin click must not silently drop a changed host form.
        if dismissed && self.remote_ui.draft_changed() {
            self.remote_ui.confirm_discard = true;
        } else {
            close |= dismissed;
        }
        if close {
            self.remote_ui.close();
            self.remote_ui.import = None;
            self.remote_ui.hosts.page_open = false;
            self.selection_generation = self.selection_generation.wrapping_add(1);
        }
    }

    /// Asked after a margin click on a changed host form. Returns whether
    /// the user chose to discard the changes.
    fn ssh_discard_bar(&mut self, ui: &mut egui::Ui) -> bool {
        if !self.remote_ui.confirm_discard {
            return false;
        }
        let mut discard = false;
        ui.horizontal(|ui| {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "Discard the unsaved changes to this host?",
            );
            discard = ui.button("Discard changes").clicked();
            if ui.button("Keep editing").clicked() {
                self.remote_ui.confirm_discard = false;
            }
        });
        ui.add_space(8.0);
        discard
    }

    fn ssh_address_book(&mut self, ui: &mut egui::Ui) {
        self.poll_host_status(ui.ctx());
        self.ssh_address_bar(ui);
        self.ssh_confirm_remove_bar(ui);
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .id_salt("ssh-address-book")
            .auto_shrink([false, false])
            .show(ui, |ui| self.ssh_host_grid(ui));
    }

    /// Typing filters the cards; Enter or Connect opens a terminal on the
    /// matching saved host, or New host with the address filled in.
    fn ssh_address_bar(&mut self, ui: &mut egui::Ui) {
        let hosts = &mut self.remote_ui.hosts;
        let submit = ui
            .horizontal(|ui| {
                let width = (ui.available_width() - 90.0).max(120.0);
                let field = ui.add(
                    appearance::singleline(&mut hosts.address)
                        .hint_text("Find or add a host: user@host:port or config alias")
                        .desired_width(width),
                );
                if field.changed() {
                    hosts.page_error = None;
                }
                let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if enter {
                    // The cards below must not also see this Enter.
                    ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
                }
                enter || form::primary_button(ui, "Connect").clicked()
            })
            .inner;
        if let Some(error) = &self.remote_ui.hosts.page_error {
            ui.colored_label(appearance::color(&self.theme.status_failed), error);
        }
        if submit {
            self.ssh_open_address();
        }
    }

    pub(crate) fn ssh_open_address(&mut self) {
        let typed = self.remote_ui.hosts.address.trim().to_owned();
        if typed.is_empty() {
            return;
        }
        let Some(ssh) = parse_target(&typed) else {
            self.remote_ui.hosts.page_error =
                Some("Enter a host name or config alias, user@host or user@host:port.".into());
            return;
        };
        self.remote_ui.hosts.address.clear();
        let existing = self
            .state
            .remote_projects
            .iter()
            .filter(|p| {
                p.ssh.host.eq_ignore_ascii_case(&ssh.host)
                    && (ssh.username.is_empty() || p.ssh.username == ssh.username)
                    && (ssh.port.is_none() || p.ssh.port == ssh.port)
            })
            .max_by_key(|p| p.last_connected)
            .map(|p| p.id.clone());
        if let Some(id) = existing {
            self.open_host_terminal(id);
            return;
        }
        let draft = RemoteProject {
            name: ssh.authority(),
            ssh,
            ..crate::remote_ui::blank_project()
        };
        self.open_ssh_properties(draft);
    }

    fn ssh_confirm_remove_bar(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.remote_ui.hosts.confirm_remove.clone() else {
            return;
        };
        let Some(name) = self
            .state
            .remote_projects
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.name.clone())
        else {
            self.remote_ui.hosts.confirm_remove = None;
            return;
        };
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(format!("Remove {name}? Close its terminals first."));
            if ui.button("Remove").clicked() {
                self.remote_ui.hosts.confirm_remove = None;
                self.remote_ui.hosts.status.remove(&id);
                self.send(Request::RemoveRemoteProject {
                    project: id.clone(),
                });
            }
            if ui.button("Cancel").clicked() {
                self.remote_ui.hosts.confirm_remove = None;
            }
        });
    }

    fn ssh_host_grid(&mut self, ui: &mut egui::Ui) {
        let filter = self.remote_ui.hosts.address.trim().to_lowercase();
        let groups = self.group_names();
        let hosts = &mut self.remote_ui.hosts;
        if hosts
            .group_filter
            .as_ref()
            .is_some_and(|g| !groups.contains(g))
        {
            hosts.group_filter = None;
        }
        let group = hosts.group_filter.clone();
        let mut projects: Vec<RemoteProject> = self
            .state
            .remote_projects
            .iter()
            .filter(|p| matches(p, &filter))
            .filter(|p| group.is_none() || p.group == group)
            .cloned()
            .collect();
        projects.sort_by_key(|p| std::cmp::Reverse(p.last_connected));
        self.ssh_book_controls(ui, &groups);
        ui.add_space(8.0);
        let mut actions: Vec<(RemoteProject, CardAction)> = grid_key_action(
            ui,
            &projects,
            |p| &p.id,
            &mut self.remote_ui.hosts.selected_card,
        )
        .into_iter()
        .collect();
        actions.extend(self.ssh_host_items(ui, &projects));
        ui.add_space(10.0);
        let count = projects.len();
        form::hint(
            ui,
            &format!(
                "{count} host{} · Double-click or Enter to open a terminal · Right-click for more",
                if count == 1 { "" } else { "s" }
            ),
            "",
        );
        for (project, action) in actions {
            self.ssh_card_action(&project, action);
        }
    }

    /// Group chips to filter by, Import, and the Grid / List switch.
    fn ssh_book_controls(&mut self, ui: &mut egui::Ui, groups: &[String]) {
        let mut import = false;
        let hosts = &mut self.remote_ui.hosts;
        ui.horizontal_wrapped(|ui| {
            if !groups.is_empty() {
                if ui
                    .selectable_label(hosts.group_filter.is_none(), "All")
                    .clicked()
                {
                    hosts.group_filter = None;
                }
                for group in groups {
                    let on = hosts.group_filter.as_ref() == Some(group);
                    if ui.selectable_label(on, group).clicked() {
                        hosts.group_filter = (!on).then(|| group.clone());
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                import = ui
                    .button("Import…")
                    .on_hover_text("Import hosts from ~/.ssh/config")
                    .clicked();
                form::segmented(ui, &mut hosts.list_view, &[(true, "List"), (false, "Grid")]);
            });
        });
        if import {
            self.open_ssh_import();
        }
    }

    /// Hosts as cards or rows, then the "New host" item.
    fn ssh_host_items(
        &mut self,
        ui: &mut egui::Ui,
        projects: &[RemoteProject],
    ) -> Vec<(RemoteProject, CardAction)> {
        let mut actions = Vec::new();
        if self.remote_ui.hosts.list_view {
            for project in projects {
                if let Some(action) = self.ssh_host_item(ui, project, false) {
                    actions.push((project.clone(), action));
                }
            }
            if ui.button("+ New host").clicked() {
                self.new_ssh_host();
            }
            return actions;
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(CARD_GAP, CARD_GAP);
            for project in projects {
                if let Some(action) = self.ssh_host_item(ui, project, true) {
                    actions.push((project.clone(), action));
                }
            }
            if new_host_card(ui, "Add an SSH host").clicked() {
                self.new_ssh_host();
            }
        });
        actions
    }

    fn ssh_host_item(
        &mut self,
        ui: &mut egui::Ui,
        project: &RemoteProject,
        card: bool,
    ) -> Option<CardAction> {
        let size = if card {
            egui::vec2(CARD_WIDTH, CARD_HEIGHT)
        } else {
            egui::vec2(ui.available_width(), ROW_HEIGHT)
        };
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
        let status = (self.remote_ui.hosts.status.get(&project.id) == Some(&true))
            .then(|| appearance::color(&self.theme.status_running));
        let forwarding = self
            .remote_ui
            .hosts
            .forwards
            .active
            .iter()
            .any(|a| a.project == project.id);
        let input = PaintCardInput {
            ui,
            rect,
            hovered: response.hovered(),
            selected: self.remote_ui.hosts.selected_card.as_deref() == Some(project.id.as_str()),
            content: card_content(project, forwarding),
            texture: None,
            status,
        };
        if card {
            paint_card(input);
        } else {
            paint_row(input);
        }
        self.ssh_item_response(response, project)
    }

    /// Selection, double-click and the context menu of a card or row.
    fn ssh_item_response(
        &mut self,
        response: egui::Response,
        project: &RemoteProject,
    ) -> Option<CardAction> {
        if response.clicked() || response.secondary_clicked() {
            self.remote_ui.hosts.selected_card = Some(project.id.clone());
        }
        let response = response.on_hover_text(project.label());
        let mut action = response.double_clicked().then_some(CardAction::Connect);
        let editable = self.edit_supported();
        let managed = self.hosts_supported();
        appearance::context_menu(&response, |ui| {
            appearance::target_header(ui, &project.name, &project.label());
            let items = [
                ("Open terminal", "Terminal", CardAction::Connect, true),
                ("Properties", "Settings2", CardAction::Properties, editable),
                ("Duplicate", "Copy", CardAction::Duplicate, true),
            ];
            for (label, icon, item, enabled) in items {
                if enabled && menu_item(ui, label, icon) {
                    action = Some(item);
                }
            }
            if managed {
                ui.separator();
                if menu_item(ui, "Remove host…", "Trash") {
                    action = Some(CardAction::Remove);
                }
            }
        });
        action
    }

    fn ssh_card_action(&mut self, project: &RemoteProject, action: CardAction) {
        match action {
            CardAction::Connect => self.open_host_terminal(project.id.clone()),
            CardAction::Properties if self.edit_supported() => {
                self.open_ssh_properties(project.clone());
            }
            CardAction::Properties => {
                self.remote_ui.hosts.page_error =
                    Some("Restart the session service to edit SSH hosts.".into());
            }
            CardAction::Duplicate => self.duplicate_ssh_host(project),
            CardAction::Remove if self.hosts_supported() => {
                self.remote_ui.hosts.confirm_remove = Some(project.id.clone());
            }
            CardAction::Remove => {}
        }
    }
}

/// Name, "user@host · /root · 2h ago · #group", and a globe while a port
/// forward runs.
fn card_content(project: &RemoteProject, forwarding: bool) -> CardContent {
    let mut text = subtitle(project, terminator_core::now());
    if let Some(group) = &project.group {
        text.push_str(" · #");
        text.push_str(group);
    }
    CardContent {
        name: project.name.clone(),
        subtitle: text,
        placeholder: "Terminal",
        corner: forwarding.then_some("Globe"),
    }
}

/// One context-menu row; closes the menu when clicked.
fn menu_item(ui: &mut egui::Ui, label: &str, icon: &str) -> bool {
    let clicked = appearance::menu_item(appearance::MenuItemInput {
        ui,
        label,
        icon,
        shortcut: "",
    })
    .clicked();
    if clicked {
        ui.close();
    }
    clicked
}
