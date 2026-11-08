//! SSH host Properties on the SSH page: add, edit or duplicate a saved SSH
//! project. Same look as VNC Properties: a titled card, section captions,
//! labels above fields, and one accent button.
use super::fields::{
    apps_command_field, command_fields, connection_fields, group_field, mosh_field, sign_in_fields,
    timeout_field,
};
use super::*;
use crate::vnc_ui::form;
use eframe::egui::RichText;
use std::time::Duration;
use terminator_core::remote::HOME_ROOT;

/// What the page asked for this frame.
#[derive(Default)]
struct PageActions {
    test: bool,
    login_dir: bool,
    save: bool,
    cancel: bool,
    fix: Option<FormFix>,
}

/// One fix button for a failed test or save, chosen from the SSH failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FormFix {
    /// Passwords and 2FA need a terminal: save, then open one.
    SignIn,
    /// Show the host-key prompt again.
    HostKey,
    /// Test the connection again.
    Retry,
}

impl FormFix {
    pub(crate) fn for_failure(failure: &SshFailure) -> Option<Self> {
        match failure {
            SshFailure::AuthFailed => Some(Self::SignIn),
            SshFailure::UnknownHostKey | SshFailure::ChangedHostKey => Some(Self::HostKey),
            SshFailure::Unreachable => Some(Self::Retry),
            SshFailure::Other(_) => None,
        }
    }

    fn label(self, editing: bool) -> &'static str {
        match self {
            Self::SignIn if editing => "Save and open terminal",
            Self::SignIn => "Add and open terminal",
            Self::HostKey => "Review host key",
            Self::Retry => "Retry",
        }
    }
}

impl App {
    /// Opens SSH Properties on the SSH page on `draft`.
    pub(crate) fn open_ssh_properties(&mut self, draft: RemoteProject) {
        if self.keep_changed_ssh_form() {
            return;
        }
        self.show_ssh_page();
        let generation = self.remote_ui.open_draft(draft);
        let _ = self.jobs.send(Job::SshConfigHosts(generation));
    }

    pub(crate) fn new_ssh_host(&mut self) {
        self.open_ssh_properties(blank_project());
    }

    /// A copy without id, layout or forwards; the user changes its path.
    pub(crate) fn duplicate_ssh_host(&mut self, project: &RemoteProject) {
        let mut copy = project.clone();
        copy.id.clear();
        copy.name = format!("{} copy", project.name);
        copy.layout = serde_json::Value::Null;
        copy.forwards.clear();
        copy.last_connected = 0;
        self.open_ssh_properties(copy);
    }

    pub(crate) fn edit_supported(&self) -> bool {
        self.state
            .capabilities
            .iter()
            .any(|c| c == terminator_core::remote::EDIT_CAPABILITY)
    }

