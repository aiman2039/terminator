//! Clipboard sync: yank to the system clipboard, OSC 52 for terminals.
//!
//! Every unnamed-register write stages text for the host to copy (the GUI
//! view forwards it to the system clipboard, like `clipboard=unnamed`).
//! [`copy_sequence`] builds the OSC 52 escape for terminal contexts —
//! remote shells over SSH where no system clipboard exists. Terminal panes
//! (daemon PTYs) can adopt it later; the GUI never emits escapes itself.
//!
//! Sequences are capped: most terminals reject pastes past ~100 KiB, and
//! silently truncating a yank would corrupt it, so oversized yanks simply
//! produce no sequence.
use base64::Engine as _;

/// Yanked text over this size gets no OSC 52 sequence.
pub const MAX_OSC52_BYTES: usize = 100 * 1024;

/// OSC 52 set-clipboard sequence for `text` (`c` = system clipboard),
/// BEL-terminated. `None` for empty or oversized input.
#[must_use]
pub fn copy_sequence(text: &str) -> Option<String> {
    if text.is_empty() || text.len() > MAX_OSC52_BYTES {
        return None;
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(text);
    Some(format!("\x1b]52;c;{encoded}\x07"))
}

/// Parse a sequence built by [`copy_sequence`] back to its text.
/// `None` for anything else (foreign pastes never decode by accident).
#[must_use]
pub fn parse_sequence(sequence: &str) -> Option<String> {
    let payload = sequence
        .strip_prefix("\x1b]52;c;")
        .and_then(|rest| rest.strip_suffix('\x07'))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .ok()?;
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_round_trips() {
        let sequence = copy_sequence("hello").expect("sequence");
        assert!(sequence.starts_with("\x1b]52;c;"));
        assert!(sequence.ends_with('\x07'));
        assert_eq!(parse_sequence(&sequence).as_deref(), Some("hello"));
    }

    #[test]
    fn empty_and_oversized_yield_nothing() {
        assert_eq!(copy_sequence(""), None);
        let big = "x".repeat(MAX_OSC52_BYTES + 1);
        assert_eq!(copy_sequence(&big), None);
    }

    #[test]
    fn foreign_sequences_do_not_parse() {
        assert_eq!(parse_sequence("hello"), None);
        assert_eq!(parse_sequence("\x1b]52;c;!!!\x07"), None);
        assert_eq!(parse_sequence("\x1b]52;p;aGk=\x07"), None);
    }
}
