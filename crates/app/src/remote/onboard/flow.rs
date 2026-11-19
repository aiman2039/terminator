//! State changes of the add-hosts flow: start, probe, host key, sign-in,
//! add, and moving to the next host.
use super::{HostRun, Onboard, Phase, SignInStep, SignInUpdate, Stage, osc7};
use crate::remote_ui::{Failure, FormResult};
use crate::{App, Job};
use terminator_core::{
    remote::{HOME_ROOT, RemoteProject},
    ssh::{HostKeyScan, SshFailure, SshOptions, sign_in::Secret},
};

impl App {
    /// Builds one run per chosen host and starts the first.
    pub(super) fn start_onboard(&mut self) {
        let Some(onboard) = self.remote_ui.onboard.as_mut() else {
            return;
        };
        match plan(onboard) {
            Ok(runs) => {
                onboard.error = None;
                onboard.runs = runs;
                onboard.current = 0;
                onboard.stage = Stage::Connect;
                self.probe_current(None);
            }
            Err(error) => onboard.error = Some(error),
        }
    }

    /// Probes the current host at `root`, or its login directory.
    pub(super) fn probe_current(&mut self, root: Option<String>) {
        let Some(onboard) = self.remote_ui.onboard.as_mut() else {
            return;
        };
        let generation = onboard.generation;
        let Some(run) = onboard.run_mut() else {
            return;
        };
        run.phase = if root.is_some() {
            Phase::Saving
        } else {
            Phase::Probing
        };
        run.error = None;
        run.busy = true;
        let mut project = run.project.clone();
        project.root = root.unwrap_or_else(|| HOME_ROOT.to_owned());
        run.project.root.clone_from(&project.root);
        let _ = self.jobs.send(Job::RemoteAdd {
            project,
            probe: true,
            generation,
        });
    }

    /// Whether a probe or add result belongs to the flow.
    pub(crate) fn onboard_owns(&self, project: &RemoteProject) -> bool {
        self.remote_ui
            .onboard
            .as_ref()
            .and_then(Onboard::run)
            .is_some_and(|r| {
                r.busy && r.project.ssh == project.ssh && r.project.root == project.root
            })
    }

    pub(crate) fn apply_onboard_result(
        &mut self,
        project: &RemoteProject,
        result: Result<FormResult, Failure>,
    ) {
        let generation = self.remote_ui.onboard.as_ref().map_or(0, |o| o.generation);
        let Some(run) = self.remote_ui.onboard.as_mut().and_then(Onboard::run_mut) else {
            return;
        };
        run.busy = false;
        match result {
            Ok(FormResult::Probe(root, _)) if run.phase == Phase::Saving => {
                run.project.root = root;
                run.busy = true;
                let mut adding = run.project.clone();
                let trimmed = adding.name.trim().to_owned();
                adding.name = trimmed;
                if adding.name.is_empty() {
                    adding.name.clone_from(&adding.ssh.host);
                }
                let _ = self.jobs.send(Job::RemoteAdd {
                    project: adding,
                    probe: false,
                    generation,
                });
            }
            Ok(FormResult::Probe(root, tools)) => {
                run.key_ok = true;
                run.auth_ok = true;
                run.osc7 = tools
                    .as_ref()
                    .map_or(osc7::Shell::Zsh, |t| osc7::Shell::from_name(&t.shell));
                run.tools = tools;
                run.folder.clone_from(&root);
                run.login_root = root;
                run.phase = Phase::Checked;
                if let Some(onboard) = self.remote_ui.onboard.as_mut() {
                    onboard.stage = Stage::Ready;
                }
            }
            Ok(FormResult::Added(state)) => {
                let id = state
                    .remote_projects
                    .iter()
                    .find(|p| p.ssh == project.ssh && p.root == project.root)
                    .map(|p| p.id.clone())
                    .unwrap_or_default();
                run.phase = Phase::Added(id);
                self.apply_state(*state);
                self.next_onboard_host();
            }
            Err(failure) => self.onboard_failed(failure),
        }
    }

