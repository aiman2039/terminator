//! Keyboard selection in a terminal: hints mode (short labels on links,
//! paths, `file:line` and hashes on screen) and copy mode (vi-like cursor
//! and selection). While a mode is on, its pane's keys never reach the PTY:
//! [`take_mode_keys`] removes every keyboard event before the terminal
//! widget reads input.
use eframe::egui;
use egui_term::LinkTarget;

/// Label letters, home row first. Lower case only: Shift+label copies.
const LABEL_KEYS: &str = "asdfghjklqwertyuiopzxcvbnm";

/// One keyboard mode on one terminal.
#[derive(Clone, Debug)]
pub(crate) enum KeyMode {
    Hints(Hints),
    Copy { pending_g: bool },
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Hints {
    /// `(label, target)`, top to bottom.
    pub(crate) items: Vec<(String, LinkTarget)>,
    pub(crate) typed: String,
}

/// A hint picked for opening while its target resolves in the background.
#[derive(Clone, Debug)]
pub(crate) struct PendingHint {
    pub(crate) session: String,
    pub(crate) key: String,
    pub(crate) token: String,
    /// Pane coordinates of the token's first cell.
    pub(crate) rect: egui::Rect,
}

/// Keyboard mode state of the app: at most one mode at a time.
#[derive(Clone, Debug, Default)]
pub(crate) struct KeyModes {
    pub(crate) active: Option<(String, KeyMode)>,
    pub(crate) pending: Option<PendingHint>,
}

impl KeyModes {
    pub(crate) fn on(&self, sid: &str) -> Option<&KeyMode> {
        self.active
            .as_ref()
            .filter(|(owner, _)| owner == sid)
            .map(|(_, mode)| mode)
    }
}

/// A key typed in a keyboard mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModeKey {
    Char(char),
    /// Ctrl (or Command) with a letter.
    Ctrl(char),
    Named(egui::Key),
}

/// Remove every keyboard event (keys, text, paste, copy, cut) from
/// `events` and return the keys. Nothing is left for the terminal.
pub(crate) fn take_mode_keys(events: &mut Vec<egui::Event>) -> Vec<ModeKey> {
    let mut keys = Vec::new();
    events.retain(|event| match event {
        egui::Event::Text(text) => {
            keys.extend(text.chars().filter(|c| !c.is_control()).map(ModeKey::Char));
            false
        }
        egui::Event::Key {
            key,
            pressed,
            modifiers,
            ..
        } => {
            if *pressed {
                keys.extend(named_key(*key, *modifiers));
            }
            false
        }
        egui::Event::Paste(_) | egui::Event::Copy | egui::Event::Cut => false,
        _ => true,
    });
    keys
}

fn named_key(key: egui::Key, modifiers: egui::Modifiers) -> Option<ModeKey> {
    use egui::Key;
    if modifiers.ctrl || modifiers.command {
        let name = key.name();
        let mut chars = name.chars();
        return match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii_alphabetic() => {
                Some(ModeKey::Ctrl(c.to_ascii_lowercase()))
            }
            _ => None,
        };
    }
    matches!(
        key,
        Key::Escape
            | Key::Enter
            | Key::Backspace
            | Key::ArrowUp
            | Key::ArrowDown
            | Key::ArrowLeft
            | Key::ArrowRight
            | Key::PageUp
            | Key::PageDown
            | Key::Home
            | Key::End
    )
    .then_some(ModeKey::Named(key))
}

/// `count` distinct labels of one length, so none is a prefix of another.
pub(crate) fn labels(count: usize) -> Vec<String> {
    let keys: Vec<char> = LABEL_KEYS.chars().collect();
    if count <= keys.len() {
        return keys.iter().take(count).map(char::to_string).collect();
    }
    keys.iter()
        .flat_map(|first| keys.iter().map(move |second| format!("{first}{second}")))
        .take(count)
        .collect()
}

/// A token worth a hint: a link, URL, path, `file:line` or commit hash.
pub(crate) fn hintable(target: &LinkTarget) -> bool {
    let text = target.text.as_str();
    if target.hyperlink || text.contains("://") || text.starts_with("mailto:") {
        return true;
    }
    if text.chars().count() < 2 {
        return false;
    }
    text.contains('/') || text.starts_with('~') || file_name(text) || commit_hash(text)
}

/// `name.ext` or `name.ext:line[:column]`.
fn file_name(text: &str) -> bool {
    let name = text.split(':').next().unwrap_or_default();
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && (1..=8).contains(&extension.len())
        && extension.chars().all(|c| c.is_ascii_alphanumeric())
        && extension.chars().any(|c| c.is_ascii_alphabetic())
}

fn commit_hash(text: &str) -> bool {
    (7..=40).contains(&text.len())
        && text
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        && text.chars().any(|c| c.is_ascii_digit())
        && text.chars().any(|c| c.is_ascii_alphabetic())
}

/// What a typed key does in hints mode.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum HintStep {
    /// Keep waiting for more letters.
    Wait,
    Leave,
    /// Open (or, with Shift, copy) the item at this index.
    Pick {
        index: usize,
        copy: bool,
    },
}

impl Hints {
    pub(crate) fn new(targets: Vec<LinkTarget>) -> Self {
        let targets: Vec<LinkTarget> = targets.into_iter().filter(hintable).collect();
        let items = labels(targets.len()).into_iter().zip(targets).collect();
        Self {
            items,
            typed: String::new(),
        }
    }

    pub(crate) fn key(&mut self, key: ModeKey) -> HintStep {
        let c = match key {
            ModeKey::Named(egui::Key::Escape) => return HintStep::Leave,
            ModeKey::Named(egui::Key::Backspace) => {
                self.typed.pop();
                return HintStep::Wait;
            }
            ModeKey::Char(c) => c,
            _ => return HintStep::Wait,
        };
        let copy = c.is_uppercase();
        self.typed.push(c.to_ascii_lowercase());
        if let Some(index) = self
            .items
            .iter()
            .position(|(label, _)| *label == self.typed)
        {
            return HintStep::Pick { index, copy };
        }
        if !self
            .items
            .iter()
            .any(|(label, _)| label.starts_with(&self.typed))
        {
            self.typed.pop();
        }
        HintStep::Wait
    }
}

#[cfg(test)]
#[path = "key_modes_tests.rs"]
mod tests;
