//! Resize cycles: the daemon parser, its attachment replay, and the GUI
//! terminal model. Real-PTY sizes are checked by `cargo xtask integration`.
use super::scrollback_tests::{IgnoreEvents, gui_replay, gui_row, history_rows};
use super::*;
use alacritty_terminal::{
    grid::Dimensions,
    index::{Column, Line, Point, Side},
    selection::{Selection, SelectionType},
    term::{Config, Term, TermMode, test::TermSize},
    vte::ansi::Processor,
};

/// Replays `parser` into a GUI terminal and checks history, visible rows,
/// size, and cursor against the daemon screen.
fn assert_replay_matches(parser: &vt100::Parser<Events>, label: &str) -> Term<IgnoreEvents> {
    let gui = gui_replay(parser);
    let screen = parser.screen();
    let (rows, cols) = screen.size();
    assert_eq!(
        (gui.screen_lines(), gui.columns()),
        (usize::from(rows), usize::from(cols)),
        "{label}"
    );
    let history = history_rows(screen);
    assert_eq!(gui.history_size(), history.len(), "{label}");
    let depth = i32::try_from(history.len()).unwrap();
    for (index, line) in history.iter().enumerate() {
        let row = i32::try_from(index).unwrap().saturating_sub(depth);
        assert_eq!(gui_row(&gui, row), *line, "{label}: history {index}");
    }
    for (row, line) in screen.rows(0, cols).enumerate() {
        let row = i32::try_from(row).unwrap();
        assert_eq!(gui_row(&gui, row), line, "{label}: row {row}");
    }
    let (row, col) = screen.cursor_position();
    let cursor = gui.grid().cursor.point;
    // After a full row, vt100 reports the pending-wrap column `cols`;
    // alacritty keeps the cursor on the last column with a wrap flag.
    let col = col.min(cols.saturating_sub(1));
    assert_eq!(
        (cursor.line.0, cursor.column.0),
        (i32::from(row), usize::from(col)),
        "{label}: cursor"
    );
    gui
}

fn gui_term(rows: usize, cols: usize, bytes: &[u8]) -> Term<IgnoreEvents> {
    let mut term = Term::new(Config::default(), &TermSize::new(cols, rows), IgnoreEvents);
    let mut processor: Processor = Processor::new();
    processor.advance(&mut term, bytes);
    term
}

fn visible(term: &Term<IgnoreEvents>) -> Vec<String> {
    (0..i32::try_from(term.screen_lines()).unwrap())
        .map(|row| gui_row(term, row))
        .collect()
}

#[test]
fn main_screen_shrink_and_expand_cycles_replay_the_daemon_screen() {
    let mut parser = vt100::Parser::new_with_callbacks(6, 20, 100, Events::default());
    for index in 0..12 {
        process(&mut parser, format!("line {index:02}\r\n").as_bytes());
    }
    // 50 characters wrap across three 20-column rows.
    process(&mut parser, "abcdefghij".repeat(5).as_bytes());
    process(&mut parser, b"\r\n$ tail");
    assert_replay_matches(&parser, "initial");
    for (rows, cols) in [(3, 20), (3, 10), (8, 40), (12, 8), (2, 30), (6, 20)] {
        parser.screen_mut().set_size(rows, cols);
        assert_replay_matches(&parser, &format!("resized to {rows}x{cols}"));
        process(&mut parser, format!("\r\nafter {rows}x{cols}").as_bytes());
        assert_replay_matches(&parser, &format!("output after {rows}x{cols}"));
    }
}

#[test]
fn alternate_screen_resize_while_detached_replays_and_restores_primary() {
    let mut parser = vt100::Parser::new_with_callbacks(6, 30, 100, Events::default());
    for index in 0..10 {
        process(&mut parser, format!("shell {index}\r\n").as_bytes());
    }
    process(&mut parser, b"\x1b[?1049h\x1b[HTOP\x1b[6;1HBOTTOM\x1b[3;5H");
    let attached = assert_replay_matches(&parser, "alternate at 6x30");
    assert!(attached.mode().contains(TermMode::ALT_SCREEN));
    for (rows, cols) in [(4, 12), (10, 40)] {
        // No GUI is attached while the session is resized.
        parser.screen_mut().set_size(rows, cols);
        let gui = assert_replay_matches(&parser, &format!("alternate at {rows}x{cols}"));
        assert!(gui.mode().contains(TermMode::ALT_SCREEN));
        assert_eq!(gui.history_size(), 0, "the alternate screen has no history");
    }
    process(&mut parser, b"\x1b[?1049l");
    let gui = assert_replay_matches(&parser, "primary restored at 10x40");
    assert!(!gui.mode().contains(TermMode::ALT_SCREEN));
    assert!(
        history_rows(parser.screen())
            .iter()
            .all(|row| !row.contains("TOP"))
    );
}