    fn onboard_failed(&mut self, failure: Failure) {
        let sign_in = self.sign_in_supported();
        let Some(run) = self.remote_ui.onboard.as_mut().and_then(Onboard::run_mut) else {
            return;
        };
        let saving = run.phase == Phase::Saving;
        run.error = Some(failure.text.clone());
        let ssh = run.ssh().clone();
        match failure.ssh {
            Some(SshFailure::UnknownHostKey) => {
                run.phase = Phase::HostKey;
                run.busy = true;
                run.keys = None;
                let _ = self.jobs.send(Job::HostKey { ssh, keys: None });
            }
            Some(SshFailure::ChangedHostKey) => run.phase = Phase::ChangedKey,
            Some(SshFailure::AuthFailed) if sign_in && !run.auth_ok => {
                run.key_ok = true;
                run.phase = Phase::SigningIn;
                run.error = None;
                run.prompt = None;
                run.polled = None;
                let _ = self.jobs.send(Job::SshSignIn(ssh, SignInStep::Run));
            }
            // A bad folder keeps the Ready step so the path can change.
            _ if saving => run.phase = Phase::Checked,
            _ => run.phase = Phase::Failed,
        }
    }

    /// Host-key scan for the flow; other callers are unaffected.
    pub(crate) fn onboard_host_key_scan(
        &mut self,
        ssh: &SshOptions,
        result: &Result<HostKeyScan, String>,
    ) {
        let Some(run) = self.remote_ui.onboard.as_mut().and_then(|o| o.run_for(ssh)) else {
            return;
        };
        if run.phase != Phase::HostKey {
            return;
        }
        run.busy = false;
        match result {
            Ok(scan) => {
                run.keys = Some(scan.clone());
                run.error = None;
            }
            Err(error) => run.error = Some(error.clone()),
        }
    }

    pub(super) fn trust_onboard_key(&mut self) {
        let Some(run) = self.remote_ui.onboard.as_mut().and_then(Onboard::run_mut) else {
            return;
        };
        let Some(keys) = run.keys.as_ref().map(|s| s.keys.clone()) else {
            return;
        };
        run.busy = true;
        let ssh = run.ssh().clone();
        let _ = self.jobs.send(Job::HostKey {
            ssh,
            keys: Some(keys),
        });
    }

    pub(crate) fn onboard_host_key_trusted(
        &mut self,
        ssh: &SshOptions,
        result: &Result<(), String>,
    ) {
        let Some(run) = self.remote_ui.onboard.as_mut().and_then(|o| o.run_for(ssh)) else {
            return;
        };
        if run.phase != Phase::HostKey {
            return;
        }
        run.busy = false;
        match result {
            Ok(()) => {
                run.key_ok = true;
                self.probe_current(None);
            }
            Err(error) => run.error = Some(error.clone()),
        }
    }

    pub(crate) fn apply_sign_in(&mut self, ssh: &SshOptions, update: SignInUpdate) {
        let Some(run) = self.remote_ui.onboard.as_mut().and_then(|o| o.run_for(ssh)) else {
            return;
        };
        if run.phase != Phase::SigningIn {
            return;
        }
        match update {
            SignInUpdate::Prompt(prompt) => {
                if prompt.as_ref().map(|p| p.id) != run.prompt.as_ref().map(|p| p.id) {
                    run.answer.clear();
                }
                run.prompt = prompt;
            }
            SignInUpdate::Answered(Err(error)) => run.error = Some(error),
            SignInUpdate::Answered(Ok(())) => {}
            SignInUpdate::Done(Ok(())) => {
                run.auth_ok = true;
                run.prompt = None;
                self.probe_current(None);
            }
            SignInUpdate::Done(Err(failure)) => {
                run.prompt = None;
                run.phase = Phase::Failed;
                run.error = Some(match failure.ssh {
                    Some(SshFailure::AuthFailed) => {
                        "Sign-in failed. Check the password or code, then retry.".into()
                    }
                    _ => failure.text,
                });
            }
        }
    }

    /// Sends the typed answer to the waiting prompt; `cancel` sends none.
    pub(super) fn answer_onboard_prompt(&mut self, cancel: bool) {
        let Some(run) = self.remote_ui.onboard.as_mut().and_then(Onboard::run_mut) else {
            return;
        };
        let Some(prompt) = run.prompt.take() else {
            return;
        };
        let answer = (!cancel).then(|| Secret::new(std::mem::take(&mut run.answer)));
        run.answer.clear();
        let ssh = run.ssh().clone();
        let _ = self.jobs.send(Job::SshSignIn(
            ssh,
            SignInStep::Answer {
                id: prompt.id,
                answer,
            },
        ));
    }

    /// Ready step: add the host at the chosen folder.
    pub(super) fn add_onboard_host(&mut self, open_terminal: bool) {
        let Some(run) = self.remote_ui.onboard.as_mut().and_then(Onboard::run_mut) else {
            return;
        };
        run.open_terminal = open_terminal;
        let folder = run.folder.trim().to_owned();
        let folder = if folder.is_empty() {
            run.login_root.clone()
        } else {
            folder
        };
        self.probe_current(Some(folder));
    }

