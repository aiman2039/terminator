use super::*;

fn osc(text: &str) -> Vec<Mark> {
    let parts: Vec<&[u8]> = text.split(';').map(str::as_bytes).collect();
    parse_osc(&parts)
}

fn hex(text: &str) -> String {
    use std::fmt::Write;
    text.bytes().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

#[test]
fn final_term_marks_are_parsed() {
    assert_eq!(osc("133;A"), [Mark::PromptStart]);
    assert_eq!(osc("133;B"), [Mark::PromptEnd]);
    assert_eq!(osc("133;C"), [Mark::CommandStart { command: None }]);
    assert_eq!(
        osc("133;D;2"),
        [Mark::CommandFinished { exit_code: Some(2) }]
    );
    assert_eq!(osc("133;D"), [Mark::CommandFinished { exit_code: None }]);
    assert!(osc("133;Z").is_empty());
    assert!(osc("7;file:///tmp").is_empty());
}

#[test]
fn vscode_command_line_and_cwd_are_unescaped() {
    assert_eq!(
        osc(r"633;E;echo a\x3bb \\ c"),
        [Mark::CommandLine(r"echo a;b \ c".into())]
    );
    assert_eq!(
        osc(r"633;P;Cwd=/tmp/with\x3bsemi"),
        [Mark::Cwd("/tmp/with;semi".into())]
    );
    assert_eq!(
        osc("633;D;0"),
        [Mark::CommandFinished { exit_code: Some(0) }]
    );
}

#[test]
fn jetbrains_values_are_hex_decoded() {
    assert_eq!(
        osc(&format!("1341;command_started;command={}", hex("ls -la"))),
        [Mark::CommandStart {
            command: Some("ls -la".into())
        }]
    );
    assert_eq!(
        osc(&format!(
            "1341;command_finished;exit_code={};current_directory={}",
            hex("1"),
            hex("/home/u")
        )),
        [
            Mark::CommandFinished { exit_code: Some(1) },
            Mark::Cwd("/home/u".into())
        ]
    );
    assert_eq!(osc("1341;prompt_started"), [Mark::PromptStart]);
    // Odd-length hex is rejected rather than misread.
    assert_eq!(
        osc("1341;command_started;command=abc"),
        [Mark::CommandStart { command: None }]
    );
}

fn run(tracker: &mut Tracker, marks: &[&str]) -> Vec<CommandBlock> {
    let mut finished = Vec::new();
    for (at, mark) in marks.iter().enumerate() {
        if let Some(text) = mark.strip_prefix("text:") {
            tracker.text(text);
            continue;
        }
        for mark in osc(mark) {
            finished.extend(tracker.apply(mark, (at as u64).saturating_mul(10)));
        }
    }
    finished
}

#[test]
fn a_command_becomes_a_block_with_output_exit_and_duration() {
    let mut tracker = Tracker::default();
    let done = run(
        &mut tracker,
        &[
            r"633;P;Cwd=/work",
            "133;A",
            "text:$ ",
            "133;B",
            r"633;E;make test",
            "133;C",
            "text:running\n",
            "text:FAILED\n",
            "133;D;2",
        ],
    );
    assert_eq!(done.len(), 1);
    let block = &done[0];
    assert_eq!(block.command, "make test");
    assert_eq!(block.cwd.as_deref(), Some(std::path::Path::new("/work")));
    assert_eq!(block.output, "running\nFAILED\n");
    assert_eq!(block.exit_code, Some(2));
    assert!(block.failed());
    assert_eq!(block.duration_ms(), Some(30));
    assert_eq!(tracker.blocks().count(), 1);
}

#[test]
fn prompt_text_is_not_output_and_echo_is_the_fallback_command() {
    let mut tracker = Tracker::default();
    let done = run(
        &mut tracker,
        &[
            "133;A",
            "text:user@host $ ",
            "133;B",
            "text:  git status ",
            "133;C",
            "text:clean\n",
            "133;D;0",
            "133;A",
            "text:user@host $ ",
        ],
    );
    assert_eq!(done[0].command, "git status");
    assert_eq!(done[0].output, "clean\n");
    assert!(!done[0].failed());
}

#[test]
fn a_new_prompt_without_finish_ends_the_running_block() {
    let mut tracker = Tracker::default();
    let done = run(&mut tracker, &["133;B", "133;C", "text:x", "133;A"]);
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].exit_code, None);
    assert!(!done[0].running());
}

#[test]
fn marks_without_a_command_start_do_not_create_blocks() {
    let mut tracker = Tracker::default();
    let done = run(&mut tracker, &["133;A", "133;B", "133;D;1", "text:junk"]);
    assert!(done.is_empty());
    assert_eq!(tracker.blocks().count(), 0);
}

#[test]
fn output_keeps_the_tail_within_the_limit() {
    let mut tracker = Tracker::default();
    run(&mut tracker, &["133;B", "133;C"]);
    let chunk = "é".repeat(1000);
    for _ in 0..100 {
        tracker.text(&chunk);
    }
    tracker.text("END");
    let block = tracker.blocks().last().unwrap();
    assert!(block.output.len() <= MAX_OUTPUT);
    assert!(block.output.ends_with("END"));
    assert!(block.output_truncated);
}

#[test]
fn block_count_is_bounded() {
    let mut tracker = Tracker::default();
    for _ in 0..MAX_BLOCKS + 5 {
        run(&mut tracker, &["133;B", "133;C", "133;D;0"]);
    }
    assert_eq!(tracker.blocks().count(), MAX_BLOCKS);
    assert_eq!(tracker.blocks().next().unwrap().id, 6);
}

#[test]
fn backspace_edits_the_echoed_command() {
    let mut tracker = Tracker::default();
    run(&mut tracker, &["133;B", "text:lss"]);
    tracker.backspace();
    let done = run(&mut tracker, &["133;C", "133;D;0"]);
    assert_eq!(done[0].command, "ls");
}
