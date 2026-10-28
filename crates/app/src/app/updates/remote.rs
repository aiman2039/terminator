//! SSH remote project results.

use std::path::PathBuf;

use crate::*;
use terminator_core::remote::RemoteProject;
use terminator_core::ssh::SshFailure;

impl App {
    pub(super) fn handle_remote_added(
        &mut self,
        project: RemoteProject,
        generation: u64,
        result: Result<crate::remote_ui::FormResult, crate::remote_ui::Failure>,
    ) {
        if self.remote_ui.hosts.quick.as_ref() == Some(&project) {
            self.apply_quick_connect(&project, generation, result);
            return;
        }
        if self.remote_ui.active.as_ref() != Some(&project) {
            return;
        }
        self.remote_ui.busy = false;
        self.remote_ui.active = None;
        match result {
            Ok(crate::remote_ui::FormResult::Probe(root)) => self.remote_probed(&project, root),
            Ok(crate::remote_ui::FormResult::Added(state)) => {
                self.remote_added(&project, generation, *state);
            }
            Err(failure) => self.remote_form_failed(project, generation, failure),
        }
    }

    fn remote_probed(&mut self, project: &RemoteProject, root: String) {
        if !self
            .remote_ui
            .draft
            .as_ref()
            .is_some_and(|p| p.ssh == project.ssh && p.root == project.root)
        {
            return;
        }
        if let Some(draft) = &mut self.remote_ui.draft {
            draft.root = root;
        }
        self.remote_ui.error = None;
        self.info = Some("SSH connection verified.".into());
    }

    fn remote_added(&mut self, project: &RemoteProject, generation: u64, state: State) {
        let id = state
            .remote_projects
            .iter()
            .find(|p| p.ssh == project.ssh && p.root == project.root)
            .map(|p| p.id.clone());
        self.apply_state(state);
        self.remote_ui.open = false;
        if self.selection_generation == generation
            && let Some(id) = id
        {
            self.reveal_project(id);
        }
    }

    fn remote_form_failed(
        &mut self,
        project: RemoteProject,
        generation: u64,
        failure: crate::remote_ui::Failure,
    ) {
        if matches!(
            failure.ssh,
            Some(SshFailure::UnknownHostKey | SshFailure::ChangedHostKey)
        ) {
            let changed = failure.ssh == Some(SshFailure::ChangedHostKey);
            // Only the probe connects; adding never does.
            self.prompt_host_key(
                project.ssh.clone(),
                crate::remote_ui::Retry::Form {
                    project: Box::new(project),
                    probe: true,
                    generation,
                },
                changed,
            );
        }
        self.remote_ui.error = Some(failure.text);
    }

    pub(super) fn handle_ssh_config_hosts(
        &mut self,
        generation: u64,
        hosts: Vec<terminator_core::ssh::ConfigHost>,
    ) {
        if generation == self.remote_ui.hosts.config_generation {
            self.remote_ui.hosts.config_hosts = Some(hosts.clone());
        }
        if generation == self.remote_ui.config_generation {
            self.remote_ui.config_hosts = Some(hosts);
        }
    }

    pub(super) fn handle_remote_asset(&mut self, project: String, path: PathBuf, after: After) {
        self.images.remove(&path);
        let tab = if crate::image_preview::supported(&path) {
            Tab::Image { path }
        } else if crate::browser::supported_file(&path) {
            Tab::browser_file(path)
        } else if crate::player::supported(&path) {
            self.open_audio(&project, path, None);
            return;
        } else {
            Tab::NativeEditor { path }
        };
        self.place_gui_tab(project, tab, after);
    }
}
