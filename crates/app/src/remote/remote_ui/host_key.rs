//! SSH host-key trust prompts.
use super::{Failure, HostKeyPrompt, Retry};
use crate::{App, Job, appearance};
use eframe::egui;
use terminator_core::ssh::{HostKey, HostKeyScan, SshFailure, SshOptions};

impl App {
    /// Offers the host-key dialog for a host-key failure in project `id`.
    pub(super) fn prompt_for_failure(&mut self, id: &str, ssh: SshOptions, failure: &Failure) {
        if matches!(
            failure.ssh,
            Some(SshFailure::UnknownHostKey | SshFailure::ChangedHostKey)
        ) && !self.remote_ui.host_key_declined.contains(id)
        {
            self.prompt_host_key(
                ssh,
                Retry::Project(id.to_owned()),
                failure.ssh == Some(SshFailure::ChangedHostKey),
            );
        }
    }
    /// A transfer's classified SSH failure: offer the host-key dialog or ask
    /// for a terminal sign-in.
    pub(crate) fn remote_ssh_failure(&mut self, id: &str, failure: &Failure) {
        let Some(ssh) = self
            .state
            .remote_projects
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.ssh.clone())
        else {
            return;
        };
        self.prompt_for_failure(id, ssh, failure);
        if failure.ssh == Some(SshFailure::AuthFailed) {
            self.remote_ui
                .views
                .entry(id.to_owned())
                .or_default()
                .sign_in = true;
        }
    }
    fn host_keys_supported(&self) -> bool {
        self.connected
            && self
                .state
                .capabilities
                .iter()
                .any(|c| c == terminator_core::ssh::HOST_KEYS_CAPABILITY)
    }
    /// Opens the trust dialog and scans the host's keys off the GUI thread.
    /// One prompt at a time; repeated poll failures do not stack.
    pub(crate) fn prompt_host_key(&mut self, ssh: SshOptions, retry: Retry, changed: bool) {
        if !self.host_keys_supported() || self.remote_ui.host_key.is_some() {
            return;
        }
        self.remote_ui.host_key = Some(HostKeyPrompt {
            ssh: ssh.clone(),
            retry,
            changed,
            scan: None,
            error: None,
            busy: true,
        });
        let _ = self.jobs.send(Job::HostKey { ssh, keys: None });
    }
    pub(crate) fn apply_host_key_scan(
        &mut self,
        ssh: &SshOptions,
        result: Result<HostKeyScan, String>,
    ) {
        let Some(prompt) = self.remote_ui.host_key.as_mut().filter(|p| p.ssh == *ssh) else {
            return;
        };
        prompt.busy = false;
        match result {
            Ok(scan) => {
                prompt.scan = Some(scan);
                prompt.error = None;
            }
            Err(error) => prompt.error = Some(error),
        }
    }
    pub(crate) fn apply_host_key_trust(&mut self, ssh: &SshOptions, result: Result<(), String>) {
        if self
            .remote_ui
            .host_key
            .as_ref()
            .is_none_or(|p| p.ssh != *ssh)
        {
            return;
        }
        if let Err(error) = result {
            if let Some(prompt) = &mut self.remote_ui.host_key {
                prompt.busy = false;
                prompt.error = Some(error);
            }
            return;
        }
        let Some(prompt) = self.remote_ui.host_key.take() else {
            return;
        };
        self.info = Some("SSH host key trusted.".into());
        match prompt.retry {
            Retry::Form {
                project,
                probe,
                generation,
            } => {
                if !self.remote_ui.open || generation != self.selection_generation {
                    return;
                }
                self.remote_ui.busy = true;
                self.remote_ui.error = None;
                self.remote_ui.active = Some((*project).clone());
                let _ = self.jobs.send(Job::RemoteAdd {
                    project: *project,
                    probe,
                    generation,
                });
            }
            Retry::Project(id) => {
                self.remote_ui.host_key_declined.remove(&id);
                if let Some(view) = self.remote_ui.views.get_mut(&id) {
                    view.files_loading = false;
                    view.pending_path = None;
                    view.files_polled = None;
                    view.git_loading = false;
                    view.git_polled = None;
                    view.file_error = None;
                    view.git_error = None;
                }
            }
        }
    }
    fn dismiss_host_key(&mut self) {
        if let Some(prompt) = self.remote_ui.host_key.take()
            && let Retry::Project(id) = prompt.retry
        {
            self.remote_ui.host_key_declined.insert(id);
        }
    }
    /// First-connect trust confirmation. A changed key is only explained;
    /// removing the old key stays a manual, explicit user action.
    pub(crate) fn host_key_dialog(&mut self, ctx: &egui::Context) {
        let Some(prompt) = &self.remote_ui.host_key else {
            return;
        };
        let card = HostKeyCard::new(prompt, appearance::color(&self.theme.status_failed));
        let window = self.popups.window(
            ctx,
            if card.changed {
                "SSH host key changed"
            } else {
                "Trust SSH host?"
            },
        );
        let answer = appearance::confirm(ctx, window, card.dialog(), |ui| card.show(ui));
        match answer {
            Some(appearance::ConfirmAnswer::Destructive) => {
                if let Some(prompt) = &mut self.remote_ui.host_key {
                    prompt.busy = true;
                    prompt.error = None;
                    let _ = self.jobs.send(Job::HostKey {
                        ssh: prompt.ssh.clone(),
                        keys: Some(card.keys),
                    });
                }
            }
            Some(_) => self.dismiss_host_key(),
            None => {}
        }
    }
}

