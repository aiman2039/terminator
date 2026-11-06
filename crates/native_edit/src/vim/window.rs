//! Window and jump keys that hand over to the GUI: Ctrl-W pane commands,
//! and Ctrl-O / Ctrl-I past the ends of this buffer's jump list.
use super::super::doc::Buffer;
use super::mode::{Effect, PaneMove};
use super::state::VimEngine;

impl VimEngine {
    /// `Ctrl-W x`: pane focus, splits and close; the GUI owns panes.
    pub(super) fn window_command(&mut self, ch: char) -> Vec<Effect> {
        let effect = match ch {
            'h' => Effect::FocusPane(PaneMove::Left),
            'l' => Effect::FocusPane(PaneMove::Right),
            'k' => Effect::FocusPane(PaneMove::Up),
            'j' => Effect::FocusPane(PaneMove::Down),
            'w' => Effect::FocusPane(PaneMove::Next),
            's' | 'S' => Effect::Split,
            'v' => Effect::Vsplit,
            'q' | 'c' => Effect::Quit,
            _ => return self.finish_simple(false),
        };
        self.finish_simple(true);
        vec![effect]
    }

    /// Ctrl-O (`older`) / Ctrl-I: walk this buffer's jumps; past either
    /// end, the GUI's Back/Forward history carries on into other files,
    /// as vim's jumplist does.
    pub(super) fn walk_jumps_or_history<B: Buffer>(
        &mut self,
        doc: &B,
        older: bool,
        count: usize,
    ) -> Vec<Effect> {
        let walked = self.walk_jumps(doc, older, count);
        self.finish_simple(true);
        match (walked, older) {
            (true, _) => Vec::new(),
            (false, true) => vec![Effect::JumpBack],
            (false, false) => vec![Effect::JumpForward],
        }
    }
}
