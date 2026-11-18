//! Add-hosts flow: plan, probe, host key, sign-in prompts, add.
use super::*;
use crate::Update;
use crate::app::tests::support::{drain_updates, fixture, painted_strings};
use crate::remote_ui::{Failure, FormResult};
use eframe::egui;
use std::sync::mpsc;
use terminator_core::remote::host_check::{HostTool, HostTools};
use terminator_core::ssh::{SshError, SshFailure};

/// An app whose jobs land in the returned receiver instead of a worker.
fn app_with_jobs() -> (App, egui::Context, tempfile::TempDir, mpsc::Receiver<Job>) {
    let (mut app, ctx, dir) = fixture();
    let (jobs, requests) = mpsc::channel();
    app.jobs = jobs.into();
    app.connected = true;
    app.state.capabilities = vec![
        terminator_core::remote::CAPABILITY.into(),
        terminator_core::remote::HOSTS_CAPABILITY.into(),
        terminator_core::ssh::HOST_KEYS_CAPABILITY.into(),
    ];
    (app, ctx, dir, requests)
}

fn alias(name: &str) -> Candidate {
    Candidate {
        host: terminator_core::ssh::ConfigHost {
            alias: name.into(),
            hostname: format!("{name}.example.com"),
            ..Default::default()
        },
        selected: true,
    }
}

/// The probe the flow sent last.
fn last_probe(requests: &mpsc::Receiver<Job>) -> RemoteProject {
    requests
        .try_iter()
        .filter_map(|job| match job {
            Job::RemoteAdd { project, probe, .. } if probe => Some(project),
            _ => None,
        })
        .last()
        .expect("a probe")
}

fn reply(
    app: &mut App,
    ctx: &egui::Context,
    project: RemoteProject,
    result: Result<FormResult, Failure>,
) {
    let generation = app.remote_ui.onboard.as_ref().unwrap().generation;
    app.update_tx
        .send(Update::RemoteAdded(project, generation, result))
        .unwrap();
    drain_updates(app, ctx);
}

fn ssh_failure(failure: SshFailure) -> Failure {
    Failure::from_error(&SshError {
        failure,
        message: String::new(),
    })
}

fn phase(app: &App) -> Phase {
    app.remote_ui
        .onboard
        .as_ref()
        .unwrap()
        .run()
        .unwrap()
        .phase
        .clone()
}

#[test]
fn typed_host_and_ticked_aliases_each_get_one_run() {
    let mut onboard = Onboard::new("me@build:2222".into(), 0);
    onboard.candidates = Some(vec![alias("gpu"), alias("build")]);
    onboard.candidates.as_mut().unwrap()[1].selected = false;
    onboard.template.name = "ignored for several".into();
    onboard.template.group = Some("lab".into());
    let runs = flow::plan(&onboard).unwrap();
    let hosts: Vec<_> = runs.iter().map(|r| r.project.ssh.authority()).collect();
    assert_eq!(hosts, ["me@build:2222", "gpu"]);
    assert_eq!(runs[0].project.name, "build");
    assert!(
        runs.iter()
            .all(|r| r.project.group.as_deref() == Some("lab"))
    );
}

#[test]
fn name_user_and_port_apply_to_a_single_host_only() {
    let mut onboard = Onboard::new(String::new(), 0);
    onboard.candidates = Some(vec![alias("gpu")]);
    onboard.template.name = "GPU box".into();
    onboard.template.ssh.username = "me".into();
    onboard.template.ssh.port = Some(2200);
    let runs = flow::plan(&onboard).unwrap();
    assert_eq!(runs[0].project.name, "GPU box");
    assert_eq!(runs[0].project.ssh.authority(), "me@gpu:2200");
    assert!(flow::plan(&Onboard::new(String::new(), 0)).is_err());
    assert!(flow::plan(&Onboard::new("bad host!".into(), 0)).is_err());
}

#[test]
fn probe_success_shows_the_ready_step_with_tools_and_the_login_folder() {
    let (mut app, ctx, _dir, requests) = app_with_jobs();
    app.open_add_hosts("me@box".into());
    app.start_onboard();
    let probing = last_probe(&requests);
    assert_eq!(probing.root, terminator_core::remote::HOME_ROOT);
    let tools = HostTools {
        shell: "bash".into(),
        tools: vec![HostTool {
            name: "mosh-server".into(),
            version: "mosh 1.4.0".into(),
            found: true,
        }],
    };
    reply(
        &mut app,
        &ctx,
        probing,
        Ok(FormResult::Probe("/home/me".into(), Some(tools))),
    );
    let onboard = app.remote_ui.onboard.as_ref().unwrap();
    assert_eq!(onboard.stage, Stage::Ready);
    let run = onboard.run().unwrap();
    assert_eq!(run.folder, "/home/me");
    assert_eq!(run.osc7, osc7::Shell::Bash);
    assert!(run.key_ok && run.auth_ok);
}

