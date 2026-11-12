//! Program Status Protocol (OSC 7501), display only.
//!
//! `OSC 7501 ; state=<s> [: key=value]… ST`, at most 4096 bytes. A program
//! reports `idle`, `working`, `done`, `blocked` or `error` (or `clear`),
//! with optional `kind`, `progress`, `app`, `title` and `msg` (base64
//! UTF-8). Only the root record (no `id`) is kept. The status is a pane
//! badge and, for `blocked` and `done`, a terminal notice. It never feeds
//! agent lifecycle, presence, Resume or agent notifications: hooks own
//! those.
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

/// The daemon parses OSC 7501 and publishes `State::program_status`.
pub const CAPABILITY: &str = "program-status-v1";
/// Longest accepted sequence payload.
pub const MAX_SEQUENCE: usize = 4096;
const MAX_APP: usize = 32;
const MAX_TITLE: usize = 192;
const MAX_MESSAGE: usize = 2048;
/// Reply to `OSC 7501 ; ?`.
pub const QUERY_REPLY: &str =
    "\x1b]7501;?:states=idle,working,done,blocked,error:kinds=permission,question,auth\x1b\\";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramState {
    Idle,
    Working,
    Done,
    Blocked,
    Error,
}

impl ProgramState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Working => "working",
            Self::Done => "done",
            Self::Blocked => "waiting",
            Self::Error => "error",
        }
    }

    /// Working and blocked end at the next shell prompt.
    pub const fn transient(self) -> bool {
        matches!(self, Self::Working | Self::Blocked)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramStatus {
    pub state: ProgramState,
    /// `permission`, `question` or `auth`; only with `blocked`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// 0–100 with `working` or `blocked`; `None` is indeterminate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// One OSC 7501 sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Report {
    Set(ProgramStatus),
    /// `state=clear` without `id`: drop the status.
    Clear,
    Query,
}

/// Parse the OSC parameters (`params[0]` is `7501`). `None` for another
/// code, a child record (`id`), or a report the spec says to discard.
pub fn parse(params: &[&[u8]]) -> Option<Report> {
    let (code, rest) = params.split_first()?;
    if *code != b"7501" {
        return None;
    }
    let payload = rest.join(&b';');
    if payload.len() > MAX_SEQUENCE {
        return None;
    }
    let payload = std::str::from_utf8(&payload).ok()?;
    if payload == "?" {
        return Some(Report::Query);
    }
    let mut fields = Fields::default();
    for pair in payload.split(':') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        if key.is_empty()
            || !key.bytes().all(|b| b.is_ascii_lowercase())
            || !value.bytes().all(value_byte)
        {
            continue;
        }
        fields.set(key, value);
    }
    fields.report()
}

/// Value bytes the spec allows: `A–Z a–z 0–9 _ . , + / = -`.
fn value_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"_.,+/=-".contains(&byte)
}

#[derive(Default)]
struct Fields<'a> {
    state: Option<&'a str>,
    id: Option<&'a str>,
    kind: Option<&'a str>,
    progress: Option<&'a str>,
    app: Option<&'a str>,
    title: Option<&'a str>,
    message: Option<&'a str>,
}

impl<'a> Fields<'a> {
    /// The last value of a key wins; unknown keys are ignored.
    fn set(&mut self, key: &str, value: &'a str) {
        let slot = match key {
            "state" => &mut self.state,
            "id" => &mut self.id,
            "kind" => &mut self.kind,
            "progress" => &mut self.progress,
            "app" => &mut self.app,
            "title" => &mut self.title,
            "msg" => &mut self.message,
            _ => return,
        };
        *slot = Some(value);
    }

    fn report(self) -> Option<Report> {
        if self.id.is_some() {
            return None;
        }
        let state = match self.state? {
            "idle" => ProgramState::Idle,
            "working" => ProgramState::Working,
            "done" => ProgramState::Done,
            "blocked" => ProgramState::Blocked,
            "error" => ProgramState::Error,
            "clear" => return Some(Report::Clear),
            _ => return None,
        };
        let kind = match self.kind {
            Some(kind @ ("permission" | "question" | "auth")) if state == ProgramState::Blocked => {
                Some(kind.to_owned())
            }
            _ => None,
        };
        let progress = match self.progress {
            Some(raw) if state.transient() => Some(raw.parse::<u8>().ok().filter(|p| *p <= 100)?),
            _ => None,
        };
        let app = match self.app {
            Some(app) if app.len() > MAX_APP => return None,
            app => app.map(str::to_owned),
        };
        Some(Report::Set(ProgramStatus {
            state,
            kind,
            progress,
            app,
            title: optional_text(self.title, MAX_TITLE).ok()?,
            message: optional_text(self.message, MAX_MESSAGE).ok()?,
        }))
    }
}

/// An absent text field is fine; an invalid one (`Err`) discards the
/// whole report.
fn optional_text(value: Option<&str>, limit: usize) -> Result<Option<String>, ()> {
    value.map_or(Ok(None), |value| decode(value, limit).map(Some))
}

/// Base64 UTF-8 text at most `limit` bytes, without control characters.
fn decode(value: &str, limit: usize) -> Result<String, ()> {
    let bytes = STANDARD.decode(value).map_err(|_| ())?;
    if bytes.len() > limit {
        return Err(());
    }
    let text = String::from_utf8(bytes).map_err(|_| ())?;
    if text.chars().any(char::is_control) {
        return Err(());
    }
    Ok(text.chars().filter(|c| !invisible(*c)).collect())
}

/// Bidi overrides and invisible formatting characters are stripped.
fn invisible(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
}

/// Short badge text: the state, progress, and the app when known.
pub fn badge(status: &ProgramStatus) -> String {
    let mut text = status.state.label().to_owned();
    if let Some(progress) = status.progress {
        text = format!("{text} {progress}%");
    }
    match &status.app {
        Some(app) => format!("{app}: {text}"),
        None => text,
    }
}

/// The terminal notice for a status, when it is one that asks for one.
pub fn notice(status: &ProgramStatus) -> Option<(String, String)> {
    let who = status.app.clone().unwrap_or_else(|| "Program".into());
    let title = match (status.state, status.kind.as_deref()) {
        (ProgramState::Blocked, Some(kind)) => format!("{who} is waiting ({kind})"),
        (ProgramState::Blocked, None) => format!("{who} is waiting"),
        (ProgramState::Done, _) => format!("{who} is done"),
        _ => return None,
    };
    let body = status
        .message
        .clone()
        .or_else(|| status.title.clone())
        .unwrap_or_default();
    Some((title, body))
}

#[cfg(test)]
#[path = "program_status_tests.rs"]
mod tests;