/// Per-frame snapshot of the host-key prompt for rendering.
struct HostKeyCard {
    failed: egui::Color32,
    host: String,
    changed: bool,
    busy: bool,
    error: Option<String>,
    keys: Vec<HostKey>,
    /// Command that removes the old key from known_hosts.
    remove: String,
}

impl HostKeyCard {
    fn new(prompt: &HostKeyPrompt, failed: egui::Color32) -> Self {
        let host = prompt
            .scan
            .as_ref()
            .map_or_else(|| prompt.ssh.host.clone(), |scan| scan.host.clone());
        let keys: Vec<HostKey> = prompt
            .scan
            .as_ref()
            .map(|s| s.keys.clone())
            .unwrap_or_default();
        let remove = format!(
            "ssh-keygen -R {}{}",
            terminator_core::quote(&host),
            prompt
                .ssh
                .known_hosts_file
                .as_ref()
                .map(|path| format!(" -f {}", terminator_core::quote(&path.to_string_lossy())))
                .unwrap_or_default()
        );
        Self {
            failed,
            host,
            changed: prompt.changed,
            busy: prompt.busy,
            error: prompt.error.clone(),
            keys,
            remove,
        }
    }

    /// The prompt's text above the buttons.
    fn show(&self, ui: &mut egui::Ui) {
        self.intro(ui);
        ui.add_space(4.0);
        self.fingerprints(ui);
        if self.changed {
            ui.add_space(4.0);
            ui.label("If you trust the change, remove the old key in a terminal, then reconnect:");
            ui.label(egui::RichText::new(&self.remove).monospace());
        }
        if let Some(error) = &self.error {
            ui.colored_label(self.failed, error);
        }
    }

    /// A changed key only closes. Trust is a risk, so it is never the
    /// Enter default: Enter cancels.
    fn dialog(&self) -> appearance::Confirm<'static> {
        if self.changed {
            return appearance::Confirm::new("", "")
                .cancel(appearance::ConfirmButton::new("Close"));
        }
        let ready = !self.busy && !self.keys.is_empty();
        appearance::Confirm::new("", "").destructive(
            appearance::ConfirmButton::new("Trust and connect")
                .id("ssh-hostkey-trust")
                .disabled((!ready).then_some("Reading host keys…")),
        )
    }

    fn intro(&self, ui: &mut egui::Ui) {
        let host = &self.host;
        if self.changed {
            ui.label(
                egui::RichText::new(format!(
                    "The host key for {host} does not match the one in known_hosts."
                ))
                .strong()
                .color(self.failed),
            );
            ui.label(
                egui::RichText::new(
                    "Someone may be intercepting the connection, or the host was reinstalled. Terminator will not replace the key.",
                )
                .color(self.failed),
            );
        } else {
            ui.label(format!("First connection to {host}."));
            ui.weak("Compare these fingerprints with ones from the host's administrator before trusting it.");
        }
    }

    fn fingerprints(&self, ui: &mut egui::Ui) {
        if self.busy && self.keys.is_empty() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Reading host keys…");
            });
        }
        if self.keys.is_empty() {
            return;
        }
        if self.changed {
            ui.label("The host now presents:");
        }
        for key in &self.keys {
            ui.label(
                egui::RichText::new(format!("{}  {}", key.key_type, key.fingerprint)).monospace(),
            );
        }
    }
}
