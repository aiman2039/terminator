//! Terminal window titles (OSC 0/2) reported by the daemon per live session.
//! Titles are untrusted program output. They are display only: never a
//! lifecycle, notice, ntfy or notification source.
use crate::State;

/// The daemon fills `State::terminal_titles`.
pub const CAPABILITY: &str = "terminal-title-v1";
/// Longest title kept per session, in characters.
pub const LIMIT: usize = 120;

/// Lossy UTF-8 without control characters, trimmed and capped at [`LIMIT`].
#[must_use]
pub fn sanitize(bytes: &[u8]) -> String {
    let text: String = String::from_utf8_lossy(bytes)
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    text.trim().chars().take(LIMIT).collect()
}

/// The title to show for a live session, only when the daemon advertises
/// [`CAPABILITY`]. Older daemons never send one.
#[must_use]
pub fn for_session<'a>(state: &'a State, session: &str) -> Option<&'a str> {
    if !state.capabilities.iter().any(|c| c == CAPABILITY) {
        return None;
    }
    let live = state
        .sessions
        .iter()
        .any(|s| s.id == session && s.lifecycle.live());
    if !live {
        return None;
    }
    state
        .terminal_titles
        .get(session)
        .map(String::as_str)
        .filter(|t| !t.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live_state(capability: bool) -> State {
        let mut state: State = serde_json::from_value(serde_json::json!({
            "sessions": [{
                "id": "s", "project_id": "p", "label": "zsh", "cwd": "/",
                "kind": "shell", "file": null, "lifecycle": "running", "created": 0,
                "exit_code": null, "rows": 24, "cols": 80, "generation": "g",
                "pid": 1, "truncated": false, "cwd_confirmed": true
            }]
        }))
        .expect("state");
        if capability {
            state.capabilities.push(CAPABILITY.into());
        }
        state
            .terminal_titles
            .insert("s".into(), "vim main.rs".into());
        state
    }

    #[test]
    fn sanitizing_strips_control_characters_and_trims() {
        assert_eq!(sanitize(b"  a\x07b\x1b[c\r\n\t "), "ab[c");
        assert_eq!(
            sanitize("caf\u{e9} \u{2733}".as_bytes()),
            "caf\u{e9} \u{2733}"
        );
    }

    #[test]
    fn sanitizing_caps_long_titles_by_characters() {
        let long = "\u{e9}".repeat(LIMIT + 50);
        let title = sanitize(long.as_bytes());
        assert_eq!(title.chars().count(), LIMIT);
    }

    #[test]
    fn title_is_hidden_without_the_capability() {
        assert_eq!(for_session(&live_state(false), "s"), None);
        assert_eq!(for_session(&live_state(true), "s"), Some("vim main.rs"));
    }

    #[test]
    fn title_is_hidden_for_ended_sessions_and_empty_titles() {
        let mut state = live_state(true);
        state.terminal_titles.insert("s".into(), String::new());
        assert_eq!(for_session(&state, "s"), None);
        let mut state = live_state(true);
        state.sessions[0].lifecycle = crate::Lifecycle::Ended;
        assert_eq!(for_session(&state, "s"), None);
    }

    #[test]
    fn titles_from_older_daemons_deserialize_as_empty() {
        let state: State = serde_json::from_str("{}").expect("state");
        assert!(state.terminal_titles.is_empty());
    }
}