    pub(super) fn skip_onboard_host(&mut self) {
        if let Some(run) = self.remote_ui.onboard.as_mut().and_then(Onboard::run_mut) {
            run.phase = Phase::Skipped;
            run.busy = false;
        }
        self.close_onboard_prompt();
        self.next_onboard_host();
    }

    /// Cancels a prompt the current host still waits on.
    fn close_onboard_prompt(&mut self) {
        let Some(run) = self.remote_ui.onboard.as_mut().and_then(Onboard::run_mut) else {
            return;
        };
        if run.prompt.is_some() {
            self.answer_onboard_prompt(true);
        }
    }

    /// Starts the next host, or ends the flow.
    fn next_onboard_host(&mut self) {
        let Some(onboard) = self.remote_ui.onboard.as_mut() else {
            return;
        };
        let next = onboard.current.saturating_add(1);
        if next < onboard.runs.len() {
            onboard.current = next;
            onboard.stage = Stage::Connect;
            self.probe_current(None);
            return;
        }
        let open: Vec<String> = onboard
            .runs
            .iter()
            .filter(|r| r.open_terminal)
            .filter_map(|r| match &r.phase {
                Phase::Added(id) if !id.is_empty() => Some(id.clone()),
                _ => None,
            })
            .collect();
        let added = onboard
            .runs
            .iter()
            .filter(|r| matches!(r.phase, Phase::Added(_)))
            .count();
        self.remote_ui.onboard = None;
        self.info = Some(match added {
            0 => "No SSH host added.".into(),
            1 => "SSH host added.".into(),
            n => format!("{n} SSH hosts added."),
        });
        if open.is_empty() {
            self.hide_center_overlay();
        }
        for id in open {
            self.open_host_terminal(id);
        }
    }

    /// Retry the current host from the start.
    pub(super) fn retry_onboard_host(&mut self) {
        if let Some(run) = self.remote_ui.onboard.as_mut().and_then(Onboard::run_mut) {
            run.auth_ok = false;
        }
        self.probe_current(None);
    }
}

/// One run per chosen host: the typed address first, then the ticked
/// aliases. Advanced fields apply to all; name, user, port and key only
/// to a single host.
pub(super) fn plan(onboard: &Onboard) -> Result<Vec<HostRun>, String> {
    let mut hosts: Vec<(SshOptions, String)> = Vec::new();
    let typed = onboard.address.trim();
    if !typed.is_empty() {
        let ssh = crate::hosts_ui::parse_target(typed).ok_or_else(|| {
            "Enter a host name or config alias, user@host or user@host:port.".to_owned()
        })?;
        let summary = onboard
            .candidates
            .iter()
            .flatten()
            .find(|c| c.host.alias == ssh.host)
            .map_or_else(
                || ssh.authority(),
                |c| crate::remote_ui::config_host_summary(&c.host),
            );
        hosts.push((ssh, summary));
    }
    for candidate in onboard.candidates.iter().flatten().filter(|c| c.selected) {
        if hosts
            .iter()
            .any(|(ssh, _)| ssh.host == candidate.host.alias)
        {
            continue;
        }
        let ssh = SshOptions {
            host: candidate.host.alias.clone(),
            ..SshOptions::default()
        };
        hosts.push((ssh, crate::remote_ui::config_host_summary(&candidate.host)));
    }
    if hosts.is_empty() {
        return Err("Type a host or choose one from ~/.ssh/config.".into());
    }
    let single = hosts.len() == 1;
    let template = &onboard.template;
    let group = terminator_core::remote::normalize_group(template.group.as_deref())
        .map_err(|e| e.to_string())?;
    hosts
        .into_iter()
        .map(|(mut ssh, summary)| {
            if single {
                if !template.ssh.username.trim().is_empty() {
                    template.ssh.username.trim().clone_into(&mut ssh.username);
                }
                ssh.port = template.ssh.port.or(ssh.port);
                ssh.identity_file.clone_from(&template.ssh.identity_file);
            }
            ssh.validate().map_err(|e| e.to_string())?;
            let name = if single && !template.name.trim().is_empty() {
                template.name.trim().to_owned()
            } else {
                ssh.host.clone()
            };
            let project = RemoteProject {
                name,
                ssh,
                group: group.clone(),
                command_timeout_seconds: template.command_timeout_seconds,
                shell_command: template.shell_command.clone(),
                ..crate::remote_ui::blank_project()
            };
            Ok(HostRun::new(project, summary))
        })
        .collect()
}
