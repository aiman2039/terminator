use super::*;
use alacritty_terminal::{
    event::{Event, EventListener},
    grid::{Dimensions, Scroll},
    index::{Column, Line},
    term::{Config, Term, TermMode, test::TermSize},
    vte::ansi::Processor,
};

#[derive(Clone)]
struct IgnoreEvents;
impl EventListener for IgnoreEvents {
    fn send_event(&self, _: Event) {}
}

fn history_rows(screen: &vt100::Screen) -> Vec<String> {
    let mut history = screen.clone();
    history.set_scrollback(usize::MAX);
    (1..=history.scrollback())
        .rev()
        .map(|offset| {
            history.set_scrollback(offset);
            history.rows(0, screen.size().1).next().unwrap()
        })
        .collect()
}

fn gui_replay(parser: &vt100::Parser<Events>) -> Term<IgnoreEvents> {
    let (rows, cols) = parser.screen().size();
    let mut term = Term::new(
        Config::default(),
        &TermSize::new(usize::from(cols), usize::from(rows)),
        IgnoreEvents,
    );
    let mut processor: Processor = Processor::new();
    processor.advance(&mut term, &attachment_snapshot(parser));
    term
}

fn gui_row(term: &Term<IgnoreEvents>, row: i32) -> String {
    (0..term.columns())
        .map(|column| term.grid()[Line(row)][Column(column)].c)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

#[test]
fn bounded_region_keeps_history_and_fixed_footer() {
    let mut parser = vt100::Parser::new_with_callbacks(4, 32, 3, Events::default());
    process(
        &mut parser,
        b"\x1b[4;1HFOOTER\x1b[1;3r\x1b[Hone\r\ntwo\r\nthree\r\nfour\r\nfive",
    );
    assert_eq!(history_rows(parser.screen()), ["one", "two"]);
    assert_eq!(parser.screen().rows(0, 32).last().unwrap(), "FOOTER");
    process(&mut parser, b"\r\nsix\r\nseven");
    assert_eq!(history_rows(parser.screen()), ["two", "three", "four"]);
    assert_eq!(parser.screen().rows(0, 32).last().unwrap(), "FOOTER");
}

#[test]
fn region_below_fixed_header_does_not_add_history() {
    let mut parser = vt100::Parser::new_with_callbacks(4, 32, 100, Events::default());
    process(
        &mut parser,
        b"HEADER\x1b[2;4r\x1b[2;1Hone\r\ntwo\r\nthree\r\nfour\r\nfive",
    );
    assert!(history_rows(parser.screen()).is_empty());
    assert_eq!(parser.screen().rows(0, 32).next().unwrap(), "HEADER");
}

#[test]
fn attachment_keeps_every_history_row_even_when_shorter_than_one_screen() {
    for rows in [1, 4, 24] {
        for capacity in [0, 1, 2, 25, 40] {
            let mut parser =
                vt100::Parser::new_with_callbacks(rows, 32, capacity, Events::default());
            for index in 0..70 {
                process(&mut parser, format!("PUBLIC {index:03}\r\n").as_bytes());
            }
            let expected = history_rows(parser.screen());
            let mut gui = gui_replay(&parser);
            assert_eq!(
                gui.history_size(),
                expected.len(),
                "{rows} rows, cap {capacity}"
            );
            for (index, line) in expected.iter().enumerate() {
                let row = i32::try_from(index).unwrap() - i32::try_from(expected.len()).unwrap();
                assert_eq!(gui_row(&gui, row), *line);
            }
            for (row, expected) in parser.screen().rows(0, 32).enumerate() {
                assert_eq!(gui_row(&gui, i32::try_from(row).unwrap()), expected);
            }
            let (row, col) = parser.screen().cursor_position();
            assert_eq!(gui.grid().cursor.point.line.0, i32::from(row));
            assert_eq!(gui.grid().cursor.point.column.0, usize::from(col));
            gui.scroll_display(Scroll::Delta(1));
            assert_eq!(
                gui.grid().display_offset(),
                usize::from(!expected.is_empty())
            );
        }
    }
}

#[test]
fn bounded_region_history_scrolls_after_attachment_without_resize() {
    let mut parser = vt100::Parser::new_with_callbacks(4, 32, 100, Events::default());
    process(
        &mut parser,
        b"\x1b[4;1HFOOTER\x1b[1;3r\x1b[Hone\r\ntwo\r\nthree\r\nfour\r\nfive",
    );
    let mut gui = gui_replay(&parser);
    assert_eq!(gui.history_size(), 2);
    assert_eq!(gui_row(&gui, 3), "FOOTER");
    gui.scroll_display(Scroll::Delta(2));
    assert_eq!(gui.grid().display_offset(), 2);
    assert_eq!(gui_row(&gui, -2), "one");
    assert_eq!(gui_row(&gui, -1), "two");
}

#[test]
fn clear_keeps_later_same_read_history_and_scrolling_region() {
    for chunked in [false, true] {
        let mut parser = vt100::Parser::new_with_callbacks(4, 32, 100, Events::default());
        process(&mut parser, b"old\r\nold\r\nold\r\nold\r\n");
        let output = b"\x1b[4;1HFOOTER\x1b[1;3r\x1b[H\x1b[3Jone\r\ntwo\r\nthree\r\nfour\r\nfive";
        if chunked {
            for byte in output {
                process(&mut parser, &[*byte]);
            }
        } else {
            process(&mut parser, output);
        }
        assert_eq!(history_rows(parser.screen()), ["one", "two"]);
        assert_eq!(parser.screen().rows(0, 32).last().unwrap(), "FOOTER");
        assert_eq!(gui_replay(&parser).history_size(), 2);
    }
}

#[test]
fn alternate_screen_snapshot_keeps_modes_without_adding_history_or_replaying_notices() {
    let mut parser = vt100::Parser::new_with_callbacks(4, 32, 100, Events::default());
    process(&mut parser, b"old\r\nold\r\nold\r\nold\r\n");
    let primary = history_rows(parser.screen());
    process(
        &mut parser,
        b"\x1b[?1049h\x1b[?1000;1006;1004h\x1b[3J\x1b]9;notice\x07\x1b[31mALT",
    );
    let gui = gui_replay(&parser);
    assert_eq!(gui.history_size(), 0);
    assert!(gui.mode().contains(
        TermMode::ALT_SCREEN
            | TermMode::MOUSE_REPORT_CLICK
            | TermMode::SGR_MOUSE
            | TermMode::FOCUS_IN_OUT
    ));
    assert_eq!(gui_row(&gui, 0), "ALT");
    assert_eq!(
        gui.grid()[Line(0)][Column(0)].fg,
        alacritty_terminal::vte::ansi::Color::Named(alacritty_terminal::vte::ansi::NamedColor::Red)
    );
    assert!(!String::from_utf8_lossy(&attachment_snapshot(&parser)).contains("notice"));
    process(&mut parser, b"\x1b[?1049l");
    assert_eq!(history_rows(parser.screen()), primary);
}
