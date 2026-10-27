use super::*;
use super::*;
#[test]
fn reconnect_preserves_focus_reporting_and_explicit_alternate_scroll() {
    let mut parser = vt100::Parser::new_with_callbacks(24, 80, 100, Events::default());
    for byte in b"\x1b[?1004;1007h\x1b[?1007l" {
        process(&mut parser, &[*byte]);
    }
    assert_eq!(
        parser.callbacks().modes_formatted(),
        b"\x1b[?1004h\x1b[?1007l"
    );
    let mut replay = vt100::Parser::new_with_callbacks(24, 80, 100, Events::default());
    process(&mut replay, &parser.screen().state_formatted());
    process(&mut replay, &parser.callbacks().modes_formatted());
    assert_eq!(
        replay.callbacks().modes_formatted(),
        parser.callbacks().modes_formatted()
    );
    process(&mut replay, b"\x1b[?1004$p\x1b[?1007$p");
    assert_eq!(
        replay
            .callbacks()
            .replies
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        ["\x1b[?1004;1$y", "\x1b[?1007;2$y"]
    );
}

#[test]
fn terminal_reset_clears_extension_modes_in_stream_order_across_chunks() {
    let mut parser = vt100::Parser::new_with_callbacks(24, 80, 100, Events::default());
    process(&mut parser, b"\x1b[?1004h\x1b[?1007lX\x1b");
    process(&mut parser, b"\0c\x1b[?1007l");
    assert!(parser.screen().contents().is_empty());
    assert_eq!(parser.callbacks().modes_formatted(), b"\x1b[?1007l");
    process(&mut parser, b"\x1b[?1004h\x1b c");
    assert_eq!(
        parser.callbacks().modes_formatted(),
        b"\x1b[?1004h\x1b[?1007l"
    );
    process(&mut parser, b"\x1b]9;public\x1bc");
    assert!(parser.callbacks().modes_formatted().is_empty());
}

