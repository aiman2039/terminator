//! SSH page: typed addresses, New host, and adding at the login directory.
use super::*;
use terminator_core::remote::HOME_ROOT;

#[test]
fn typed_new_address_opens_new_host_on_the_ssh_page() {
    let (mut app, _, _dir) = fixture();
    app.show_ssh_page();
    app.remote_ui.hosts.address = "me@rno2-host.example.com:2222".into();
    app.ssh_open_address();
    assert!(app.ssh_page_visible());
    assert!(app.remote_ui.open);
    let draft = app.remote_ui.draft.clone().unwrap();
    assert!(draft.id.is_empty());
    assert_eq!(draft.ssh.host, "rno2-host.example.com");
    assert_eq!(draft.ssh.username, "me");
    assert_eq!(draft.ssh.port, Some(2222));
    assert!(draft.root.is_empty());
    assert!(app.remote_ui.hosts.address.is_empty());
    app.show_vnc_page();
    assert!(!app.ssh_page_visible());
}

#[test]
fn invalid_typed_address_shows_an_error_and_keeps_the_text() {
    let (mut app, _, _dir) = fixture();
    app.show_ssh_page();
    app.remote_ui.hosts.address = "bad host!".into();
    app.ssh_open_address();
    assert!(!app.remote_ui.open);
    assert!(app.remote_ui.hosts.page_error.is_some());
    assert_eq!(app.remote_ui.hosts.address, "bad host!");
}

#[test]
fn adding_without_a_path_probes_the_login_directory_then_saves_there() {
    let (mut app, ctx, _dir) = fixture();
    // No worker may answer the probe: the test supplies the result.
    let (jobs, _requests) = std::sync::mpsc::channel();
    app.jobs = jobs.into();
    let mut draft = crate::remote_ui::blank_project();
    draft.ssh.host = "build.example.com".into();
    draft.name = "build".into();
    app.open_ssh_properties(draft);
    app.save_ssh_properties();
    assert!(app.remote_ui.save_after_probe);
    let probing = app.remote_ui.active.clone().unwrap();
    assert_eq!(probing.root, HOME_ROOT);

    let generation = app.selection_generation;
    app.update_tx
        .send(Update::RemoteAdded(
            probing,
            generation,
            Ok(crate::remote_ui::FormResult::Probe("/home/me".into())),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(!app.remote_ui.save_after_probe);
    let saving = app.remote_ui.active.clone().unwrap();
    assert_eq!(saving.root, "/home/me");
    assert!(saving.id.is_empty());
    assert!(app.remote_ui.busy);
}

#[test]
fn failed_probe_does_not_save_on_the_next_test() {
    let (mut app, ctx, _dir) = fixture();
    // No worker may answer the probe: the test supplies the result.
    let (jobs, _requests) = std::sync::mpsc::channel();
    app.jobs = jobs.into();
    let mut draft = crate::remote_ui::blank_project();
    draft.ssh.host = "slow.example.com".into();
    draft.name = "slow".into();
    app.open_ssh_properties(draft);
    app.save_ssh_properties();
    let probing = app.remote_ui.active.clone().unwrap();
    let generation = app.selection_generation;
    app.update_tx
        .send(Update::RemoteAdded(
            probing,
            generation,
            Err(String::from("timed out").into()),
        ))
        .unwrap();
    drain_updates(&mut app, &ctx);
    assert!(!app.remote_ui.save_after_probe);
    assert_eq!(app.remote_ui.error.as_deref(), Some("timed out"));
}

fn paint_ssh_page(app: &mut App, ctx: &egui::Context) -> Vec<String> {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 1600.0),
            )),
            ..Default::default()
        },
        |ui| app.ssh_page_or_vnc_center(ui),
    );
    output.textures_delta.clear();
    painted_text(&output.shapes)
        .into_iter()
        .map(|(_, text)| text)
        .collect()
}

#[test]
fn ssh_page_draws_saved_hosts_and_the_new_host_form() {
    let (mut app, ctx, _dir) = fixture();
    app.connected = true;
    app.state.capabilities = vec![
        terminator_core::remote::CAPABILITY.into(),
        terminator_core::remote::HOSTS_CAPABILITY.into(),
        terminator_core::remote::EDIT_CAPABILITY.into(),
        terminator_core::remote::HOST_TIMEOUT_CAPABILITY.into(),
    ];
    let mut saved = crate::remote_ui::blank_project();
    saved.id = "r1".into();
    saved.name = "Build box".into();
    saved.ssh.host = "build.example.com".into();
    saved.root = "/srv/build".into();
    app.state.remote_projects.push(saved);
    app.show_ssh_page();
    let texts = paint_ssh_page(&mut app, &ctx);
    assert!(texts.iter().any(|t| t == "Build box"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "New host"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "Connect"), "{texts:?}");

    app.new_ssh_host();
    let texts = paint_ssh_page(&mut app, &ctx);
    assert!(
        texts.iter().any(|t| t == "Host or config alias"),
        "{texts:?}"
    );
    assert!(texts.iter().any(|t| t == "Add host"), "{texts:?}");
    assert!(
        texts.iter().any(|t| t == "Command timeout (seconds)"),
        "{texts:?}"
    );
}
