//! Inbox key handling as a pure state machine. A reply reaches a session
//! only from the confirm step: `s` sends it with Enter, `t` types it
//! without Enter. No other key sends text.
use super::rows::{Notice, Row};

/// Longest reply line, in characters.
const MAX_REPLY: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    List,
    /// Read-only screen view of a session.
    Screen {
        session: String,
    },
    /// Typing a one-line reply.
    Reply {
        session: String,
        line: String,
    },
    /// The reply is complete; nothing is sent until `s` or `t`.
    Confirm {
        session: String,
        line: String,
    },
}

/// What the loop must do after a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Effect {
    None,
    Quit,
    OpenScreen(String),
    Send {
        session: String,
        text: String,
        enter: bool,
    },
    DismissAgent(String),
    DismissTerminal(String),
    Snooze(String),
}

/// A key, reduced to what the inbox reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Key {
    Char(char),
    Enter,
    Esc,
    Backspace,
    Up,
    Down,
}

pub(super) struct Inbox {
    pub(super) mode: Mode,
    pub(super) selected: usize,
    pub(super) status: String,
}

impl Inbox {
    pub(super) fn new() -> Self {
        Self {
            mode: Mode::List,
            selected: 0,
            status: String::new(),
        }
    }

    pub(super) fn key(&mut self, key: Key, rows: &[Row]) -> Effect {
        match std::mem::replace(&mut self.mode, Mode::List) {
            Mode::List => self.list_key(key, rows),
            Mode::Screen { session } => self.screen_key(key, session),
            Mode::Reply { session, line } => self.reply_key(key, session, line),
            Mode::Confirm { session, line } => self.confirm_key(key, session, line),
        }
    }

    fn list_key(&mut self, key: Key, rows: &[Row]) -> Effect {
        let row = rows.get(self.selected);
        match (key, row) {
            (Key::Char('q') | Key::Esc, _) => Effect::Quit,
            (Key::Up | Key::Char('k'), _) => {
                self.selected = self.selected.saturating_sub(1);
                Effect::None
            }
            (Key::Down | Key::Char('j'), _) => {
                self.selected = self
                    .selected
                    .saturating_add(1)
                    .min(rows.len().saturating_sub(1));
                Effect::None
            }
            (Key::Enter, Some(row)) => Effect::OpenScreen(row.session.clone()),
            (Key::Char('r'), Some(row)) => {
                self.start_reply(row.session.clone());
                Effect::None
            }
            (Key::Char('d'), Some(row)) => match &row.notice {
                Some(Notice::Agent { id, .. }) => Effect::DismissAgent(id.clone()),
                Some(Notice::Terminal { id, .. }) => Effect::DismissTerminal(id.clone()),
                None => self.note("No notice to dismiss"),
            },
            (Key::Char('s'), Some(row)) => match &row.notice {
                Some(Notice::Agent { id, .. }) => Effect::Snooze(id.clone()),
                _ => self.note("Only agent notices can be snoozed"),
            },
            _ => Effect::None,
        }
    }

    fn screen_key(&mut self, key: Key, session: String) -> Effect {
        match key {
            Key::Esc | Key::Char('q') => Effect::None,
            Key::Char('r') => {
                self.start_reply(session);
                Effect::None
            }
            Key::Enter => {
                self.mode = Mode::Screen {
                    session: session.clone(),
                };
                Effect::OpenScreen(session)
            }
            _ => {
                self.mode = Mode::Screen { session };
                Effect::None
            }
        }
    }

    fn reply_key(&mut self, key: Key, session: String, mut line: String) -> Effect {
        match key {
            Key::Esc => return self.note("Reply discarded"),
            Key::Enter if !line.is_empty() => {
                self.mode = Mode::Confirm { session, line };
                return Effect::None;
            }
            Key::Backspace => {
                line.pop();
            }
            Key::Char(c) if !c.is_control() && line.chars().count() < MAX_REPLY => line.push(c),
            _ => {}
        }
        self.mode = Mode::Reply { session, line };
        Effect::None
    }

    fn confirm_key(&mut self, key: Key, session: String, line: String) -> Effect {
        match key {
            Key::Char('s') => Effect::Send {
                session,
                text: line,
                enter: true,
            },
            Key::Char('t') => Effect::Send {
                session,
                text: line,
                enter: false,
            },
            Key::Esc => {
                self.mode = Mode::Reply { session, line };
                Effect::None
            }
            _ => {
                self.mode = Mode::Confirm { session, line };
                Effect::None
            }
        }
    }

    fn start_reply(&mut self, session: String) {
        self.mode = Mode::Reply {
            session,
            line: String::new(),
        };
    }

    fn note(&mut self, text: &str) -> Effect {
        text.clone_into(&mut self.status);
        Effect::None
    }

    /// Keeps the selection on a row after a refresh.
    pub(super) fn clamp(&mut self, rows: usize) {
        self.selected = self.selected.min(rows.saturating_sub(1));
    }
}
