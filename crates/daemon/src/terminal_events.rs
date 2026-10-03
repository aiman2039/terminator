//! Extra VT events use vt100's existing parser callbacks. There is one screen
//! model/parser in the daemon, and no terminal-text heuristics for agent state.
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, STANDARD_NO_PAD},
};
use std::collections::{BTreeMap, VecDeque};
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

/// VT420 + 132-col + printer + selective erase + ANSI color.
/// VT102 (`?6c`) makes TUIs skip mouse, alt-screen, and truecolor probes.
const PRIMARY_DA: &str = "\x1b[?64;1;2;6;22c";
const SECONDARY_DA: &str = "\x1b[>0;100;1c";

/// Islands Dark 16-color palette, matching `egui_term` defaults.
const ANSI16: [[u8; 3]; 16] = [
    [0x19, 0x1a, 0x1c],
    [0xac, 0x42, 0x42],
    [0x90, 0xa9, 0x59],
    [0xf4, 0xbf, 0x75],
    [0x6a, 0x9f, 0xb5],
    [0xaa, 0x75, 0x9f],
    [0x75, 0xb5, 0xaa],
    [0xd1, 0xd3, 0xd9],
    [0x6b, 0x6b, 0x6b],
    [0xc5, 0x55, 0x55],
    [0xaa, 0xc4, 0x74],
    [0xfe, 0xca, 0x88],
    [0x82, 0xb8, 0xc8],
    [0xc2, 0x8c, 0xb8],
    [0x93, 0xd3, 0xc3],
    [0xf8, 0xf8, 0xf8],
];
#[derive(Clone, Debug, Default)]
pub struct Notice {
    pub title: String,
    pub body: String,
}
#[derive(Default)]
pub struct Events {
    pub replies: VecDeque<String>,
    pub notices: VecDeque<Notice>,
    chunks: BTreeMap<String, Notice>,
    extra_modes: BTreeMap<u16, bool>,
    escape_pending: bool,
    scrollback_cap: usize,
    scrollback_clear_pending: bool,
}
/// Feed the existing vt100 parser, clearing extension modes in stream order on
/// RIS. vt100 handles RIS internally without a callback. ESC starts an escape
/// from every VTE state; C0 controls/DEL do not finish that escape.
pub fn process(parser: &mut vt100::Parser<Events>, bytes: &[u8]) {
    let mut escaped = parser.callbacks().escape_pending;
    let mut start = 0;
    for (index, byte) in bytes.iter().copied().enumerate() {
        match byte {
            0x1b => escaped = true,
            b'c' if escaped => {
                let Some(chunk) = bytes.get(start..=index) else {
                    return;
                };
                parser.process(chunk);
                parser.callbacks_mut().extra_modes.clear();
                start = index.saturating_add(1);
                escaped = false;
            }
            0x00..=0x17 | 0x19 | 0x1c..=0x1f | 0x7f..=0xff => {}
            _ => escaped = false,
        }
    }
    if let Some(rest) = bytes.get(start..) {
        parser.process(rest);
    }
    if parser.callbacks().scrollback_clear_pending {
        rebuild_without_scrollback(parser);
    }
    parser.callbacks_mut().escape_pending = escaped;
}

/// vt100 models ED 0-2 but offers no scrollback-clear primitive, so plain
/// `CSI 3 J` (emitted by `clear`, and again by scrollback-clearing aliases)
/// lands in `unhandled_csi`. Without this rebuild the attach snapshot keeps
/// replaying pre-clear scrollback and every reattach resurrects cleared
/// lines. The fresh parser replays the formatted screen, cursor, colors, and
/// input modes; only saved scrollback is dropped. Callback state (replies,
/// notices, extension modes) moves across untouched.
///
/// The rebuild runs after the chunk that carried E3, so same-chunk output
/// scrolled after E3 is dropped from the model too; the live attach stream
/// is raw passthrough and unaffected. Never rebuilds on the alternate
/// screen: its grid holds no scrollback, and replaying alt-screen content
/// onto a fresh primary grid would corrupt the model.
fn rebuild_without_scrollback(parser: &mut vt100::Parser<Events>) {
    parser.callbacks_mut().scrollback_clear_pending = false;
    if parser.screen().alternate_screen() {
        return;
    }
    let (rows, cols) = parser.screen().size();
    parser.screen_mut().set_scrollback(usize::MAX);
    let len = parser.screen().scrollback();
    parser.screen_mut().set_scrollback(0);
    let cap = parser.callbacks().scrollback_cap.max(len);
    let state = parser.screen().state_formatted();
    let cursor = parser.screen().cursor_state_formatted();
    let attrs = parser.screen().attributes_formatted();
    let mut fresh = vt100::Parser::new_with_callbacks(rows, cols, cap, Events::default());
    std::mem::swap(fresh.callbacks_mut(), parser.callbacks_mut());
    fresh.process(&state);
    fresh.process(&cursor);
    fresh.process(&attrs);
    *parser = fresh;
}

