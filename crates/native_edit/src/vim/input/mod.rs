//! Input plumbing between the view and the modes: vimrc mappings, macro
//! capture, the key record behind `.`, and key-by-key replay.
mod macros;
pub(crate) mod mapping;

use super::super::doc::Buffer;
use super::mode::{Effect, Input, Key, Mode};
use super::state::VimEngine;
use mapping::{MapMode, Mappings};

/// Nested mapping / macro expansion limit (a recursive `map` or `@a`
/// inside `qa` stops here instead of looping).
const MAX_DEPTH: usize = 50;

/// `inputs` with its leading count replaced by `count` (`3.`).
pub(crate) fn with_count(inputs: Vec<Input>, count: usize) -> Vec<Input> {
    let skip = inputs
        .iter()
        .take_while(|input| matches!(input, Input::Key(Key::Char(c)) if c.is_ascii_digit()))
        .count();
    count
        .to_string()
        .chars()
        .map(|c| Input::Key(Key::Char(c)))
        .chain(inputs.into_iter().skip(skip))
        .collect()
}

/// Single typed chars match mappings like keys do.
fn as_key(input: &Input) -> Option<Key> {
    match input {
        Input::Key(key) => Some(*key),
        Input::Text(text) => {
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Some(Key::Char(c)),
                _ => None,
            }
        }
    }
}

impl VimEngine {
    /// Entry for every key and typed text: macro capture, then mappings,
    /// then the mode handlers.
    pub(super) fn feed<B: Buffer>(&mut self, doc: &mut B, input: Input) -> Vec<Effect> {
        if self.modeless {
            return self.dispatch(doc, input);
        }
        if self.x.replay_depth == 0
            && let Some((_, keys)) = self.x.recording_macro.as_mut()
        {
            keys.push(input.clone());
        }
        let Some(maps) = self.options.maps.clone() else {
            return self.dispatch(doc, input);
        };
        if as_key(&input).is_none() {
            let mut effects = self.flush_maps(doc);
            effects.extend(self.dispatch(doc, input));
            return effects;
        }
        if self.x.map_pending.is_empty() && self.map_mode().is_none() {
            return self.dispatch(doc, input);
        }
        self.x.map_pending.push(input);
        self.drain_maps(doc, &maps)
    }

    /// Which mapping table applies now; `None` while a command waits for
    /// a literal char (`f`, `r`, `m`, …) or on the command line.
    fn map_mode(&self) -> Option<MapMode> {
        if self.x.prefix.is_some() {
            return None;
        }
        match self.mode {
            Mode::Insert => Some(MapMode::Insert),
            Mode::Normal if self.pending.is_some() => Some(MapMode::OpPending),
            Mode::Normal => Some(MapMode::Normal),
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock => Some(MapMode::Visual),
            Mode::Command => None,
        }
    }

    /// Resolve held keys: wait while they may grow into a mapping, expand
    /// a complete one, else pass the first key through and retry.
    fn drain_maps<B: Buffer>(&mut self, doc: &mut B, maps: &Mappings) -> Vec<Effect> {
        let mut effects = Vec::new();
        while !self.x.map_pending.is_empty() {
            let Some(mode) = self.map_mode() else {
                let first = self.x.map_pending.remove(0);
                effects.extend(self.dispatch(doc, first));
                continue;
            };
            let seq: Vec<Key> = self.x.map_pending.iter().filter_map(as_key).collect();
            let (exact, longer) = maps.lookup(mode, &seq);
            if longer {
                break;
            }
            if let Some(map) = exact {
                self.x.map_pending.clear();
                effects.extend(self.run_rhs(doc, &map.rhs, map.noremap));
                continue;
            }
            let prefix = (1..seq.len())
                .rev()
                .find_map(|n| maps.lookup(mode, seq.get(..n)?).0.map(|map| (n, map)));
            if let Some((n, map)) = prefix {
                self.x.map_pending.drain(..n);
                effects.extend(self.run_rhs(doc, &map.rhs, map.noremap));
            } else {
                let first = self.x.map_pending.remove(0);
                effects.extend(self.dispatch(doc, first));
            }
        }
        effects
    }