#[test]
fn an_unknown_host_key_is_trusted_in_the_checklist_then_probed_again() {
    let (mut app, ctx, _dir, requests) = app_with_jobs();
    app.open_add_hosts("box".into());
    app.start_onboard();
    let probing = last_probe(&requests);
    reply(
        &mut app,
        &ctx,
        probing,
        Err(ssh_failure(SshFailure::UnknownHostKey)),
    );
    assert_eq!(phase(&app), Phase::HostKey);
    let ssh = app
        .remote_ui
        .onboard
        .as_ref()
        .unwrap()
        .run()
        .unwrap()
        .ssh()
        .clone();
    let scan = terminator_core::ssh::HostKeyScan {
        host: "box".into(),
        keys: vec![terminator_core::ssh::HostKey {
            key_type: "ssh-ed25519".into(),
            key: "AAAA".into(),
            fingerprint: "SHA256:abc".into(),
        }],
    };
    app.update_tx
        .send(Update::HostKeyScanned(ssh.clone(), Ok(scan)))
        .unwrap();
    drain_updates(&mut app, &ctx);
    // The modal host-key dialog stays closed: the checklist asks.
    assert!(app.remote_ui.host_key.is_none());
    app.trust_onboard_key();
    assert!(
        requests
            .try_iter()
            .any(|job| matches!(job, Job::HostKey { keys: Some(_), .. }))
    );
    app.update_tx
        .send(Update::HostKeyTrusted(ssh, Ok(())))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert_eq!(phase(&app), Phase::Probing);
    assert!(
        app.remote_ui
            .onboard
            .as_ref()
            .unwrap()
            .run()
            .unwrap()
            .key_ok
    );
}

#[test]
fn a_password_prompt_shows_in_the_page_and_the_answer_is_sent_once() {
    let (mut app, ctx, _dir, requests) = app_with_jobs();
    app.state
        .capabilities
        .push(terminator_core::ssh::sign_in::SIGN_IN_CAPABILITY.into());
    app.open_add_hosts("box".into());
    app.start_onboard();
    let probing = last_probe(&requests);
    reply(
        &mut app,
        &ctx,
        probing,
        Err(ssh_failure(SshFailure::AuthFailed)),
    );
    assert_eq!(phase(&app), Phase::SigningIn);
    assert!(
        requests
            .try_iter()
            .any(|job| matches!(job, Job::SshSignIn(_, SignInStep::Run)))
    );
    let ssh = app
        .remote_ui
        .onboard
        .as_ref()
        .unwrap()
        .run()
        .unwrap()
        .ssh()
        .clone();
    let prompt = SignInPrompt {
        id: 7,
        text: "Password:".into(),
    };
    app.apply_sign_in(&ssh, SignInUpdate::Prompt(Some(prompt)));
    app.remote_ui
        .onboard
        .as_mut()
        .unwrap()
        .run_mut()
        .unwrap()
        .answer = "pw".into();
    app.answer_onboard_prompt(false);
    let sent = requests.try_iter().find_map(|job| match job {
        Job::SshSignIn(_, SignInStep::Answer { id, answer }) => Some((id, answer)),
        _ => None,
    });
    let (id, answer) = sent.expect("an answer");
    assert_eq!(id, 7);
    assert_eq!(answer.unwrap().expose(), "pw");
    let run = app.remote_ui.onboard.as_ref().unwrap().run().unwrap();
    assert!(run.answer.is_empty() && run.prompt.is_none());
    app.apply_sign_in(&ssh, SignInUpdate::Done(Ok(())));
    assert_eq!(phase(&app), Phase::Probing);
}

#[test]
fn without_in_app_sign_in_a_password_host_fails_with_its_reason() {
    let (mut app, ctx, _dir, requests) = app_with_jobs();
    app.open_add_hosts("box".into());
    app.start_onboard();
    let probing = last_probe(&requests);
    reply(
        &mut app,
        &ctx,
        probing,
        Err(ssh_failure(SshFailure::AuthFailed)),
    );
    assert_eq!(phase(&app), Phase::Failed);
    assert!(
        !requests
            .try_iter()
            .any(|job| matches!(job, Job::SshSignIn(..)))
    );
}

#[test]
fn adding_probes_the_chosen_folder_then_adds_and_closes() {
    let (mut app, ctx, _dir, requests) = app_with_jobs();
    app.open_add_hosts("box".into());
    app.start_onboard();
    let probing = last_probe(&requests);
    reply(
        &mut app,
        &ctx,
        probing,
        Ok(FormResult::Probe("/home/me".into(), None)),
    );
    app.remote_ui
        .onboard
        .as_mut()
        .unwrap()
        .run_mut()
        .unwrap()
        .folder = "/srv/app".into();
    app.add_onboard_host(false);
    let checking = last_probe(&requests);
    assert_eq!(checking.root, "/srv/app");
    reply(
        &mut app,
        &ctx,
        checking,
        Ok(FormResult::Probe("/srv/app".into(), None)),
    );
    let adding = requests
        .try_iter()
        .find_map(|job| match job {
            Job::RemoteAdd { project, probe, .. } if !probe => Some(project),
            _ => None,
        })
        .expect("an add");
    assert_eq!(adding.root, "/srv/app");
    let mut state = app.state.clone();
    let mut saved = adding.clone();
    saved.id = "r1".into();
    state.remote_projects.push(saved);
    reply(
        &mut app,
        &ctx,
        adding,
        Ok(FormResult::Added(Box::new(state))),
    );
    assert!(app.remote_ui.onboard.is_none());
    assert!(app.state.remote_projects.iter().any(|p| p.id == "r1"));
}

#[test]
fn the_add_page_draws_the_host_field_and_config_cards() {
    let (mut app, ctx, _dir, _requests) = app_with_jobs();
    app.open_add_hosts(String::new());
    app.remote_ui.hosts.config_hosts = Some(vec![alias("gpu-box").host]);
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 1200.0),
            )),
            ..Default::default()
        },
        |ui| app.center_pane(ui),
    );
    output.textures_delta.clear();
    let texts = painted_strings(&output.shapes);
    for text in ["Add SSH hosts", "Host", "gpu-box", "From ~/.ssh/config"] {
        assert!(texts.iter().any(|t| t == text), "{text}: {texts:?}");
    }
}
