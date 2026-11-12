use super::super::settings::BackendSettings;
use super::*;
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn spawn(
    script: &str,
) -> (
    TerminalBackend,
    mpsc::Receiver<(u64, super::super::PtyEvent)>,
) {
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

#[test]
fn copy_mode_moves_a_cursor_and_selects_lines_without_input() {
    let (mut backend, _events) = spawn("printf 'first line\\r\\nsecond line\\r\\n'; sleep 30");
    wait_text(&backend, "second line");
    assert!(!backend.copy_mode());
    backend.set_copy_mode(true);
    assert!(backend.copy_mode());
    backend.copy_mode_command(CopyCommand::Top);
    assert_eq!(backend.copy_mode_cursor(), Some((0, 0)));
    backend.copy_mode_command(CopyCommand::Motion(ViMotion::SemanticRight));
    assert_eq!(backend.copy_mode_cursor(), Some((0, 6)));
    backend.copy_mode_command(CopyCommand::Select(SelectionType::Lines));
    backend.copy_mode_command(CopyCommand::Motion(ViMotion::Down));
    assert!(backend.has_selection());
    assert_eq!(backend.selectable_content(), "first line\nsecond line\n");
    // The same kind again ends the selection.
    backend.copy_mode_command(CopyCommand::Select(SelectionType::Lines));
    assert!(!backend.has_selection());
    backend.copy_mode_command(CopyCommand::Motion(ViMotion::First));
    backend.copy_mode_command(CopyCommand::Motion(ViMotion::SemanticRight));
    assert_eq!(backend.copy_mode_cursor(), Some((1, 7)));
    backend.copy_mode_command(CopyCommand::Select(SelectionType::Simple));
    backend.copy_mode_command(CopyCommand::Motion(ViMotion::SemanticRightEnd));
    assert_eq!(backend.selectable_content(), "line");
    backend.set_copy_mode(false);
    assert!(!backend.copy_mode() && !backend.has_selection());
}
