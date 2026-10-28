//! Command-line mode: `/` search, `:set`, `:s` and the ex command table.
use super::super::doc::Buffer;
use super::mode::Effect;
use super::mode::Key;
use super::operators::JumpToMatchInput;
use super::state::VimEngine;

impl VimEngine {
    pub(super) fn command_key<B: Buffer>(&mut self, doc: &mut B, key: Key) -> Vec<Effect> {
        match key {
            Key::Escape => {
                self.command.clear();
                self.enter_normal(doc);
                self.rec_discard();
            }
            Key::Enter => return self.run_command_line(doc),
            Key::Backspace => {
                self.command.pop();
            }
            Key::Char(c) => {
                self.command.push(c);
            }
            _ => return vec![Effect::Bell],
        }
        Vec::new()
    }

    fn run_command_line<B: Buffer>(&mut self, doc: &mut B) -> Vec<Effect> {
        // Search and ex commands never join the `.` recording.
        self.rec_discard();
        if self.command.starts_with('/') {
            return self.run_search(doc);
        }
        // Ex commands are not `.`-repeatable; the in-progress change (if
        // any) is dropped, the last change keeps.
        self.rec_discard();
        if Self::is_substitute(&self.command) {
            let command = self.command.clone();
            let outcome = self.substitute(doc, &command);
            self.command.clear();
            self.enter_normal(doc);
            return outcome;
        }
        if let Some(args) = self.set_args() {
            return self.run_set(doc, &args);
        }
        let effects = Self::ex_effects(&self.command);
        let failed = effects == vec![Effect::Bell];
        self.command.clear();
        self.enter_normal(doc);
        if failed {
            return vec![Effect::Bell];
        }
        effects
    }

    /// Vim `/pattern`: jump to the next match, wrapping. An empty pattern
    /// repeats the last search.
    fn run_search<B: Buffer>(&mut self, doc: &mut B) -> Vec<Effect> {
        let pattern = if self.command.len() > 1 {
            self.command.get(1..).unwrap_or("").to_string()
        } else {
            self.search_last.clone()
        };
        if pattern.is_empty() {
            return vec![Effect::Bell];
        }
        self.search_last.clone_from(&pattern);
        let from = self.char_idx(doc, self.cursor);
        self.command.clear();
        self.enter_normal(doc);
        if !self.jump_to_match(
            doc,
            JumpToMatchInput {
                pattern: &pattern,
                from,
                forward: true,
            },
        ) {
            return vec![Effect::Bell];
        }
        Vec::new()
    }

    /// The arguments of `:set …` / `:setlocal …` (empty for the bare form).
    fn set_args(&self) -> Option<String> {
        ["set", "setlocal"].iter().find_map(|name| {
            if self.command == *name {
                Some(String::new())
            } else {
                self.command
                    .strip_prefix(&format!("{name} "))
                    .map(str::to_owned)
            }
        })
    }

    /// `:set` switches the view (wrap) and search (ignorecase); unknown
    /// names bell instead of silently succeeding.
    fn run_set<B: Buffer>(&mut self, doc: &mut B, args: &str) -> Vec<Effect> {
        // Bare `:set` lists options in vim; here it is a no-op rather than
        // an error.
        let known = args.trim().is_empty() || self.apply_set_args(args);
        self.command.clear();
        self.enter_normal(doc);
        if known {
            Vec::new()
        } else {
            vec![Effect::Bell]
        }
    }

    fn ex_effects(command: &str) -> Vec<Effect> {
        match command {
            "w" => vec![Effect::Save],
            "w!" => vec![Effect::SaveForce],
            "q" => vec![Effect::Quit],
            "q!" => vec![Effect::QuitForce],
            // `:qw` is not real vim, but listed next to `:wq` so often
            // that accepting it avoids a dead end.
            "wq" | "qw" | "x" => vec![Effect::WriteQuit { force: false }],
            "wq!" | "qw!" | "x!" => vec![Effect::WriteQuit { force: true }],
            "qa" => vec![Effect::QuitAll { force: false }],
            "qa!" => vec![Effect::QuitAll { force: true }],
            // Workspace layout stays with the GUI: the engine only asks,
            // via effects the session layer maps to tabs.
            "split" | "sp" => vec![Effect::Split],
            "vsplit" | "vs" => vec![Effect::Vsplit],
            "tabnew" => vec![Effect::TabNew],
            _ => vec![Effect::Bell],
        }
    }

    /// True for `:s/delim/…` and `:%s/delim/…`. Anything else starting
    /// with `s` (`split`, `sp`) belongs to the plain ex table.
    pub(super) fn is_substitute(command: &str) -> bool {
        let body = command.strip_prefix('%').unwrap_or(command);
        let mut chars = body.chars();
        matches!((chars.next(), chars.next()), (Some('s'), Some(d)) if !d.is_alphanumeric())
    }
}
