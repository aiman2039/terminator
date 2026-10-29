//! Import many `~/.ssh/config` hosts at once, on a main-pane page like VNC
//! import. Each chosen alias is added either at its login directory (one
//! connection each, so it must sign in without a prompt) or at a fixed
//! path (no connection). Adds run one at a time.
use super::quick_aliases;
use crate::{App, Job, appearance, vnc_ui::form};
use eframe::egui::{self, RichText};
use std::collections::VecDeque;
use terminator_core::{
    remote::{HOME_ROOT, RemoteProject},
    ssh::{ConfigHost, SshOptions},
};

pub(crate) struct Candidate {
    pub(crate) host: ConfigHost,
    pub(crate) selected: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RootChoice {
    Login,
    Path,
}

pub(crate) struct HostImport {
    /// `None` until the config hosts have loaded.
    pub(crate) candidates: Option<Vec<Candidate>>,
    pub(crate) root: RootChoice,
    pub(crate) path: String,
    pub(crate) group: String,
    pub(crate) queue: VecDeque<RemoteProject>,
    pub(crate) in_flight: Option<RemoteProject>,
    pub(crate) added: usize,
    /// Alias and why it was not added.
    pub(crate) failed: Vec<(String, String)>,
    pub(crate) error: Option<String>,
}

impl HostImport {
    fn new() -> Self {
        Self {
            candidates: None,
            root: RootChoice::Login,
            path: String::new(),
            group: String::new(),
            queue: VecDeque::new(),
            in_flight: None,
            added: 0,
            failed: Vec::new(),
            error: None,
        }
    }

    fn running(&self) -> bool {
        self.in_flight.is_some() || !self.queue.is_empty()
    }

    fn finished(&self) -> bool {
        !self.running() && (self.added > 0 || !self.failed.is_empty())
    }

    /// The projects to add, or why the choices are invalid.
    fn plan(&self) -> Result<VecDeque<RemoteProject>, String> {
        let group = terminator_core::remote::normalize_group(Some(&self.group))
            .map_err(|e| e.to_string())?;
        let root = match self.root {
            RootChoice::Login => HOME_ROOT.to_owned(),
            RootChoice::Path => self.path.trim().to_owned(),
        };
        let mut plan = VecDeque::new();
        for candidate in self.candidates.iter().flatten().filter(|c| c.selected) {
            let project = RemoteProject {
                name: candidate.host.alias.clone(),
                ssh: SshOptions {
                    host: candidate.host.alias.clone(),
                    ..SshOptions::default()
                },
                root: root.clone(),
                group: group.clone(),
                ..crate::remote_ui::blank_project()
            };
            let mut checked = project.clone();
            if checked.root == HOME_ROOT {
                checked.root = "/".into();
            }
            checked.validate().map_err(|e| e.to_string())?;
            plan.push_back(project);
        }
        if plan.is_empty() {
            return Err("Choose at least one host.".into());
        }
        Ok(plan)
    }
}

impl App {
    pub(crate) fn open_ssh_import(&mut self) {
        self.hide_center_overlay();
        self.remote_ui.import = Some(HostImport::new());
        self.load_config_hosts();
    }

