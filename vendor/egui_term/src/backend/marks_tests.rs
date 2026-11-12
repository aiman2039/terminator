use super::*;
use alacritty_terminal::event::VoidListener;
use alacritty_terminal::term::{test::TermSize, Config, Term};
use alacritty_terminal::vte::ansi::Processor;

fn rewrite_all(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    MarkRewriter::new(CommandLog::default()).feed(input, &mut out);
    out
}

fn rewrite_bytewise(input: &[u8]) -> Vec<u8> {
    let mut rewriter = MarkRewriter::new(CommandLog::default());
    let mut out = Vec::new();
    for byte in input {
        rewriter.feed(std::slice::from_ref(byte), &mut out);
    }
    out
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn prompt_and_output_marks_become_private_links() {
    let out = text(&rewrite_all(
        b"\x1b]133;A\x07$ \x1b]133;B\x07ls\r\n\x1b]133;C\x07a\r\n\x1b]133;D;0\x07",
    ));
    assert!(!out.contains("133;"), "{out:?}");
    assert!(out.contains("\x1b]8;id=tmk-p1;terminator-mark:prompt\x1b\\$ "));
    assert!(out.contains("\x1b]8;id=tmk-o1;terminator-mark:output\x1b\\a"));
    assert_eq!(out.matches("\x1b]8;;\x1b\\").count(), 2);
}

#[test]
fn the_next_prompt_carries_the_exit_code_of_the_last_command() {
    let out = text(&rewrite_all(
        b"\x1b]133;A\x07\x1b]133;C\x07\x1b]133;D;2\x07\x1b]633;A\x1b\\",
    ));
    assert!(out.contains("terminator-mark:prompt?last=2"), "{out:?}");
}

#[test]
fn marks_split_at_every_byte_rewrite_like_whole_reads() {
    let stream: &[u8] =
        b"x\x1b[31mred\x1b]133;A\x07$ \x1b]133;B\x1b\\cmd\x1b]0;title\x07\x1b]133;D;1\x07";
    assert_eq!(rewrite_bytewise(stream), rewrite_all(stream));
}

#[test]
fn other_sequences_pass_through_unchanged() {
    for stream in [
        b"\x1b]0;title\x07".as_slice(),
        b"\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\",
        b"\x1b]1337;File=x\x07",
        b"\x1b[?2004h\x1bc plain \x1b",
        b"\x1b]13x;A\x07",
    ] {
        let mut out = rewrite_all(stream);
        if stream.ends_with(b"\x1b") {
            // A trailing ESC waits for the next byte.
            out.push(0x1b);
        }
        assert_eq!(text(&out), text(stream));
    }
}

#[test]
fn malformed_and_oversized_marks_pass_through_unchanged() {
    let unknown = b"\x1b]133;Z\x07";
    assert_eq!(rewrite_all(unknown), unknown);
    let mut oversized = b"\x1b]133;D;".to_vec();
    oversized.extend(std::iter::repeat_n(b'9', 64 * 1024));
    oversized.push(0x07);
    assert_eq!(rewrite_all(&oversized), oversized);
    // A broken terminator gives the held bytes back before the new escape.
    let broken = b"\x1b]133;A\x1b[0m";
    assert_eq!(rewrite_all(broken), broken);
    let log = CommandLog::default();
    let mut rewriter = MarkRewriter::new(log.clone());
    rewriter.feed(b"\x1b]133;C\x07\x1b]133;D;abc\x07", &mut Vec::new());
    assert_eq!(log.take()[0].exit_code, None);
}

#[test]
fn only_a_started_command_is_logged_when_it_ends() {
    let log = CommandLog::default();
    let mut rewriter = MarkRewriter::new(log.clone());
    let mut out = Vec::new();
    rewriter.feed(b"\x1b]133;D;0\x07\x1b]133;A\x07", &mut out);
    assert!(log.take().is_empty());
    rewriter.feed(b"\x1b]133;C\x07\x1b]133;D;7\x07", &mut out);
    let ended = log.take();
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].exit_code, Some(7));
    // Ctrl-C at a shell that sends no `D`: the next prompt ends it.
    rewriter.feed(b"\x1b]133;C\x07\x1b]133;A\x07", &mut out);
    assert_eq!(log.take()[0].exit_code, None);
}

fn term_with(stream: &[u8], lines: usize) -> Term<VoidListener> {
    let mut term = Term::new(Config::default(), &TermSize::new(20, lines), VoidListener);
    Processor::<alacritty_terminal::vte::ansi::StdSyncHandler>::new()
        .advance(&mut term, &rewrite_all(stream));
    term
}

fn session(commands: &[(&str, i32)]) -> Vec<u8> {
    let mut stream = Vec::new();
    for (command, code) in commands {
        stream.extend_from_slice(b"\x1b]133;A\x07$ \x1b]133;B\x07");
        stream.extend_from_slice(command.as_bytes());
        stream.extend_from_slice(b"\r\n\x1b]133;C\x07out\r\n");
        stream.extend_from_slice(format!("\x1b]133;D;{code}\x07").as_bytes());
    }
    stream.extend_from_slice(b"\x1b]133;A\x07$ \x1b]133;B\x07");
    stream
}

#[test]
fn prompts_are_found_with_the_exit_code_of_their_command() {
    let term = term_with(&session(&[("ok", 0), ("bad", 3)]), 10);
    let marks = prompt_marks(term.grid());
    let lines: Vec<i32> = marks.iter().map(|m| m.line).collect();
    assert_eq!(lines, [0, 2, 4]);
    assert_eq!(marks[0].exit_code, Some(0));
    assert_eq!(marks[1].exit_code, Some(3));
    assert!(marks[1].finished && !marks[2].finished);
    assert_eq!(marks[1].output, Some(Point::new(Line(3), Column(0))));
    assert_eq!(
        term.bounds_to_string(
            Point::new(Line(3), Column(0)),
            Point::new(Line(3), Column(19))
        ),
        "out"
    );
}

#[test]
fn mark_links_are_not_link_targets() {
    let term = term_with(&session(&[("ok", 0)]), 10);
    let prompt = Point::new(Line(0), Column(0));
    assert!(term.grid()[prompt].hyperlink().is_some());
    assert!(super::super::hyperlink_at(term.grid(), prompt).is_none());
    let output = Point::new(Line(1), Column(0));
    assert!(super::super::hyperlink_at(term.grid(), output).is_none());
}

#[test]
fn prompts_keep_their_place_after_scrolling_into_history() {
    let term = term_with(&session(&[("a", 0), ("b", 0), ("c", 1)]), 3);
    let marks = prompt_marks(term.grid());
    let lines: Vec<i32> = marks.iter().map(|m| m.line).collect();
    // Seven rows of output in a three-row screen: four are in history.
    assert_eq!(lines, [-4, -2, 0, 2]);
    assert_eq!(marks[2].exit_code, Some(1));
}

#[test]
fn jumps_go_to_the_prompt_above_or_below_the_view_top() {
    let marks: Vec<PromptMark> = [-10, -4, 2]
        .into_iter()
        .map(|line| PromptMark {
            line,
            output: None,
            exit_code: None,
            finished: true,
        })
        .collect();
    assert_eq!(jump_target(&marks, 0, false), Some(-4));
    assert_eq!(jump_target(&marks, -4, false), Some(-10));
    assert_eq!(jump_target(&marks, -10, false), None);
    assert_eq!(jump_target(&marks, -10, true), Some(-4));
    assert_eq!(jump_target(&marks, 2, true), None);
    assert_eq!(current_command(&marks, -4, true), Some(1));
    assert_eq!(current_command(&marks, 0, false), Some(2));
}
