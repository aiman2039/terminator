//! Add SSH hosts on the SSH page in three steps: Choose (one host field and
//! the `~/.ssh/config` aliases), Connect (a checklist: config, host key,
//! sign-in, host check) and Ready (tools found, folder, and the OSC 7 lines
//! that let the Explorer follow the terminal). Hosts run one at a time.
//!
//! Nothing is installed or written on a host. Prompts for passwords and
//! codes show in the page only with `ssh-askpass-v1`; answers go to the
//! daemon's sign-in and are never kept. Saved hosts are edited in
//! Properties, not here.
mod choose;
mod connect;
mod flow;
pub(crate) mod osc7;
mod ready;
#[cfg(test)]
mod tests;

use crate::{App, Job};
use std::time::Instant;
use terminator_core::{
    remote::{RemoteProject, host_check::HostTools},
    ssh::{
        ConfigHost, HostKeyScan, SshOptions,
        sign_in::{Secret, SignInPrompt},
    },
};

/// One request of a sign-in with prompts in the GUI.
#[derive(Clone, Debug)]
pub(crate) enum SignInStep {
    Run,
    Poll,
    Answer { id: u64, answer: Option<Secret> },
}

impl SignInStep {
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::Poll => "poll",
            Self::Answer { .. } => "answer",
        }
    }

    /// The update owed when the job itself failed.
    pub(crate) fn failed(self, error: String) -> SignInUpdate {
        match self {
            Self::Run => SignInUpdate::Done(Err(error.into())),
            Self::Poll => SignInUpdate::Prompt(None),
            Self::Answer { .. } => SignInUpdate::Answered(Err(error)),
        }
    }
}

pub(crate) enum SignInUpdate {
    Done(Result<(), crate::remote_ui::Failure>),
    Prompt(Option<SignInPrompt>),
    Answered(Result<(), String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stage {
    Choose,
    Connect,
    Ready,
}

/// Where one host is in the flow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// Login-directory probe (connects, checks tools).
    Probing,
    /// Unknown host key: the fingerprints wait for Trust.
    HostKey,
    /// The key in `known_hosts` differs. Never trusted from here.
    ChangedKey,
    /// The daemon's sign-in runs; prompts show in the page.
    SigningIn,
    /// Connected and checked: the Ready step.
    Checked,
    /// Probing the chosen folder, then adding.
    Saving,
    Added(String),
    Failed,
    Skipped,
}

pub(crate) struct Candidate {
    pub(crate) host: ConfigHost,
    pub(crate) selected: bool,
}

/// One host being added.
pub(crate) struct HostRun {
    pub(crate) project: RemoteProject,
    /// `user@host · via jump` for the checklist.
    pub(crate) summary: String,
    pub(crate) phase: Phase,
    pub(crate) error: Option<String>,
    /// The host key was accepted (trusted now or already known).
    pub(crate) key_ok: bool,
    /// Sign-in worked (keys, agent, or the prompts here).
    pub(crate) auth_ok: bool,
    pub(crate) keys: Option<HostKeyScan>,
    pub(crate) busy: bool,
    pub(crate) prompt: Option<SignInPrompt>,
    /// Text typed for `prompt`. Cleared when sent.
    pub(crate) answer: String,
    pub(crate) polled: Option<Instant>,
    pub(crate) tools: Option<HostTools>,
    pub(crate) login_root: String,
    pub(crate) folder: String,
    pub(crate) osc7: osc7::Shell,
    /// Open a terminal on this host when the flow ends.
    pub(crate) open_terminal: bool,
}

impl HostRun {
    pub(crate) fn new(project: RemoteProject, summary: String) -> Self {
        Self {
            project,
            summary,
            phase: Phase::Probing,
            error: None,
            key_ok: false,
            auth_ok: false,
            keys: None,
            busy: false,
            prompt: None,
            answer: String::new(),
            polled: None,
            tools: None,
            login_root: String::new(),
            folder: String::new(),
            osc7: osc7::Shell::Zsh,
            open_terminal: false,
        }
    }

    pub(crate) fn ssh(&self) -> &SshOptions {
        &self.project.ssh
    }
}

/// The add-hosts flow while it shows on the SSH page.
pub(crate) struct Onboard {
    pub(crate) stage: Stage,
    /// Typed `user@host:port` or alias.
    pub(crate) address: String,
    /// Config aliases no saved host uses; `None` while loading.
    pub(crate) candidates: Option<Vec<Candidate>>,
    /// Advanced fields shared by the chosen hosts.
    pub(crate) template: RemoteProject,
    pub(crate) runs: Vec<HostRun>,
    pub(crate) current: usize,
    pub(crate) error: Option<String>,
    /// Generation the probes and adds belong to.
    pub(crate) generation: u64,
}

impl Onboard {
    pub(crate) fn new(address: String, generation: u64) -> Self {
        Self {
            stage: Stage::Choose,
            address,
            candidates: None,
            template: crate::remote_ui::blank_project(),
            runs: Vec::new(),
            current: 0,
            error: None,
            generation,
        }
    }

