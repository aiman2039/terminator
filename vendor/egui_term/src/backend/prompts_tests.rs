//! Real-PTY checks: marks pass through the PTY reader wrapper.
use super::super::settings::BackendSettings;
use super::super::{BackendCommand, TerminalBackend};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Two commands (`ok` exits 0, `bad` exits 4), then a prompt, then enough
/// lines to push them all into history. Default size: 80x50.
const SCRIPT: &str = r#"
p() { printf '\033]133;A\007$ \033]133;B\007%s\r\n\033]133;C\007' "$1"; }
p ok; printf 'out-ok\r\n'; printf '\033]133;D;0\007'
p bad; printf 'out-bad\r\nline2\r\n'; printf '\033]133;D;4\007'
printf '\033]133;A\007$ \033]133;B\007'
i=0; while [ $i -lt 60 ]; do printf 'fill\r\n'; i=$((i+1)); done
sleep 30
"#;

fn spawn() -> (
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
            args: vec!["-c".into(), SCRIPT.into()],
            ..BackendSettings::default()
        },
    )
    .expect("spawn sh");
    (backend, rx)
}

fn wait_for_prompts(backend: &mut TerminalBackend, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        // The cache rescans after 250 ms; give the PTY reader time too.
        std::thread::sleep(Duration::from_millis(300));
        backend.sync();
        if backend.prompt_marks().len() >= count {
            return;
        }
        assert!(Instant::now() < deadline, "prompts never arrived");
    }
}

#[test]
fn marks_from_a_real_pty_drive_jumps_selection_and_output_copy() {
    let (mut backend, _events) = spawn();
    wait_for_prompts(&mut backend, 3);
    let marks = backend.prompt_marks().to_vec();
    assert_eq!(marks[0].exit_code, Some(0));
    assert_eq!(marks[1].exit_code, Some(4));
    assert_eq!(
        backend.last_command_output().as_deref(),
        Some("out-bad\nline2")
    );
    let ended = backend.take_ended_commands();
    let codes: Vec<_> = ended.iter().map(|e| e.exit_code).collect();
    assert_eq!(codes, [Some(0), Some(4)]);
    // Jumps are local scrolls: the view top lands on each prompt.
    assert!(backend.jump_to_prompt(false));
    let top = -(backend.sync().display_offset as i32);
    assert_eq!(top, marks[2].line);
    assert!(backend.jump_to_prompt(false));
    assert!(backend.jump_to_prompt(false));
    assert_eq!(-(backend.sync().display_offset as i32), marks[0].line);
    assert!(!backend.jump_to_prompt(false), "no prompt above the first");
    assert!(backend.select_command());
    assert!(backend.selectable_content().starts_with("$ ok\nout-ok"));
    assert!(backend.jump_to_prompt(true));
    assert!(backend.jump_to_prompt(true));
    assert!(
        backend.jump_to_prompt(true),
        "past the last prompt: the live screen"
    );
    assert_eq!(backend.sync().display_offset, 0);
    backend.process_command(BackendCommand::Write(b"\x03".to_vec()));
}
