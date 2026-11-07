//! One saved host in the Hosts sidebar: status dot, name, a subtitle with
//! address, path and last connection, and actions. Rarely used actions sit
//! in the row menu; the forwards toggle shows on hover or while in use.
//! The sidebar only lists: group and forward edits open SSH Properties.
use crate::{App, appearance};
use eframe::egui::{self, RichText};
use terminator_core::{Request, remote::RemoteProject};

const EDIT_REFUSED: &str = "Restart the session service to edit SSH hosts.";

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
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.host_row_actions(ui, project, managed);
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    self.host_status_and_name(ui, project);
                });
            });
        });
        ui.indent(("host-root", &project.id), |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(subtitle(project, terminator_core::now()))
                        .size(appearance::FONT_CAPTION)
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
            || self.remote_ui.hosts.cursor.as_deref() == Some(project.id.as_str());
        let name = ui.add(egui::Button::selectable(selected, &project.name).truncate());
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
        if appearance::menu_action(ui, true, "Open terminal", "Terminal").clicked() {
            self.open_host_terminal(project.id.clone());
        }
        ui.separator();
        let editable = self.edit_supported();
        if appearance::menu_action(ui, editable, "Properties…", "Settings2")
            .on_disabled_hover_text(EDIT_REFUSED)
            .clicked()
        {
            self.open_ssh_properties(project.clone());
        }
        if appearance::menu_action(ui, true, "Duplicate…", "Copy").clicked() {
            self.duplicate_ssh_host(project);
        }
        if appearance::menu_action(ui, editable, "Set group…", "Folder")
            .on_hover_text("Edit the group in Properties")
            .on_disabled_hover_text(EDIT_REFUSED)
            .clicked()
        {
            self.open_ssh_properties(project.clone());
        }
        if self.forwarding_supported()
            && appearance::menu_action(ui, editable, "Port forwards…", "Globe")
                .on_hover_text("Add or edit forwards in Properties")
                .on_disabled_hover_text(EDIT_REFUSED)
                .clicked()
        {
            self.open_ssh_properties(project.clone());
        }
        ui.separator();
        if appearance::menu_action(ui, managed, "Remove…", "Trash").clicked() {
            self.remote_ui.hosts.confirm_remove = Some(project.id.clone());
        }
    }

    fn host_forwards_summary(&mut self, ui: &mut egui::Ui, project: &RemoteProject) {
        if self.remote_ui.hosts.forwards_open.contains(&project.id) {
            self.forwards_summary(ui, project);
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
}