fn first_param(params: &[&[u16]]) -> u16 {
    params.first().and_then(|p| p.first()).copied().unwrap_or(0)
}

fn text(bytes: &[u8], limit: usize) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect()
}
impl Events {
    pub fn set_scrollback_cap(&mut self, cap: usize) {
        self.scrollback_cap = cap;
    }
    pub fn modes_formatted(&self) -> Vec<u8> {
        self.extra_modes
            .iter()
            .flat_map(|(mode, enabled)| {
                format!("\x1b[?{mode}{}", if *enabled { 'h' } else { 'l' }).into_bytes()
            })
            .collect()
    }
    fn reply(&mut self, reply: String) {
        if self.replies.len() < 64 {
            self.replies.push_back(reply);
        }
    }
    fn notice(&mut self, title: String, body: String) {
        if title.is_empty() && body.is_empty() {
            return;
        }
        if self.notices.len() == 16 {
            self.notices.pop_front();
        }
        self.notices.push_back(Notice { title, body });
    }
    fn kitty(&mut self, params: &[&[u8]]) {
        if params.len() < 3 {
            return;
        }
        let Some(metadata) = params.get(1) else {
            return;
        };
        let metadata = String::from_utf8_lossy(metadata);
        let fields = metadata
            .split(':')
            .filter_map(|s| s.split_once('='))
            .collect::<BTreeMap<_, _>>();
        let id = fields.get("i").copied().unwrap_or("0");
        if id.len() > 64
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return;
        }
        let part = fields.get("p").copied().unwrap_or("title");
        if part == "?" {
            self.reply(format!("\x1b]99;i={id}:p=?;p=title,body:o=always\x1b\\"));
            return;
        }
        if !matches!(part, "title" | "body") {
            return;
        }
        let Some(parts) = params.get(2..) else {
            return;
        };
        let joined = parts.join(&b';');
        let payload = if fields.get("e") == Some(&"1") {
            let Ok(data) = STANDARD
                .decode(&joined)
                .or_else(|_| STANDARD_NO_PAD.decode(&joined))
            else {
                return;
            };
            data
        } else {
            joined
        };
        if !self.chunks.contains_key(id) && self.chunks.len() >= 16 {
            self.chunks.pop_first();
        }
        let chunk = self.chunks.entry(id.into()).or_default();
        let target = if part == "title" {
            &mut chunk.title
        } else {
            &mut chunk.body
        };
        let remaining = 1024usize.saturating_sub(target.chars().count());
        target.push_str(&text(&payload, remaining));
        if fields.get("d") != Some(&"0")
            && let Some(notice) = self.chunks.remove(id)
        {
            self.notice(notice.title, notice.body);
        }
    }
    fn reply_csi(&mut self, screen: &vt100::Screen, i1: Option<u8>, params: &[&[u16]], c: char) {
        let arg = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        match (i1, c, arg) {
            (None, 'c', 0) => self.reply(PRIMARY_DA.into()),
            (Some(b'>'), 'c', 0) => self.reply(SECONDARY_DA.into()),
            (None, 'n', 5) => self.reply("\x1b[0n".into()),
            (None, 'n', 6) => {
                let (row, col) = screen.cursor_position();
                self.reply(format!(
                    "\x1b[{};{}R",
                    row.saturating_add(1),
                    col.saturating_add(1)
                ));
            }
            (None, 't', 18) => {
                let (rows, cols) = screen.size();
                self.reply(format!("\x1b[8;{rows};{cols}t"));
            }
            // Do not advertise keyboard modes that the daemon parser does not track.
            _ => {}
        }
    }
    fn reply_decrqm(&mut self, screen: &vt100::Screen, params: &[&[u16]]) {
        let mode = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        let status = if matches!(mode, 1004 | 1007) {
            if self.extra_modes.get(&mode).copied().unwrap_or(mode == 1007) {
                1
            } else {
                2
            }
        } else {
            dec_mode_status(screen, mode)
        };
        self.reply(format!("\x1b[?{mode};{status}$y"));
    }
    fn reply_palette(&mut self, params: &[&[u8]]) {
        let Some(raw) = params.get(1) else {
            return;
        };
        let Ok(index) = String::from_utf8_lossy(raw).parse::<u8>() else {
            return;
        };
        let [r, g, b] = indexed_rgb(index);
        self.reply(format!("\x1b]4;{index};rgb:{}\x1b\\", rgb16(r, g, b)));
    }
    fn reply_dynamic_color(&mut self, channel: &[u8]) {
        let channel = text(channel, 2);
        let [r, g, b] = if channel == "11" {
            ANSI16[0]
        } else {
            ANSI16[7]
        };
        self.reply(format!("\x1b]{channel};rgb:{}\x1b\\", rgb16(r, g, b)));
    }
}

