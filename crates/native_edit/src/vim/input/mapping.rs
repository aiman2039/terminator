//! `.vimrc` mappings: `map`, `noremap`, `nnoremap`, `inoremap`, … with
//! `<Esc>`/`<CR>`/`<C-x>`/`<leader>` notation. No timeout: a key that may
//! still grow into a mapping waits for the next key.
use super::super::mode::Key;

/// The mode a mapping applies in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapMode {
    Normal,
    Visual,
    OpPending,
    Insert,
}

/// One mapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mapping {
    modes: Vec<MapMode>,
    lhs: Vec<Key>,
    pub(crate) rhs: Vec<Key>,
    pub(crate) noremap: bool,
}

/// Every mapping from a vimrc, later definitions winning.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mappings {
    maps: Vec<Mapping>,
    leader: Option<String>,
}

impl Mappings {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.maps.is_empty()
    }

    /// The mapping for exactly `seq`, and whether a longer one starts
    /// with it.
    pub(crate) fn lookup(&self, mode: MapMode, seq: &[Key]) -> (Option<&Mapping>, bool) {
        let mut exact = None;
        let mut longer = false;
        for map in self.maps.iter().filter(|map| map.modes.contains(&mode)) {
            if map.lhs == seq {
                exact = Some(map);
            } else if map.lhs.len() > seq.len() && map.lhs.starts_with(seq) {
                longer = true;
            }
        }
        (exact, longer)
    }

    /// Take a vimrc line if it maps keys or sets `mapleader`; `true`
    /// when the line was one of those (valid or not).
    pub(crate) fn parse_line(&mut self, line: &str) -> bool {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("let mapleader") {
            self.leader = parse_leader(rest);
            return true;
        }
        let (cmd, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let Some((modes, noremap)) = map_command(cmd) else {
            return false;
        };
        let mut rest = rest.trim_start();
        loop {
            let lower = rest.to_ascii_lowercase();
            let Some(arg) = [
                "<silent>",
                "<buffer>",
                "<nowait>",
                "<unique>",
                "<special>",
                "<script>",
                "<expr>",
            ]
            .iter()
            .find(|arg| lower.starts_with(**arg)) else {
                break;
            };
            if *arg == "<expr>" {
                return true;
            }
            rest = rest.get(arg.len()..).unwrap_or_default().trim_start();
        }
        let (lhs, rhs) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let leader = self.leader.clone().unwrap_or_else(|| "\\".into());
        if let (Some(lhs), Some(rhs)) = (parse_keys(lhs, &leader), parse_keys(rhs.trim(), &leader))
            && !lhs.is_empty()
        {
            self.maps
                .retain(|map| !(map.lhs == lhs && map.modes == modes));
            self.maps.push(Mapping {
                modes,
                lhs,
                rhs,
                noremap,
            });
        }
        true
    }
}

/// Modes and `noremap` of a map command name (abbreviations included).
fn map_command(cmd: &str) -> Option<(Vec<MapMode>, bool)> {
    use MapMode::{Insert, Normal, OpPending, Visual};
    let (base, noremap) = match cmd {
        "map" | "nmap" | "nm" | "vmap" | "vm" | "xmap" | "xm" | "omap" | "om" | "imap" | "im" => {
            (cmd, false)
        }
        "noremap" | "no" => ("map", true),
        "nnoremap" | "nn" | "nno" => ("nmap", true),
        "vnoremap" | "vn" | "vno" | "xnoremap" | "xn" | "xno" => ("vmap", true),
        "onoremap" | "ono" => ("omap", true),
        "inoremap" | "ino" => ("imap", true),
        _ => return None,
    };
    let modes = match base.chars().next() {
        Some('n') => vec![Normal],
        Some('v' | 'x') => vec![Visual],
        Some('o') => vec![OpPending],
        Some('i') => vec![Insert],
        _ => vec![Normal, Visual, OpPending],
    };
    Some((modes, noremap))
}

/// `= ","`, `=" "`, `= "\<Space>"`.
fn parse_leader(rest: &str) -> Option<String> {
    let value = rest.trim().strip_prefix('=')?.trim();
    let inner = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))?;
    Some(match inner {
        "\\<Space>" | "\\<space>" => " ".into(),
        "\\\\" => "\\".into(),
        other => other.into(),
    })
}

/// Vim key notation to keys; `None` for keys the engine has no form for
/// (`<Plug>`, function keys), which drops the mapping.
pub(crate) fn parse_keys(text: &str, leader: &str) -> Option<Vec<Key>> {
    let mut keys = Vec::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if c == '<'
            && let Some(close) = rest.find('>')
            && close > 1
        {
            let name = rest.get(1..close).unwrap_or_default();
            if name.eq_ignore_ascii_case("leader") {
                keys.extend(leader.chars().map(Key::Char));
            } else if !name.eq_ignore_ascii_case("nop") {
                keys.push(special_key(name)?);
            }
            rest = rest.get(close.saturating_add(1)..).unwrap_or_default();
            continue;
        }
        keys.push(Key::Char(c));
        rest = rest.get(c.len_utf8()..).unwrap_or_default();
    }
    Some(keys)
}

fn special_key(name: &str) -> Option<Key> {
    let lower = name.to_ascii_lowercase();
    Some(match lower.as_str() {
        "esc" => Key::Escape,
        "cr" | "enter" | "return" => Key::Enter,
        "bs" | "backspace" => Key::Backspace,
        "tab" => Key::Tab,
        "s-tab" => Key::BackTab,
        "space" => Key::Char(' '),
        "lt" => Key::Char('<'),
        "bar" => Key::Char('|'),
        "bslash" => Key::Char('\\'),
        "del" => Key::Delete,
        "up" => Key::Up,
        "down" => Key::Down,
        "left" => Key::Left,
        "right" => Key::Right,
        "home" => Key::Home,
        "end" => Key::End,
        "c-v" => Key::CtrlV,
        "c-^" | "c-6" => Key::CtrlCaret,
        "c-[" => Key::Escape,
        _ => {
            let ctrl = lower.strip_prefix("c-")?;
            let mut chars = ctrl.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_alphabetic() => Key::Ctrl(c),
                _ => return None,
            }
        }
    })
}
