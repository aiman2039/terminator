//! Macros: `q{reg}` records typed input, `q` stops, `@{reg}` replays,
//! `@@` repeats the last one and `@:` the last ex command.
use super::super::super::doc::Buffer;
use super::super::mode::{Effect, Input, Key};
use super::super::state::VimEngine;
use super::MAX_DEPTH;

impl VimEngine {
    /// `q{a-z}` starts recording (uppercase appends).
    pub(crate) fn macro_start(&mut self, reg: char) -> Vec<Effect> {
        if !reg.is_ascii_alphanumeric() {
            return self.finish_simple(false);
        }
        let key = reg.to_ascii_lowercase();
        let keys = if reg.is_ascii_uppercase() {
            self.x.macros.get(&key).cloned().unwrap_or_default()
        } else {
            Vec::new()
        };
        self.x.recording_macro = Some((key, keys));
        self.finish_simple(true)
    }

    /// `q` while recording: store the macro without that final `q`.
    pub(crate) fn macro_stop(&mut self) {
        let Some((reg, mut keys)) = self.x.recording_macro.take() else {
            return;
        };
        if matches!(keys.last(), Some(Input::Key(Key::Char('q'))))
            || matches!(keys.last(), Some(Input::Text(t)) if t == "q")
        {
            keys.pop();
        }
        self.x.macros.insert(reg, keys);
    }

    /// The register being recorded into, for a status line.
    #[must_use]
    pub fn recording_macro(&self) -> Option<char> {
        self.x.recording_macro.as_ref().map(|(reg, _)| *reg)
    }

    /// `@{reg}` `count` times; `@@` the last one; `@:` the last ex line.
    pub(crate) fn macro_play<B: Buffer>(
        &mut self,
        doc: &mut B,
        reg: char,
        count: usize,
    ) -> Vec<Effect> {
        self.clear_pending();
        self.rec_discard();
        if reg == ':' {
            let Some(line) = self.x.last_ex.clone() else {
                return vec![Effect::Bell];
            };
            let mut effects = Vec::new();
            for _ in 0..count.max(1) {
                self.begin_ex(&line);
                effects.extend(self.run_command_line(doc));
            }
            return effects;
        }
        let reg = if reg == '@' {
            match self.x.last_macro {
                Some(reg) => reg,
                None => return vec![Effect::Bell],
            }
        } else {
            reg.to_ascii_lowercase()
        };
        let Some(keys) = self.x.macros.get(&reg).cloned() else {
            return vec![Effect::Bell];
        };
        if self.x.replay_depth >= MAX_DEPTH {
            return vec![Effect::Bell];
        }
        self.x.last_macro = Some(reg);
        self.x.replay_depth = self.x.replay_depth.saturating_add(1);
        let mut effects = Vec::new();
        for _ in 0..count.max(1) {
            for input in keys.clone() {
                effects.extend(self.feed(doc, input));
            }
        }
        self.x.replay_depth = self.x.replay_depth.saturating_sub(1);
        effects
    }
}
