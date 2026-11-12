//! Caps OSC strings before the daemon's VTE parsers. vte (with `std`) keeps
//! an OSC in an unbounded buffer until BEL, CAN, SUB or ESC, so a stray
//! `ESC ]` followed by endless text would grow daemon memory without limit.
//! Only the parser input is filtered; history and attached clients still get
//! the raw bytes.
use std::borrow::Cow;

/// Longest OSC payload the parsers see. The daemon reads titles, notices and
/// shell marks from OSC; none needs more.
pub(crate) const MAX_OSC: usize = 64 * 1024;

const BEL: u8 = 0x07;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;
const ESC: u8 = 0x1b;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Phase {
    #[default]
    Ground,
    Escape,
    Osc(usize),
    /// The OSC was cancelled; drop bytes until it would have ended.
    Dropping,
}

#[derive(Debug, Default)]
pub(crate) struct OscLimit {
    phase: Phase,
}

impl OscLimit {
    /// Returns the bytes to feed the parsers: unchanged unless an OSC passed
    /// `MAX_OSC`, which is then ended with CAN and the rest of it dropped.
    pub(crate) fn filter<'a>(&mut self, bytes: &'a [u8]) -> Cow<'a, [u8]> {
        let mut out: Option<Vec<u8>> = None;
        for (index, byte) in bytes.iter().copied().enumerate() {
            let (next, emit) = step(self.phase, byte);
            self.phase = next;
            if out.is_none() && emit != Some(byte) {
                out = Some(bytes.get(..index).unwrap_or_default().to_vec());
            }
            if let (Some(out), Some(emit)) = (out.as_mut(), emit) {
                out.push(emit);
            }
        }
        out.map_or(Cow::Borrowed(bytes), Cow::Owned)
    }
}

/// One byte of the VTE state machine, reduced to what matters for OSC
/// length: the next phase and the byte to pass on (if any).
fn step(phase: Phase, byte: u8) -> (Phase, Option<u8>) {
    match (phase, byte) {
        (Phase::Dropping, ESC) => (Phase::Escape, Some(ESC)),
        (Phase::Dropping, BEL | CAN | SUB) => (Phase::Ground, None),
        (Phase::Dropping, _) => (Phase::Dropping, None),
        (_, ESC) => (Phase::Escape, Some(ESC)),
        (_, CAN | SUB) => (Phase::Ground, Some(byte)),
        (Phase::Escape, b']') => (Phase::Osc(0), Some(byte)),
        // vte stays in Escape on C0 controls, DEL and every byte >= 0x80.
        (Phase::Escape, 0x00..=0x17 | 0x19 | 0x1c..=0x1f | 0x7f..=0xff) => {
            (Phase::Escape, Some(byte))
        }
        (Phase::Osc(_), BEL) => (Phase::Ground, Some(byte)),
        (Phase::Osc(len), _) if len >= MAX_OSC => (Phase::Dropping, Some(CAN)),
        (Phase::Osc(len), _) => (Phase::Osc(len.saturating_add(1)), Some(byte)),
        (Phase::Ground | Phase::Escape, _) => (Phase::Ground, Some(byte)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_output_and_short_osc_pass_through_unchanged() {
        let mut limit = OscLimit::default();
        let data = b"ls\r\n\x1b]0;title\x07\x1b[31mred\x1b]133;A\x1b\\";
        assert!(matches!(limit.filter(data), Cow::Borrowed(_)));
    }

    #[test]
    fn an_unterminated_osc_stops_growing_the_parser_input_after_the_limit() {
        let mut limit = OscLimit::default();
        let mut fed = limit.filter(b"\x1b]").len();
        for _ in 0..100 {
            fed += limit.filter(&[b'a'; 8192]).len();
        }
        assert_eq!(fed, 2 + MAX_OSC + 1, "OSC capped, then one CAN");
        let after = limit.filter(b"more\x07prompt$ ");
        assert_eq!(after.as_ref(), b"prompt$ ");
    }

    #[test]
    fn del_or_a_high_byte_after_escape_does_not_bypass_the_limit() {
        for gap in [0x7f_u8, 0x80, 0xc3, 0xff] {
            let mut limit = OscLimit::default();
            let mut fed = limit.filter(&[ESC, gap, b']']).len();
            for _ in 0..100 {
                fed += limit.filter(&[b'a'; 8192]).len();
            }
            assert_eq!(fed, 3 + MAX_OSC + 1, "{gap:#x}");
        }
    }

    #[test]
    fn the_escape_phase_matches_the_vte_parser() {
        struct Osc(usize);
        impl vte::Perform for Osc {
            fn osc_dispatch(&mut self, _: &[&[u8]], _: bool) {
                self.0 += 1;
            }
        }
        for gap in [0x7f_u8, 0x80, 0xff] {
            let data = [ESC, gap, b']', b'0', b';', b't', BEL];
            let mut parser = vte::Parser::new();
            let mut osc = Osc(0);
            parser.advance(&mut osc, &data);
            assert_eq!(osc.0, 1, "vte opens an OSC after ESC {gap:#x}");
            let mut limit = OscLimit::default();
            limit.filter(&data[..3]);
            assert_eq!(limit.phase, Phase::Osc(0), "{gap:#x}");
        }
    }

    #[test]
    fn escape_after_a_dropped_osc_starts_the_next_sequence() {
        let mut limit = OscLimit::default();
        limit.filter(b"\x1b]");
        limit.filter(&vec![b'x'; MAX_OSC + 10]);
        assert_eq!(limit.filter(b"zz\x1b[1mB").as_ref(), b"\x1b[1mB");
    }
}