#[test]
fn replies_use_the_same_screen_as_reconnect_snapshots() {
    let mut parser = vt100::Parser::new_with_callbacks(24, 80, 100, Events::default());
    parser.process(b"\x1b[4;7H\x1b[6n\x1b[5n\x1b[c\x1b]11;?\x07");
    let replies = &parser.callbacks().replies;
    assert_eq!(replies[0], "\x1b[4;7R");
    assert_eq!(replies[1], "\x1b[0n");
    assert_eq!(replies[2], PRIMARY_DA);
    assert!(replies[3].contains("1919"));
    parser.screen_mut().set_size(40, 100);
    parser.process(b"\x1b[18t");
    assert_eq!(parser.callbacks().replies.back().unwrap(), "\x1b[8;40;100t");
}
#[test]
fn parses_chunked_notifications_without_confusing_progress_or_agent_state() {
    let mut p = vt100::Parser::new_with_callbacks(24, 80, 100, Events::default());
    for bytes in [
        b"\x1b]9;hello\x07".as_slice(),
        b"\x1b]777;notify;Title;Body\x1b\\",
        b"\x1b]99;i=one:d=0;Kitty\x1b\\",
        b"\x1b]99;i=one:p=body;done\x1b\\",
        b"\x1b]9;4;1;50\x07",
    ] {
        for byte in bytes {
            p.process(&[*byte]);
        }
    }
    assert_eq!(p.callbacks().notices.len(), 3);
    let last = p.callbacks().notices.back().unwrap();
    assert_eq!((&*last.title, &*last.body), ("Kitty", "done"));
    let mut replay = vt100::Parser::new_with_callbacks(24, 80, 100, Events::default());
    replay.process(&p.screen().state_formatted());
    assert!(replay.callbacks().notices.is_empty());
    assert!(replay.callbacks().replies.is_empty());
}
#[test]
fn notification_queues_and_payloads_are_bounded() {
    let mut p = vt100::Parser::new_with_callbacks(24, 80, 100, Events::default());
    for _ in 0..100 {
        p.process(format!("\x1b]9;{}\x07", "x".repeat(8000)).as_bytes());
    }
    assert!(p.callbacks().notices.len() <= 16);
    assert!(p.callbacks().notices.iter().all(|n| n.body.len() <= 1024));
}
#[test]
fn snapshot_retains_wide_cells_colors_alt_screen_and_input_modes() {
    let mut p = vt100::Parser::new_with_callbacks(24, 80, 100, Events::default());
    p.process(
        "scrollback\r\n\x1b[?1049h\x1b[?2004h\x1b[?1000h\x1b[31m日本\x1b[4;5Hcursor".as_bytes(),
    );
    let mut replay = vt100::Parser::new(24, 80, 100);
    if p.screen().alternate_screen() {
        replay.process(b"\x1b[?1049h");
    }
    replay.process(&p.screen().state_formatted());
    assert_eq!(replay.screen().contents(), p.screen().contents());
    assert_eq!(
        replay.screen().cursor_position(),
        p.screen().cursor_position()
    );
    assert_eq!(
        replay.screen().alternate_screen(),
        p.screen().alternate_screen()
    );
    assert_eq!(
        replay.screen().input_mode_formatted(),
        p.screen().input_mode_formatted()
    );
    assert_eq!(
        replay.screen().cell(0, 0).unwrap().fgcolor(),
        p.screen().cell(0, 0).unwrap().fgcolor()
    );
}
#[test]
fn reports_xterm_da_and_decrqm_for_alt_screen_and_mouse() {
    let mut p = vt100::Parser::new_with_callbacks(24, 80, 100, Events::default());
    p.process(b"\x1b[c\x1b[>c\x1b[?1049$p\x1b[?1000$p\x1b[?1006$p");
    let replies: Vec<_> = p.callbacks().replies.iter().cloned().collect();
    assert_eq!(replies[0], PRIMARY_DA);
    assert_eq!(replies[1], SECONDARY_DA);
    assert_eq!(replies[2], "\x1b[?1049;2$y");
    assert_eq!(replies[3], "\x1b[?1000;2$y");
    assert_eq!(replies[4], "\x1b[?1006;2$y");
    p.process(b"\x1b[?1049h\x1b[?1000h\x1b[?1006h\x1b[?1049$p\x1b[?1000$p\x1b[?1006$p");
    let replies = &p.callbacks().replies;
    assert_eq!(replies[replies.len() - 3], "\x1b[?1049;1$y");
    assert_eq!(replies[replies.len() - 2], "\x1b[?1000;1$y");
    assert_eq!(replies[replies.len() - 1], "\x1b[?1006;1$y");
}
fn scrollback_len(parser: &mut vt100::Parser<Events>) -> usize {
    parser.screen_mut().set_scrollback(usize::MAX);
    let len = parser.screen().scrollback();
    parser.screen_mut().set_scrollback(0);
    len
}
#[test]
fn e3_clears_scrollback_so_reattach_does_not_resurrect_cleared_lines() {
    let mut p = vt100::Parser::new_with_callbacks(24, 80, 100, Events::default());
    for i in 0..60 {
        process(&mut p, format!("FILL_{i}\r\n").as_bytes());
    }
    assert!(scrollback_len(&mut p) > 0);
    // `clear` output plus the trailing E3 from `clear && printf '\033[3J'`.
    process(&mut p, b"\x1b[3J\x1b[H\x1b[2J");
    process(&mut p, b"\x1b[3J");
    process(&mut p, b"AFTER\r\n");
    assert_eq!(scrollback_len(&mut p), 0);
    assert!(p.screen().contents().contains("AFTER"));
    assert!(!p.screen().contents().contains("FILL"));
}
#[test]
fn e3_preserves_screen_cursor_colors_modes_and_future_scrollback() {
    let mut p = vt100::Parser::new_with_callbacks(24, 80, 100, Events::default());
    for i in 0..60 {
        process(&mut p, format!("FILL_{i}\r\n").as_bytes());
    }
    process(&mut p, b"\x1b[?1004h\x1b[31mRED\x1b[5;10H\x1b[3J");
    assert_eq!(scrollback_len(&mut p), 0);
    assert!(p.screen().contents().contains("FILL_59"));
    assert_eq!(p.screen().cursor_position(), (4, 9));
    assert_eq!(
        p.screen().cell(23, 0).unwrap().fgcolor(),
        vt100::Color::Idx(1)
    );
    assert_eq!(p.callbacks().modes_formatted(), b"\x1b[?1004h");
    // Future scrollback uses the wired capacity, not the emptied buffer.
    process(&mut p, b"\x1b[24;1H");
    for i in 0..60 {
        process(&mut p, format!("NEW_{i}\r\n").as_bytes());
    }
    assert_eq!(scrollback_len(&mut p), 60);
}
#[test]
fn e3_split_across_chunks_still_clears_scrollback() {
    let mut p = vt100::Parser::new_with_callbacks(24, 80, 100, Events::default());
    for i in 0..30 {
        process(&mut p, format!("FILL_{i}\r\n").as_bytes());
    }
    assert!(scrollback_len(&mut p) > 0);
    process(&mut p, b"\x1b[3");
    process(&mut p, b"J");
    assert_eq!(scrollback_len(&mut p), 0);
}
#[test]
fn e3_on_the_alternate_screen_keeps_alt_content() {
    let mut p = vt100::Parser::new_with_callbacks(24, 80, 100, Events::default());
    process(&mut p, b"normal\r\n\x1b[?1049hALTSCREEN\x1b[3J");
    assert!(p.screen().alternate_screen());
    assert!(p.screen().contents().contains("ALTSCREEN"));
}
#[test]
fn palette_queries_return_distinct_ansi_colors() {
    let mut p = vt100::Parser::new_with_callbacks(24, 80, 100, Events::default());
    p.process(b"\x1b]4;1;?\x1b\\\x1b]4;2;?\x1b\\");
    let replies: Vec<_> = p.callbacks().replies.iter().cloned().collect();
    assert!(replies[0].contains("acac/4242/4242"), "{}", replies[0]);
    assert!(replies[1].contains("9090/a9a9/5959"), "{}", replies[1]);
    assert_ne!(replies[0], replies[1]);
}