    /// Pass held keys through unmapped (before multi-char text).
    fn flush_maps<B: Buffer>(&mut self, doc: &mut B) -> Vec<Effect> {
        let held = std::mem::take(&mut self.x.map_pending);
        let mut effects = Vec::new();
        for input in held {
            effects.extend(self.dispatch(doc, input));
        }
        effects
    }

    /// Feed a mapping's right-hand side (`noremap`: no further mapping).
    fn run_rhs<B: Buffer>(&mut self, doc: &mut B, rhs: &[Key], noremap: bool) -> Vec<Effect> {
        let mut effects = Vec::new();
        if self.x.replay_depth >= MAX_DEPTH {
            return vec![Effect::Bell];
        }
        self.x.replay_depth = self.x.replay_depth.saturating_add(1);
        for key in rhs {
            // Chars type in Insert mode, exactly like keyboard text.
            let input = match key {
                Key::Char(c) if self.mode == Mode::Insert => Input::Text(c.to_string()),
                _ => Input::Key(*key),
            };
            effects.extend(if noremap {
                self.dispatch(doc, input)
            } else {
                self.feed(doc, input)
            });
        }
        self.x.replay_depth = self.x.replay_depth.saturating_sub(1);
        effects
    }

    /// Hand one input to the current mode, recording it for `.`.
    pub(super) fn dispatch<B: Buffer>(&mut self, doc: &mut B, input: Input) -> Vec<Effect> {
        if !self.replaying && !self.modeless {
            if self.at_command_start() {
                self.x.raw.clear();
                self.x.keys_change = false;
            }
            self.x.raw.push(input.clone());
            if let Some(oneshot) = self.x.oneshot.as_mut() {
                oneshot.2 = true;
            }
        }
        let effects = match input {
            Input::Key(key) => self.dispatch_key(doc, key),
            Input::Text(text) => self.dispatch_text(doc, &text),
        };
        if !self.replaying {
            self.resume_oneshot(doc);
        }
        effects
    }

    fn dispatch_key<B: Buffer>(&mut self, doc: &mut B, key: Key) -> Vec<Effect> {
        match self.mode {
            Mode::Normal => self.normal_key(doc, key),
            Mode::Insert => self.insert_key(doc, key),
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock => self.visual_key(doc, key),
            Mode::Command => self.command_key(doc, key),
        }
    }

    /// Typed text: Insert types it; other modes read it as keys (the view
    /// sends printable chars as text so any keyboard layout works).
    fn dispatch_text<B: Buffer>(&mut self, doc: &mut B, text: &str) -> Vec<Effect> {
        if self.mode == Mode::Insert {
            return self.insert_text(doc, text);
        }
        let mut effects = Vec::new();
        for (i, c) in text.char_indices() {
            if self.mode == Mode::Insert {
                effects.extend(self.insert_text(doc, text.get(i..).unwrap_or_default()));
                break;
            }
            effects.extend(self.dispatch_key(doc, Key::Char(c)));
        }
        effects
    }

    /// Replay recorded inputs (`.` of a [`super::mode::Repeat::Keys`]
    /// change) as one undo step; the caller sets `replaying`.
    pub(super) fn replay_keys<B: Buffer>(&mut self, doc: &mut B, inputs: Vec<Input>) {
        doc.begin_undo_group();
        for input in inputs {
            let _ = match input {
                Input::Key(key) => self.dispatch_key(doc, key),
                Input::Text(text) => self.dispatch_text(doc, &text),
            };
        }
        if self.mode != Mode::Normal {
            let _ = self.dispatch_key(doc, Key::Escape);
        }
        doc.end_undo_group();
    }

    /// After Insert Ctrl-O's one command, return to Insert.
    fn resume_oneshot<B: Buffer>(&mut self, doc: &mut B) {
        let Some((insert_at, clamped, started)) = self.x.oneshot else {
            return;
        };
        if self.mode == Mode::Insert {
            self.x.oneshot = None;
            return;
        }
        if !started || !self.at_command_start() {
            return;
        }
        self.x.oneshot = None;
        if self.cursor == clamped {
            self.cursor = insert_at;
        }
        self.begin_insert_session(doc);
        self.clamp(doc);
    }
}