    /// The main pane while the import page is open.
    pub(crate) fn ssh_import_center(&mut self, ui: &mut egui::Ui) {
        self.fill_import_candidates();
        let mut close = false;
        egui::Frame::NONE.inner_margin(16.0).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.horizontal(|ui| {
                ui.label(RichText::new("Import SSH hosts").size(18.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    close = appearance::sidebar_action(ui, "X", "Close").clicked();
                });
            });
            egui::ScrollArea::vertical()
                .id_salt("ssh-import")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    form::card(ui, |ui| close |= self.ssh_import_form(ui));
                });
        });
        if close {
            self.remote_ui.import = None;
        }
    }

    /// Config aliases no saved project uses, once the config has loaded.
    fn fill_import_candidates(&mut self) {
        let Some(config) = self.remote_ui.hosts.config_hosts.as_deref() else {
            return;
        };
        let Some(import) = self.remote_ui.import.as_mut() else {
            return;
        };
        if import.candidates.is_none() {
            let candidates = quick_aliases(config, &self.state.remote_projects, "")
                .into_iter()
                .map(|host| Candidate {
                    host: host.clone(),
                    selected: false,
                })
                .collect();
            import.candidates = Some(candidates);
        }
    }

    /// Returns whether the page should close.
    fn ssh_import_form(&mut self, ui: &mut egui::Ui) -> bool {
        let managed = self.hosts_supported();
        let groups = self.group_names();
        let Some(import) = self.remote_ui.import.as_mut() else {
            return true;
        };
        if import.running() || import.finished() {
            return import_progress(ui, import);
        }
        form::section(ui, "Hosts in ~/.ssh/config");
        if !import_candidates(ui, import) {
            return false;
        }
        form::section(ui, "Remote path");
        form::segmented(
            ui,
            &mut import.root,
            &[
                (RootChoice::Login, "Login directory"),
                (RootChoice::Path, "Same path for all"),
            ],
        );
        match import.root {
            RootChoice::Login => form::hint(
                ui,
                "Connects to each host once. Hosts that need a password or 2FA fail; add those one by one.",
                "",
            ),
            RootChoice::Path => {
                form::text(ui, "Path", &mut import.path, "/home/you");
            }
        }
        form::section(ui, "Group");
        form::text(ui, "Group (optional)", &mut import.group, "None");
        if !groups.is_empty() {
            form::hint(ui, &format!("In use: {}", groups.join(", ")), "");
        }
        if let Some(error) = &import.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        ui.add_space(12.0);
        let count = import
            .candidates
            .iter()
            .flatten()
            .filter(|c| c.selected)
            .count();
        let mut start = false;
        let mut close = false;
        ui.horizontal(|ui| {
            ui.add_enabled_ui(managed && count > 0, |ui| {
                let label = format!("Import {count} host{}", if count == 1 { "" } else { "s" });
                start = form::primary_button(ui, &label).clicked();
            });
            close = ui.button("Cancel").clicked();
        });
        if !managed {
            form::hint(ui, "Restart the session service to import hosts.", "");
        }
        if start {
            match import.plan() {
                Ok(plan) => {
                    import.error = None;
                    import.queue = plan;
                    self.next_import();
                }
                Err(error) => import.error = Some(error),
            }
        }
        close
    }

    /// Sends the next queued add, if none is in flight.
    fn next_import(&mut self) {
        let Some(import) = self.remote_ui.import.as_mut() else {
            return;
        };
        if import.in_flight.is_some() {
            return;
        }
        let Some(project) = import.queue.pop_front() else {
            return;
        };
        import.in_flight = Some(project.clone());
        let probe = project.root == HOME_ROOT;
        let _ = self.jobs.send(Job::RemoteAdd {
            project,
            probe,
            generation: self.selection_generation,
        });
    }

    /// Whether `project` is the import's add in flight.
    pub(crate) fn import_owns(&self, project: &RemoteProject) -> bool {
        self.remote_ui
            .import
            .as_ref()
            .is_some_and(|i| i.in_flight.as_ref() == Some(project))
    }

    /// Advances the import after one probe or add.
    pub(crate) fn apply_import_result(
        &mut self,
        project: &RemoteProject,
        result: Result<crate::remote_ui::FormResult, crate::remote_ui::Failure>,
    ) {
        let Some(import) = self.remote_ui.import.as_mut() else {
            return;
        };
        import.in_flight = None;
        match result {
            Ok(crate::remote_ui::FormResult::Probe(root)) => {
                let mut added = project.clone();
                added.root = root;
                import.queue.push_front(added);
            }
            Ok(crate::remote_ui::FormResult::Added(state)) => {
                import.added = import.added.saturating_add(1);
                self.apply_state(*state);
            }
            Err(failure) => import.failed.push((project.name.clone(), failure.text)),
        }
        self.next_import();
    }
}

/// Select all/none and one checkbox per alias. Returns whether any exist.
fn import_candidates(ui: &mut egui::Ui, import: &mut HostImport) -> bool {
    let Some(candidates) = import.candidates.as_mut() else {
        ui.add(egui::Spinner::new());
        return false;
    };
    if candidates.is_empty() {
        form::hint(ui, "Every host in ~/.ssh/config is already saved.", "");
        return false;
    }
    ui.horizontal(|ui| {
        for (label, value) in [("Select all", true), ("Select none", false)] {
            if ui.small_button(label).clicked() {
                for candidate in candidates.iter_mut() {
                    candidate.selected = value;
                }
            }
        }
    });
    for candidate in candidates.iter_mut() {
        ui.horizontal(|ui| {
            ui.checkbox(&mut candidate.selected, &candidate.host.alias);
            ui.weak(crate::remote_ui::config_host_summary(&candidate.host));
        });
    }
    true
}

/// Progress while adding, then the result. Returns whether to close.
fn import_progress(ui: &mut egui::Ui, import: &HostImport) -> bool {
    if let Some(project) = &import.in_flight {
        ui.horizontal(|ui| {
            ui.add(egui::Spinner::new());
            ui.label(format!("Adding {}…", project.name));
        });
    }
    if import.added > 0 {
        ui.label(format!(
            "Added {} host{}.",
            import.added,
            if import.added == 1 { "" } else { "s" }
        ));
    }
    for (alias, error) in &import.failed {
        ui.colored_label(ui.visuals().error_fg_color, format!("{alias}: {error}"));
    }
    if !import.failed.is_empty() && !import.running() {
        form::hint(
            ui,
            "Add hosts that failed from the Hosts sidebar: quick connect shows a fix for each error.",
            "",
        );
    }
    ui.add_space(12.0);
    !import.running() && form::primary_button(ui, "Done").clicked()
}