fn dec_mode_status(screen: &vt100::Screen, mode: u16) -> u8 {
    let mouse = screen.mouse_protocol_mode();
    let encoding = screen.mouse_protocol_encoding();
    let on = |yes| if yes { 1 } else { 2 };
    match mode {
        1 => on(screen.application_cursor()),
        25 => on(!screen.hide_cursor()),
        47 | 1047 | 1049 => on(screen.alternate_screen()),
        9 => on(mouse == MouseProtocolMode::Press),
        1000 => on(mouse == MouseProtocolMode::PressRelease),
        1002 => on(mouse == MouseProtocolMode::ButtonMotion),
        1003 => on(mouse == MouseProtocolMode::AnyMotion),
        1005 => on(encoding == MouseProtocolEncoding::Utf8),
        1006 => on(encoding == MouseProtocolEncoding::Sgr),
        2004 => on(screen.bracketed_paste()),
        _ => 0,
    }
}

fn indexed_rgb(index: u8) -> [u8; 3] {
    match index {
        0..=15 => ANSI16.get(usize::from(index)).copied().unwrap_or([0, 0, 0]),
        16..=231 => {
            let i = index.saturating_sub(16);
            let level = |n: u8| {
                if n == 0 {
                    0
                } else {
                    n.saturating_mul(40).saturating_add(55)
                }
            };
            [level(i / 36), level((i / 6) % 6), level(i % 6)]
        }
        232..=255 => {
            let value = index
                .saturating_sub(232)
                .saturating_mul(10)
                .saturating_add(8);
            [value, value, value]
        }
    }
}

fn rgb16(r: u8, g: u8, b: u8) -> String {
    let wide = |c: u8| u16::from(c) * 257;
    format!("{:04x}/{:04x}/{:04x}", wide(r), wide(g), wide(b))
}

impl vt100::Callbacks for Events {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        i1: Option<u8>,
        i2: Option<u8>,
        params: &[&[u16]],
        c: char,
    ) {
        match (i1, i2, c) {
            (Some(b'?'), None, 'h' | 'l') => {
                for param in params {
                    if let [mode @ (1004 | 1007)] = *param {
                        self.extra_modes.insert(*mode, c == 'h');
                    }
                }
            }
            (None, None, 'J') if first_param(params) == 3 => {
                if !screen.alternate_screen() {
                    self.scrollback_clear_pending = true;
                }
            }
            (Some(b'?'), Some(b'$'), 'p') => self.reply_decrqm(screen, params),
            (_, Some(_), _) => {}
            (i1, None, c) => self.reply_csi(screen, i1, params, c),
        }
    }
    fn paste_from_clipboard(&mut self, _: &mut vt100::Screen, selector: &[u8]) {
        self.reply(format!("\x1b]52;{};\x1b\\", text(selector, 16)));
    }
    fn unhandled_osc(&mut self, _: &mut vt100::Screen, params: &[&[u8]]) {
        match params.first().copied().unwrap_or_default() {
            b"9" if params.len() > 1 => {
                // OSC 9;4 is ConEmu progress, not a desktop notification.
                if params.len() > 2 && params.get(1) == Some(&b"4".as_slice()) {
                    return;
                }
                let Some(body) = params.get(1..) else {
                    return;
                };
                self.notice("Terminal".into(), text(&body.join(&b';'), 1024));
            }
            b"777" if params.len() >= 3 && params.get(1) == Some(&b"notify".as_slice()) => {
                let Some(title) = params.get(2) else {
                    return;
                };
                let body = params.get(3..).unwrap_or_default();
                self.notice(text(title, 256), text(&body.join(&b';'), 1024));
            }
            b"99" => self.kitty(params),
            b"10" | b"11" | b"12" if params.get(1) == Some(&b"?".as_slice()) => {
                let Some(channel) = params.first().copied() else {
                    return;
                };
                self.reply_dynamic_color(channel);
            }
            b"4" if params.len() == 3 && params.get(2) == Some(&b"?".as_slice()) => {
                self.reply_palette(params)
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
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
        let mut events = Events::default();
        events.set_scrollback_cap(100);
        let mut p = vt100::Parser::new_with_callbacks(24, 80, 100, events);
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
}
