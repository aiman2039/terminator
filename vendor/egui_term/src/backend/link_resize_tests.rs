use super::settings::BackendSettings;
use super::*;
use std::sync::mpsc;

const FONT: Size = Size {
    width: 8.0,
    height: 16.0,
};

fn spawn(script: &str) -> (TerminalBackend, mpsc::Receiver<(u64, PtyEvent)>) {
    let (tx, rx) = mpsc::channel();
    let backend = TerminalBackend::new(
        0,
        egui::Context::default(),
        tx,
        BackendSettings {
            shell: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            ..BackendSettings::default()
        },
    )
    .expect("spawn sh");
    (backend, rx)
}

fn wait_text(backend: &TerminalBackend, needle: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !backend
        .search_rows()
        .iter()
        .any(|row| row.text.contains(needle))
    {
        assert!(Instant::now() < deadline, "never printed {needle}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn link_at(backend: &mut TerminalBackend, point: Point) -> Option<String> {
    let term = backend.term.clone();
    let term = term.lock();
    backend.link_at(&term, point)
}

#[test]
fn link_open_reads_the_link_under_the_click_after_a_resize() {
    let (mut backend, _events) = spawn("printf 'see https://example.com/abc now\\r\\n'; sleep 30");
    backend.process_command(BackendCommand::Resize(
        Size::new(80.0 * 8.0, 10.0 * 16.0),
        FONT,
    ));
    wait_text(&backend, "https://example.com/abc");
    let row = backend
        .search_rows()
        .into_iter()
        .find(|row| row.text.contains("https://"))
        .unwrap();
    let on_link = Point::new(row.line, Column(10));
    backend.process_command(BackendCommand::ProcessLink(LinkAction::Hover, on_link));
    assert!(backend.last_content().hovered_hyperlink.is_some());

    // A narrower pane drops the stored hover range of the old geometry.
    backend.process_command(BackendCommand::Resize(
        Size::new(20.0 * 8.0, 10.0 * 16.0),
        FONT,
    ));
    assert!(backend.last_content().hovered_hyperlink.is_none());

    // A click point past the new row end finds nothing and does not panic.
    assert_eq!(
        link_at(&mut backend, Point::new(row.line, Column(70))),
        None
    );
}

#[test]
fn link_open_returns_the_whole_url_under_the_click() {
    let (mut backend, _events) = spawn("printf 'see https://example.com/abc now\\r\\n'; sleep 30");
    backend.process_command(BackendCommand::Resize(
        Size::new(80.0 * 8.0, 10.0 * 16.0),
        FONT,
    ));
    wait_text(&backend, "https://example.com/abc");
    let row = backend
        .search_rows()
        .into_iter()
        .find(|row| row.text.contains("https://"))
        .unwrap();
    assert_eq!(
        link_at(&mut backend, Point::new(row.line, Column(12))).as_deref(),
        Some("https://example.com/abc")
    );
    assert_eq!(link_at(&mut backend, Point::new(row.line, Column(1))), None);
}

#[test]
fn pane_smaller_than_one_cell_is_not_a_resize_on_every_frame() {
    let (mut backend, _events) = spawn("sleep 30");
    let tiny = Size::new(3.0, 5.0);
    backend.process_command(BackendCommand::Resize(tiny, FONT));
    backend.grid_dirty.store(false, Ordering::Relaxed);
    backend.process_command(BackendCommand::Resize(tiny, FONT));
    assert!(
        !backend.grid_dirty.load(Ordering::Relaxed),
        "a repeated sub-cell layout dirtied the grid again"
    );
    // A real size still applies.
    backend.process_command(BackendCommand::Resize(Size::new(80.0, 48.0), FONT));
    assert_eq!(backend.size.num_cols, 10);
}
