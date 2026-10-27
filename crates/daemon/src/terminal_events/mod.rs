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
    parser.callbacks_mut().escape_pending = escaped;
}

/// Replay retained rows before restoring the visible screen. Only generated
/// screen state is sent; observational OSC effects are never replayed.
pub fn attachment_snapshot(parser: &vt100::Parser<Events>) -> Vec<u8> {
    let screen = parser.screen();
    let (rows, cols) = screen.size();
    let mut snapshot = Vec::new();
    let mut history = screen.clone();
    history.set_scrollback(10_000);
    if history.scrollback() > 0 {
        for offset in (1..=history.scrollback()).rev() {
            history.set_scrollback(offset);
            if let Some(row) = history.rows(0, cols).next() {
                snapshot.extend_from_slice(row.as_bytes());
                snapshot.extend_from_slice(b"\r\n");
            }
        }
        // The last screenful is still visible. Push it into history before
        // state_formatted() clears and replaces those rows.
        for _ in 1..rows {
            snapshot.extend_from_slice(b"\r\n");
        }
    }
    if screen.alternate_screen() {
        snapshot.extend_from_slice(b"\x1b[?1049h");
    }
    snapshot.extend(screen.state_formatted());
    snapshot.extend(parser.callbacks().modes_formatted());
    snapshot
}

fn text(bytes: &[u8], limit: usize) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect()
}
pub(crate) struct ReplyCsiInput<'a> {
    pub screen: &'a vt100::Screen,
    pub i1: Option<u8>,
    pub params: &'a [&'a [u16]],
    pub c: char,
}
impl Events {
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
    fn reply_csi(&mut self, input: ReplyCsiInput<'_>) {
        let ReplyCsiInput {
            screen,
            i1,
            params,
            c,
        } = input;
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
            (Some(b'?'), Some(b'$'), 'p') => self.reply_decrqm(screen, params),
            (_, Some(_), _) => {}
            (i1, None, c) => self.reply_csi(ReplyCsiInput {
                screen,
                i1,
                params,
                c,
            }),
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
#[path = "scrollback_tests.rs"]
mod scrollback_tests;
#[cfg(test)]
mod tests;
