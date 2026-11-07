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
        |ui| app.center_pane(ui),
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

fn forwarding_host(app: &mut App) -> terminator_core::remote::RemoteProject {
    // No worker answers status polls; their sends are dropped.
    let (jobs, _) = std::sync::mpsc::channel();
    app.jobs = jobs.into();
    app.connected = true;
    app.state.capabilities = vec![
        terminator_core::remote::CAPABILITY.into(),
        terminator_core::remote::HOSTS_CAPABILITY.into(),
        terminator_core::remote::EDIT_CAPABILITY.into(),
        terminator_core::remote::FORWARDING_CAPABILITY.into(),
    ];
    let mut saved = crate::remote_ui::blank_project();
    saved.id = "r1".into();
    saved.name = "Build box".into();
    saved.ssh.host = "build.example.com".into();
    saved.root = "/srv/build".into();
    saved.forwards = vec![terminator_core::remote::Forward {
        local_port: 8080,
        remote_host: "localhost".into(),
        remote_port: 80,
        auto_start: false,
    }];
    app.state.remote_projects.push(saved.clone());
    app.remote_ui.hosts.forward_draft = Some(crate::forwards_ui::ForwardDraft {
        project: "r1".into(),
        ..Default::default()
    });
    saved
}

#[test]
fn ssh_properties_edits_port_forwards_of_a_saved_host() {
    let (mut app, ctx, _dir) = fixture();
    let saved = forwarding_host(&mut app);
    app.open_ssh_properties(saved.clone());
    // The forward form is part of the Properties page (below the fold, so
    // only its text fields paint here).
    let page = paint_ssh_page(&mut app, &ctx);
    assert!(
        page.iter()
            .any(|t| t == "localhost (as seen from the SSH host)"),
        "{page:?}"
    );
    // The section itself lists the forward and the add form.
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
        app.forwards_section(ui, &saved);
    });
    output.textures_delta.clear();
    let texts = crate::app::tests::support::painted_strings(&output.shapes);
    assert!(texts.iter().any(|t| t == "Local port"), "{texts:?}");
    assert!(texts.contains(&saved.forwards[0].label()), "{texts:?}");
}

#[test]
fn hosts_sidebar_lists_forwards_without_edit_forms() {
    let (mut app, ctx, _dir) = fixture();
    let saved = forwarding_host(&mut app);
    app.remote_ui.hosts.forwards_open.insert("r1".into());
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(320.0, 900.0),
            )),
            ..Default::default()
        },
        |ui| app.hosts_sidebar(ui),
    );
    output.textures_delta.clear();
    let texts = painted_strings(&output.shapes);
    assert!(texts.contains(&saved.forwards[0].label()), "{texts:?}");
    assert!(!texts.iter().any(|t| t == "Local port"), "{texts:?}");
    assert!(!texts.iter().any(|t| t == "Save"), "{texts:?}");
}

fn click_ssh_margin(app: &mut App, ctx: &egui::Context) {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1000.0, 800.0));
    let pos = egui::pos2(4.0, 400.0);
    let button = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    for events in [
        vec![],
        vec![egui::Event::PointerMoved(pos), button(true)],
        vec![button(false)],
    ] {
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                events,
                ..Default::default()
            },
            |ui| app.center_pane(ui),
        );
        output.textures_delta.clear();
    }
}

#[test]
fn margin_click_asks_before_discarding_a_changed_host_form() {
    let (mut app, ctx, _dir) = fixture();
    app.new_ssh_host();
    app.remote_ui.draft.as_mut().unwrap().ssh.host = "build.example.com".into();
    click_ssh_margin(&mut app, &ctx);
    assert!(app.remote_ui.open);
    assert!(app.remote_ui.confirm_discard);
}

#[test]
fn margin_click_closes_an_unchanged_host_form() {
    let (mut app, ctx, _dir) = fixture();
    app.new_ssh_host();
    click_ssh_margin(&mut app, &ctx);
    assert!(!app.ssh_page_visible());
}