    pub(crate) fn run(&self) -> Option<&HostRun> {
        self.runs.get(self.current)
    }

    pub(crate) fn run_mut(&mut self) -> Option<&mut HostRun> {
        self.runs.get_mut(self.current)
    }

    /// The running host whose connection is `ssh`.
    pub(crate) fn run_for(&mut self, ssh: &SshOptions) -> Option<&mut HostRun> {
        self.runs.get_mut(self.current).filter(|r| r.ssh() == ssh)
    }

    pub(crate) fn selected(&self) -> usize {
        self.candidates
            .iter()
            .flatten()
            .filter(|c| c.selected)
            .count()
    }

    /// Hosts Connect would add: the ticked aliases plus a typed host.
    pub(crate) fn host_count(&self) -> usize {
        self.selected()
            .saturating_add(usize::from(!self.address.trim().is_empty()))
    }
}

impl App {
    /// Opens the add-hosts flow on the SSH page, with `address` typed in.
    pub(crate) fn open_add_hosts(&mut self, address: String) {
        if self.keep_changed_ssh_form() {
            return;
        }
        self.show_ssh_page();
        self.remote_ui.onboard = Some(Onboard::new(address, self.selection_generation));
        self.load_config_hosts();
    }

    pub(crate) fn sign_in_supported(&self) -> bool {
        self.connected
            && self
                .state
                .capabilities
                .iter()
                .any(|c| c == terminator_core::ssh::sign_in::SIGN_IN_CAPABILITY)
    }

    /// The SSH page body while the flow shows.
    pub(crate) fn onboard_body(&mut self, ui: &mut eframe::egui::Ui) {
        self.fill_onboard_candidates();
        let Some(stage) = self.remote_ui.onboard.as_ref().map(|o| o.stage) else {
            return;
        };
        eframe::egui::ScrollArea::vertical()
            .id_salt("ssh-add-hosts")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                crate::vnc_ui::form::column(ui, |ui| match stage {
                    Stage::Choose => self.onboard_choose(ui),
                    Stage::Connect => self.onboard_connect(ui),
                    Stage::Ready => self.onboard_ready(ui),
                });
            });
        self.poll_sign_in(ui.ctx());
    }

    /// Config aliases no saved host uses, once the config has loaded.
    fn fill_onboard_candidates(&mut self) {
        let Some(config) = self.remote_ui.hosts.config_hosts.as_deref() else {
            return;
        };
        let Some(onboard) = self.remote_ui.onboard.as_mut() else {
            return;
        };
        if onboard.candidates.is_none() {
            let candidates =
                crate::hosts_ui::unsaved_aliases(config, &self.state.remote_projects, "")
                    .into_iter()
                    .map(|host| Candidate {
                        host: host.clone(),
                        selected: false,
                    })
                    .collect();
            onboard.candidates = Some(candidates);
        }
    }

    /// Asks the daemon for the waiting prompt, at most every 300 ms, while
    /// a sign-in runs.
    fn poll_sign_in(&mut self, ctx: &eframe::egui::Context) {
        const EVERY: std::time::Duration = std::time::Duration::from_millis(300);
        let Some(run) = self
            .remote_ui
            .onboard
            .as_mut()
            .and_then(Onboard::run_mut)
            .filter(|r| r.phase == Phase::SigningIn)
        else {
            return;
        };
        ctx.request_repaint_after(EVERY);
        if run.polled.is_some_and(|at| at.elapsed() < EVERY) {
            return;
        }
        run.polled = Some(Instant::now());
        let ssh = run.ssh().clone();
        let _ = self.jobs.send(Job::SshSignIn(ssh, SignInStep::Poll));
    }

    pub(crate) fn close_onboard(&mut self) {
        let Some(onboard) = self.remote_ui.onboard.take() else {
            return;
        };
        // A prompt still waiting is cancelled, so OpenSSH stops.
        if let Some(run) = onboard.run()
            && let Some(prompt) = &run.prompt
        {
            let _ = self.jobs.send(Job::SshSignIn(
                run.ssh().clone(),
                SignInStep::Answer {
                    id: prompt.id,
                    answer: None,
                },
            ));
        }
    }
}