    /// The SSH page body while Properties is open.
    pub(crate) fn ssh_properties_body(&mut self, ui: &mut egui::Ui) {
        let mut actions = PageActions::default();
        let draft = self.remote_ui.draft.as_ref();
        let editing = draft.is_some_and(|d| !d.id.is_empty());
        let title = match draft {
            Some(d) if editing => format!("{} – Properties", d.name),
            _ => "New host".to_owned(),
        };
        egui::ScrollArea::vertical()
            .id_salt("ssh-properties")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                form::column(ui, |ui| {
                    ui.label(RichText::new(title).size(appearance::FONT_HEADING).strong());
                    ui.add_space(6.0);
                    self.ssh_properties_form(ui, editing, &mut actions);
                });
                self.ssh_properties_forwards(ui);
            });
        if self.remote_ui.probe.busy() {
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        self.apply_page_actions(actions);
    }

    /// Port forwards of the saved host being edited; they save at once
    /// and are not part of the draft.
    fn ssh_properties_forwards(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.remote_ui.draft.as_ref().map(|d| d.id.clone()) else {
            return;
        };
        let Some(saved) = self
            .state
            .remote_projects
            .iter()
            .find(|p| !id.is_empty() && p.id == id)
            .cloned()
        else {
            return;
        };
        ui.add_space(8.0);
        form::column(ui, |ui| {
            ui.separator();
            form::section(ui, "Port forwards");
            self.forwards_section(ui, &saved);
        });
    }

    fn ssh_properties_form(&mut self, ui: &mut egui::Ui, editing: bool, actions: &mut PageActions) {
        let supported = self.remote_supported() && (!editing || self.edit_supported());
        if !supported {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "Restart the session service to save SSH hosts.",
            );
        }
        self.ssh_properties_error(ui, editing, actions);
        let groups = self.group_names();
        let default_timeout = self.state.settings.ssh_command_timeout_seconds;
        let has = |capability: &str| self.state.capabilities.iter().any(|c| c == capability);
        let host_timeout = has(terminator_core::remote::HOST_TIMEOUT_CAPABILITY);
        let mosh = has(terminator_core::remote::MOSH_CAPABILITY);
        let remote = &mut self.remote_ui;
        let Some(draft) = remote.draft.as_mut() else {
            return;
        };
        let hosts = remote.config_hosts.as_deref();
        form::section(ui, "Server");
        connection_fields(ui, draft, hosts);
        timeout_field(
            ui,
            &mut draft.command_timeout_seconds,
            default_timeout,
            host_timeout,
        );
        mosh_field(ui, &mut draft.mosh, mosh);
        command_fields(ui, draft, has(terminator_core::remote::COMMANDS_CAPABILITY));
        apps_command_field(ui, draft, has(terminator_core::remote_apps::CAPABILITY));
        let config = hosts.and_then(|h| h.iter().find(|h| h.alias == draft.ssh.host));
        if let Some(found) = remote.probe.poll(&draft.ssh.host) {
            remote.resolved = Some(found);
        }
        if let Some(host) = config {
            form::hint(
                ui,
                &format!("SSH config: {}", config_host_summary(host)),
                "",
            );
        } else if let Some(found) = remote
            .resolved
            .as_ref()
            .filter(|r| r.host == draft.ssh.host.trim())
        {
            form::hint(ui, &found.found(), "");
        }
        let name_hint = draft.ssh.host.clone();
        form::text(ui, "Name", &mut draft.name, &name_hint);
        group_field(ui, &mut draft.group, &groups);
        form::section(ui, "Project");
        form::label(ui, "Remote path");
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 130.0).max(80.0);
            ui.add(
                appearance::singleline(&mut draft.root)
                    .hint_text("Blank: the login directory")
                    .desired_width(width),
            );
            actions.login_dir = ui
                .add_enabled(
                    supported && !remote.busy,
                    egui::Button::new("Login directory"),
                )
                .on_hover_text("Connect and use the directory SSH starts in")
                .clicked();
        });
        sign_in_fields(ui, &mut draft.ssh);
        Self::ssh_properties_footer(ui, remote, (supported, editing), actions);
    }

    /// `(supported, editing)`: whether saving is possible, and whether the
    /// page edits a saved host.
    fn ssh_properties_footer(
        ui: &mut egui::Ui,
        remote: &mut RemoteUi,
        (supported, editing): (bool, bool),
        actions: &mut PageActions,
    ) {
        ui.add_space(8.0);
        let label = if editing {
            "Open a terminal after saving"
        } else {
            "Open a terminal after adding"
        };
        ui.checkbox(&mut remote.connect_after, label);
        ui.add_space(6.0);
        ui.separator();
        appearance::footer_row(ui, |ui| {
            ui.add_enabled_ui(supported && !remote.busy, |ui| {
                let save = if editing { "Save" } else { "Add host" };
                actions.save = appearance::primary_button(ui, save).clicked();
                actions.test = ui.button("Test connection").clicked();
            });
            actions.cancel |= ui.button("Cancel").clicked();
            if remote.busy {
                ui.add(egui::Spinner::new());
            }
        });
        form::hint(
            ui,
            "New host keys are shown for confirmation before they are trusted. Changed keys are refused.",
            "",
        );
    }

    /// The error with its fix button, chosen from the SSH failure.
    fn ssh_properties_error(&self, ui: &mut egui::Ui, editing: bool, actions: &mut PageActions) {
        let Some(error) = &self.remote_ui.error else {
            return;
        };
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(ui.visuals().error_fg_color, error);
            if let Some(fix) = self
                .remote_ui
                .failure
                .as_ref()
                .and_then(FormFix::for_failure)
                && ui.button(fix.label(editing)).clicked()
            {
                actions.fix = Some(fix);
            }
        });
    }

    /// Group names in use, sorted, for the group menu.
    pub(crate) fn group_names(&self) -> Vec<String> {
        let mut groups: Vec<String> = self
            .state
            .remote_projects
            .iter()
            .filter_map(|p| p.group.clone())
            .collect();
        groups.sort_by_key(|g| g.to_lowercase());
        groups.dedup();
        groups
    }

    fn apply_page_actions(&mut self, actions: PageActions) {
        if actions.cancel {
            self.remote_ui.close();
            self.selection_generation = self.selection_generation.wrapping_add(1);
            return;
        }
        match actions.fix {
            Some(FormFix::SignIn) => {
                self.remote_ui.connect_after = true;
                self.submit_ssh_properties(false);
            }
            Some(FormFix::Retry) => self.submit_ssh_properties(true),
            Some(FormFix::HostKey) => self.review_form_host_key(),
            None => {}
        }
        if actions.login_dir
            && let Some(draft) = &mut self.remote_ui.draft
        {
            HOME_ROOT.clone_into(&mut draft.root);
            self.submit_ssh_properties(true);
        } else if actions.test {
            self.submit_ssh_properties(true);
        } else if actions.save {
            self.save_ssh_properties();
        }
    }

    /// Saves the draft. A new host without a remote path first probes the
    /// login directory, then adds the host there.
    pub(crate) fn save_ssh_properties(&mut self) {
        let Some(draft) = &self.remote_ui.draft else {
            return;
        };
        if draft.id.is_empty() && draft.root.trim().is_empty() {
            self.remote_ui.save_after_probe = true;
            self.submit_ssh_properties(true);
        } else {
            self.submit_ssh_properties(false);
        }
    }

    fn review_form_host_key(&mut self) {
        let Some(draft) = self.remote_ui.draft.clone() else {
            return;
        };
        let changed = self.remote_ui.failure == Some(SshFailure::ChangedHostKey);
        self.prompt_host_key(
            draft.ssh.clone(),
            Retry::Form {
                project: Box::new(draft),
                probe: true,
                generation: self.selection_generation,
            },
            changed,
        );
    }

    /// Validates the draft and sends a probe, an add, or (with an id) an
    /// update.
    pub(crate) fn submit_ssh_properties(&mut self, probe: bool) {
        let Some(mut project) = self.remote_ui.draft.clone() else {
            return;
        };
        project.ssh.host = String::from(project.ssh.host.trim());
        project.ssh.username = String::from(project.ssh.username.trim());
        project.root = String::from(project.root.trim());
        if probe && project.root.is_empty() {
            HOME_ROOT.clone_into(&mut project.root);
        }
        if project.name.trim().is_empty() {
            project.name.clone_from(&project.ssh.host);
        }
        let checked = match terminator_core::remote::normalize_group(project.group.as_deref()) {
            Ok(group) => {
                project.group = group;
                self.check_draft(&project, probe)
            }
            Err(error) => Err(error.to_string()),
        };
        if let Err(error) = checked {
            self.remote_ui.error = Some(error);
            self.remote_ui.failure = None;
            self.remote_ui.save_after_probe = false;
            return;
        }
        self.remote_ui.draft = Some(project.clone());
        self.remote_ui.busy = true;
        self.remote_ui.error = None;
        self.remote_ui.failure = None;
        self.remote_ui.active = Some(project.clone());
        let _ = self.jobs.send(Job::RemoteAdd {
            project,
            probe,
            generation: self.selection_generation,
        });
    }

    /// Field checks before anything is sent. A probe of the login directory
    /// skips the root check; only saving checks for duplicates.
    fn check_draft(&self, project: &RemoteProject, probe: bool) -> Result<(), String> {
        let mut checked = project.clone();
        if probe && checked.root == HOME_ROOT {
            checked.root = "/".into();
        }
        checked.validate().map_err(|e| e.to_string())?;
        if !probe
            && self
                .state
                .remote_projects
                .iter()
                .any(|p| p.id != project.id && p.same_connection(project))
        {
            return Err("Another SSH host already uses this host and path.".into());
        }
        Ok(())
    }
}
