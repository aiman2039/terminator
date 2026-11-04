//! Command-line mode: `/` and `?` search, `:set`, and the ex commands.
use super::super::doc::Buffer;
use super::mode::Effect;
use super::mode::Key;
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

    pub(crate) fn run_command_line<B: Buffer>(&mut self, doc: &mut B) -> Vec<Effect> {
        // Search and ex commands never join the `.` recording.
        self.rec_discard();
        if self.command.starts_with(['/', '?']) {
            return self.run_search(doc);
        }
        self.x.last_ex = Some(self.command.clone());
        if let Some(args) = self.set_args() {
            return self.run_set(doc, &args);
        }
        if let Some((line, col)) = Self::line_target(&self.command) {
            self.command.clear();
            self.enter_normal(doc);
            self.push_jump();
            let effects = self.place_cursor(doc, line.saturating_sub(1), col.saturating_sub(1));
            self.request_scroll();
            return effects;
        }
        let command = std::mem::take(&mut self.command);
        self.enter_normal(doc);
        self.run_ex(doc, &command)
    }

    /// Vim `/pattern` and `?pattern`: jump to the next (previous) match,
    /// wrapping. An empty pattern repeats the last search.
    fn run_search<B: Buffer>(&mut self, doc: &mut B) -> Vec<Effect> {
        let backward = self.command.starts_with('?');
        let pattern = if self.command.chars().count() > 1 {
            self.command.get(1..).unwrap_or("").to_string()
        } else {
            self.search_last.clone()
        };
        self.command.clear();
        self.enter_normal(doc);
        if pattern.is_empty() {
            return vec![Effect::Bell];
        }
        let Some(target) = self.search_from_cursor(doc, &pattern, backward) else {
            return vec![Effect::Bell];
        };
        self.push_jump();
        self.cursor = target;
        self.preferred_col = target.col;
        self.clamp(doc);
        self.scroll_request = true;
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

    pub(super) fn ex_effects(command: &str) -> Vec<Effect> {
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

    /// `:42` or `:42:7` (`IntelliJ` Go to Line): 1-based line and column.
    pub(super) fn line_target(command: &str) -> Option<(usize, usize)> {
        let (line, col) = command
            .trim()
            .split_once(':')
            .unwrap_or((command.trim(), "1"));
        Some((line.parse().ok()?, col.parse().ok()?))
    }
}
