//! Kitty keyboard protocol flags (`kitty-keyboard-v1`): the daemon keeps
//! the progressive-enhancement stack per session, answers `CSI ? u`, and
//! replays the stack on reattach so a new GUI encodes keys the same way.
//! The GUI terminal widget does the key encoding.

/// The daemon answers `CSI ? u` and replays the flags on reattach.
pub const CAPABILITY: &str = "kitty-keyboard-v1";
/// Flags the GUI encodes: 1 is "disambiguate escape codes". Pushed flags
/// are masked to these, so `CSI ? u` reports what keys really carry.
pub const SUPPORTED: u16 = 1;
/// Stack entries kept; the oldest is dropped first (the spec allows a
/// terminal to bound the stack).
const MAX_DEPTH: usize = 16;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KittyKeys {
    stack: Vec<u16>,
}

impl KittyKeys {
    /// Active flags: the top of the stack.
    pub fn flags(&self) -> u16 {
        self.stack.last().copied().unwrap_or(0)
    }

    /// Apply one `CSI <marker> … u`. Returns the reply for a query.
    pub fn apply(&mut self, marker: u8, params: &[&[u16]]) -> Option<String> {
        let first = params.first().and_then(|p| p.first()).copied();
        match marker {
            b'?' => return Some(format!("\x1b[?{}u", self.flags())),
            b'>' => {
                if self.stack.len() >= MAX_DEPTH {
                    self.stack.remove(0);
                }
                self.stack.push(first.unwrap_or(0) & SUPPORTED);
            }
            b'<' => {
                let count = usize::from(first.unwrap_or(1).max(1));
                let keep = self.stack.len().saturating_sub(count);
                self.stack.truncate(keep);
            }
            b'=' => {
                let flags = first.unwrap_or(0) & SUPPORTED;
                let mode = params.get(1).and_then(|p| p.first()).copied().unwrap_or(1);
                let current = self.flags();
                let next = match mode {
                    1 => flags,
                    2 => current | flags,
                    3 => current & !flags,
                    _ => return None,
                };
                match self.stack.last_mut() {
                    Some(top) => *top = next,
                    None => self.stack.push(next),
                }
            }
            _ => {}
        }
        None
    }

    pub fn clear(&mut self) {
        self.stack.clear();
    }

    /// Pushes that rebuild the stack in a fresh terminal.
    pub fn replay(&self) -> Vec<u8> {
        self.stack
            .iter()
            .flat_map(|flags| format!("\x1b[>{flags}u").into_bytes())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_query_pop_follow_the_stack() {
        let mut keys = KittyKeys::default();
        assert_eq!(keys.apply(b'?', &[]).as_deref(), Some("\x1b[?0u"));
        keys.apply(b'>', &[&[1]]);
        assert_eq!(keys.apply(b'?', &[]).as_deref(), Some("\x1b[?1u"));
        // Unsupported bits (report events, all keys as escapes) are masked.
        keys.apply(b'>', &[&[31]]);
        assert_eq!(keys.flags(), 1);
        keys.apply(b'<', &[]);
        keys.apply(b'<', &[&[5]]);
        assert_eq!(keys.apply(b'?', &[]).as_deref(), Some("\x1b[?0u"));
    }

    #[test]
    fn set_modes_replace_add_and_remove_flags() {
        let mut keys = KittyKeys::default();
        keys.apply(b'=', &[&[1]]);
        assert_eq!(keys.flags(), 1);
        keys.apply(b'=', &[&[1], &[3]]);
        assert_eq!(keys.flags(), 0);
        keys.apply(b'=', &[&[1], &[2]]);
        assert_eq!(keys.flags(), 1);
        // An unknown mode changes nothing.
        keys.apply(b'=', &[&[0], &[9]]);
        assert_eq!(keys.flags(), 1);
    }

    #[test]
    fn the_stack_is_bounded_and_replays_as_pushes() {
        let mut keys = KittyKeys::default();
        for _ in 0..100 {
            keys.apply(b'>', &[&[1]]);
        }
        assert_eq!(keys.stack.len(), MAX_DEPTH);
        let replay = keys.replay();
        let mut fresh = KittyKeys::default();
        for _ in 0..MAX_DEPTH {
            fresh.apply(b'>', &[&[1]]);
        }
        assert_eq!(fresh, keys);
        assert!(replay.starts_with(b"\x1b[>1u"));
        keys.clear();
        assert!(keys.replay().is_empty());
    }
}
