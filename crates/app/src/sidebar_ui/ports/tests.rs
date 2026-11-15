//! Ports tool: scans only while shown, lists ports per session, and asks
//! before Stop. No real process is signalled.
use super::*;
use crate::app::tests::support::{painted_strings, session_fixture};

fn port(session: &str, port: u16) -> SessionPort {
    SessionPort {
        session: session.into(),
        pid: 4242,
        started: 1,
        command: "node".into(),
        address: format!("*:{port}"),
        port,
    }
}

fn draw(app: &mut App, ctx: &egui::Context) -> Vec<String> {
    let mut output = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(320.0, 600.0),
            )),
            ..Default::default()
        },
        |ui| app.ports_panel(ui),
    );
    output.textures_delta.clear();
    painted_strings(&output.shapes)
}

#[test]
fn ports_are_scanned_only_while_the_tool_shows() {
    let (mut app, ctx, _dir) = crate::app::fixture();
    app.state.sessions = vec![session_fixture("s1", SessionKind::Shell)];
    app.selected = Some("a".into());
    draw(&mut app, &ctx);
    app.sync_ports_request();
    let request = app.ports.request.clone().unwrap();
    assert_eq!(request.sessions, [("s1".to_owned(), (42, 0))]);
    // A frame without the tool stops the scan.
    app.sync_ports_request();
    assert!(app.ports.request.is_none());
}

#[test]
fn listed_ports_show_with_their_terminal_and_stop_asks_first() {
    let (mut app, ctx, _dir) = crate::app::fixture();
    app.state.sessions = vec![session_fixture("s1", SessionKind::Shell)];
    app.selected = Some("a".into());
    draw(&mut app, &ctx);
    app.sync_ports_request();
    let generation = app.ports.generation;
    // A stale scan is ignored.
    app.handle_ports(PortsUpdate::Listed(
        generation.wrapping_sub(1),
        Ok(vec![port("s1", 9999)]),
    ));
    app.handle_ports(PortsUpdate::Listed(generation, Ok(vec![port("s1", 3000)])));
    let texts = draw(&mut app, &ctx);
    assert!(texts.iter().any(|t| t == ":3000"), "{texts:?}");
    assert!(!texts.iter().any(|t| t == ":9999"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "node (4242)"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "s1"), "{texts:?}");
    assert!(!texts.iter().any(|t| t.starts_with("Stop node")));
    app.ports.confirm = Some(port("s1", 3000));
    let texts = draw(&mut app, &ctx);
    assert!(
        texts.iter().any(|t| t == "Stop node (pid 4242)?"),
        "{texts:?}"
    );
}

#[test]
fn a_failed_stop_is_reported() {
    let (mut app, _ctx, _dir) = crate::app::fixture();
    app.handle_ports(PortsUpdate::Stopped(port("s1", 3000), Err("gone".into())));
    assert!(app.error.text().is_some_and(|t| t.contains("3000")));
}