#[test]
fn live_output_after_a_detached_resize_matches_a_continuous_parser() {
    let mut parser = vt100::Parser::new_with_callbacks(5, 20, 100, Events::default());
    process(&mut parser, b"first\r\nsecond\r\n");
    assert_replay_matches(&parser, "first attachment");
    // Detached: output continues and the session is resized.
    process(&mut parser, b"while detached\r\n");
    parser.screen_mut().set_size(8, 30);
    process(&mut parser, b"after resize\r\n");
    let mut gui = assert_replay_matches(&parser, "reattached at 8x30");
    // Output after the snapshot is relayed once, in order.
    let mut processor: Processor = Processor::new();
    for chunk in [
        &b"live 1\r\n"[..],
        b"\x1b[31mred\x1b[0m\r\n",
        b"live 2",
        b"\x1b[2;3Hx",
    ] {
        process(&mut parser, chunk);
        processor.advance(&mut gui, chunk);
    }
    let rows: Vec<String> = parser.screen().rows(0, 30).collect();
    assert_eq!(visible(&gui), rows);
    let (row, col) = parser.screen().cursor_position();
    let cursor = gui.grid().cursor.point;
    assert_eq!(
        (cursor.line.0, cursor.column.0),
        (i32::from(row), usize::from(col))
    );
    assert_eq!(rows.iter().filter(|row| row.contains("live 1")).count(), 1);
}

#[test]
fn gui_height_resize_keeps_the_selected_text_and_width_resize_clears_it() {
    let lines: Vec<String> = (0..10).map(|index| format!("row {index}")).collect();
    let mut term = gui_term(6, 20, lines.join("\r\n").as_bytes());
    assert_eq!(gui_row(&term, 3), "row 7");
    let mut selection = Selection::new(
        SelectionType::Simple,
        Point::new(Line(3), Column(0)),
        Side::Left,
    );
    selection.update(Point::new(Line(3), Column(4)), Side::Right);
    term.selection = Some(selection);
    assert_eq!(term.selection_to_string().as_deref(), Some("row 7"));
    for rows in [3_usize, 8, 6] {
        term.resize(TermSize::new(20, rows));
        assert_eq!(
            term.selection_to_string().as_deref(),
            Some("row 7"),
            "{rows} rows"
        );
        // The cursor stays on the last line; hidden lines move to history.
        assert_eq!(gui_row(&term, i32::try_from(rows).unwrap() - 1), "row 9");
        assert_eq!(term.grid().history_size(), 10 - rows);
        let cursor = term.grid().cursor.point;
        assert_eq!(
            (cursor.line.0, cursor.column.0),
            (i32::try_from(rows).unwrap() - 1, 5)
        );
    }
    term.resize(TermSize::new(10, 6));
    assert!(
        term.selection.is_none(),
        "a width change invalidates the selection"
    );
}

/// History and visible rows, oldest first.
fn all_rows(term: &Term<IgnoreEvents>) -> Vec<String> {
    let history = i32::try_from(term.grid().history_size()).unwrap();
    let lines = i32::try_from(term.screen_lines()).unwrap();
    (history.saturating_neg()..lines)
        .map(|row| gui_row(term, row))
        .collect()
}

#[test]
fn gui_width_reflow_round_trip_restores_wrapped_text_and_cursor() {
    let mut term = gui_term(4, 10, b"0123456789ABC");
    assert_eq!(visible(&term), ["0123456789", "ABC", "", ""]);
    let steps: [(usize, &[&str]); 3] = [
        (20, &["0123456789ABC"]),
        (5, &["01234", "56789", "ABC"]),
        (10, &["0123456789", "ABC"]),
    ];
    for (cols, wrapped) in steps {
        term.resize(TermSize::new(cols, 4));
        let rows = all_rows(&term);
        assert!(
            rows.windows(wrapped.len()).any(|window| window == wrapped),
            "{cols} columns: {rows:?}"
        );
        // The cursor stays right after the last character.
        let cursor = term.grid().cursor.point;
        let line = gui_row(&term, cursor.line.0);
        assert_eq!(line, *wrapped.last().unwrap(), "{cols} columns");
        assert_eq!(cursor.column.0, line.len(), "{cols} columns");
    }
}
