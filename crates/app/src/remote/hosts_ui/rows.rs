//! One saved host in the Hosts sidebar: status dot, name, a subtitle with
//! address, path and last connection, and actions. Rarely used actions sit
//! in the row menu; the forwards toggle shows on hover or while in use.
use super::HostCursor;
use crate::{App, appearance};
use eframe::egui::{self, RichText};
use terminator_core::{Request, remote::RemoteProject};

/// "user@host:port · /root · 2h ago".
pub(super) fn subtitle(project: &RemoteProject, now: u64) -> String {
    let mut text = format!("{} · {}", project.ssh.authority(), project.root);
    if project.last_connected > 0 {
        text.push_str(" · ");
        text.push_str(&crate::agent_presence::ago(project.last_connected, now));
    }
    text
}

impl App {
    pub(super) fn host_row(&mut self, ui: &mut egui::Ui, project: &RemoteProject, managed: bool) {
        ui.horizontal(|ui| {
            self.host_status_and_name(ui, project);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.host_row_actions(ui, project, managed);
            });
        });
        ui.indent(("host-root", &project.id), |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(subtitle(project, terminator_core::now()))
                        .size(11.0)
                        .color(ui.visuals().weak_text_color()),
                )
                .truncate(),
            )
            .on_hover_text(project.label());
            if self.forwarding_supported() {
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
        let selected = self.selected.as_deref() == Some(project.id.as_str())
            || self.remote_ui.hosts.cursor.as_ref()
                == Some(&HostCursor::Project(project.id.clone()));
        let name = ui.selectable_label(selected, &project.name);
        if name.double_clicked() {
            self.open_host_terminal(project.id.clone());
        } else if name.clicked() {
            self.reveal_project(project.id.clone());
        }
        name.on_hover_text("Click to show files; double-click to open a terminal");
    }

    fn host_row_actions(&mut self, ui: &mut egui::Ui, project: &RemoteProject, managed: bool) {
        appearance::icon_menu_button(ui, "Menu", |ui| self.host_menu(ui, project, managed));
        if appearance::sidebar_action(ui, "Terminal", "Open terminal").clicked() {
            self.open_host_terminal(project.id.clone());
        }
        if !self.forwarding_supported() {
            return;
        }
        let open = self.remote_ui.hosts.forwards_open.contains(&project.id);
        let active = self
            .remote_ui
            .hosts
            .forwards
            .active
            .iter()
            .any(|a| a.project == project.id);
        let row_hovered = ui.rect_contains_pointer(ui.max_rect());
        if (open || active || !project.forwards.is_empty() || row_hovered)
            && appearance::selectable_icon(appearance::SelectableIconInput {
                ui,
                icon: "Globe",
                tip: "Port forwards",
                selected: open,
            })
            .clicked()
        {
            self.toggle_host_forwards(&project.id);
        }
    }

    fn toggle_host_forwards(&mut self, id: &str) {
        let hosts = &mut self.remote_ui.hosts;
        if !hosts.forwards_open.remove(id) {
            hosts.forwards_open.insert(id.to_owned());
        }
    }

    fn host_menu(&mut self, ui: &mut egui::Ui, project: &RemoteProject, managed: bool) {
        if ui.button("Open terminal").clicked() {
            self.open_host_terminal(project.id.clone());
            ui.close();
        }
        ui.separator();
        let editable = self.edit_supported();
        if ui
            .add_enabled(editable, egui::Button::new("Properties…"))
            .on_disabled_hover_text("Restart the session service to edit SSH hosts.")
            .clicked()
        {
            self.open_ssh_properties(project.clone());
            ui.close();
        }
        if ui.button("Duplicate…").clicked() {
            self.duplicate_ssh_host(project);
            ui.close();
        }
        if ui
            .add_enabled(managed, egui::Button::new("Set group…"))
            .clicked()
        {
            self.remote_ui.hosts.editing_group = Some((
                project.id.clone(),
                project.group.clone().unwrap_or_default(),
            ));
            ui.close();
        }
        if self.forwarding_supported() && ui.button("Port forwards").clicked() {
            self.toggle_host_forwards(&project.id);
            ui.close();
        }
        ui.separator();
        if ui
            .add_enabled(managed, egui::Button::new("Remove…"))
            .clicked()
        {
            self.remote_ui.hosts.confirm_remove = Some(project.id.clone());
            ui.close();
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
            ui.label("Remove this SSH host? Close its terminals first.");
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
        let groups = self.group_names();
        if let Some((id, text)) = &mut self.remote_ui.hosts.editing_group
            && *id == project.id
        {
            ui.horizontal(|ui| {
                let response = ui.add(
                    appearance::singleline(text)
                        .hint_text("Group (blank for none)")
                        .desired_width((ui.available_width() - 150.0).max(60.0)),
                );
                let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_enabled_ui(!groups.is_empty(), |ui| {
                    appearance::icon_menu_button(ui, "ChevronDown", |ui| {
                        for name in &groups {
                            if ui.button(name).clicked() {
                                save = Some(name.clone());
                                ui.close();
                            }
                        }
                    });
                });
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
