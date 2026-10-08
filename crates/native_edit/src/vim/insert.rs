use super::super::doc::Buffer;
use super::helpers::range_text;
use super::mode::{Bit, Cursor, Effect, Key, ModalEngine, Mode, Repeat};
use super::state::VimEngine;
use super::words::line_len;
impl VimEngine {
    pub(super) fn insert_key<B: Buffer>(&mut self, doc: &mut B, key: Key) -> Vec<Effect> {
        match key {
            Key::Escape => {
                self.enter_normal(doc);
            }
            Key::Enter => {
                let at = self.char_idx(doc, self.cursor);
                if !self.replaying {
                    self.recording.push(Bit::Type("\n".to_string()));
                    if at == self.typed_end {
                        self.typed_end = self.typed_end.saturating_add(1);
                    } else {
                        self.typed_exact = false;
                    }
                }
                doc.insert(at, "\n");
                let (line, col) = doc.line_col_at(at.checked_add(1).unwrap_or(at));
                self.cursor = Cursor { line, col };
                self.preferred_col = col;
                self.clamp(doc);
            }
            Key::Backspace => {
                let at = self.char_idx(doc, self.cursor);
                if !self.replaying {
                    // Only popping the session's own tail keeps the
                    // recording exact: the cursor must sit at the end of
                    // session text, past its start.
                    if self.typed_exact && at == self.typed_end && at > self.typed_start {
                        let mut popped = false;
                        while let Some(Bit::Type(typed)) = self.recording.last_mut() {
                            if typed.pop().is_some() {
                                popped = true;
                                break;
                            }
                            self.recording.pop();
                        }
                        if popped {
                            self.typed_end = self.typed_end.saturating_sub(1);
                        } else {
                            self.typed_exact = false;
                        }
                    } else {
                        self.typed_exact = false;
                    }
                }
                if at > 0 {
                    let prev = at.saturating_sub(1);
                    doc.delete(prev..at);
                    let (line, col) = doc.line_col_at(prev);
                    self.cursor = Cursor { line, col };
                    self.preferred_col = col;
                }
                self.clamp(doc);
            }
            Key::Delete => {
                // Forward delete can eat pre-existing text: unattributable.
                if !self.replaying {
                    self.typed_exact = false;
                }
                let at = self.char_idx(doc, self.cursor);
                if at < doc.len_chars() {
                    doc.delete(at..at.checked_add(1).unwrap_or(at));
                }
                self.clamp(doc);
            }
            Key::Left => self.move_h(doc, 1),
            Key::Right => self.move_l(doc, 1),
            Key::Up => self.move_k(doc, 1),
            Key::Down => self.move_j(doc, 1),
            Key::Home => self.goto(doc, self.cursor.line, 0, true),
            Key::End => {
                let len = line_len(doc, self.cursor.line);
                self.goto(doc, self.cursor.line, len, true);
            }
            Key::Tab => {
                let _ = self.type_text(doc, "\t");
            }
            // Unreachable through the view (Ctrl-V pastes outside vim
            // Normal/Visual); bell rather than guess.
            Key::CtrlV => return vec![Effect::Bell],
            Key::Char(_) => return vec![Effect::Bell],
        }
        Vec::new()
    }

    pub(super) fn visual_key<B: Buffer>(&mut self, doc: &mut B, key: Key) -> Vec<Effect> {
        let linewise = self.mode == Mode::VisualLine;
        let blockwise = self.mode == Mode::VisualBlock;
        let key = if key == Key::Char(' ') {
            Key::Char('l')
        } else {
            key
        };
        // Digits accumulate counts for motions.
        if let Key::Char(c) = key
            && c.is_ascii_digit()
            && !(c == '0' && self.count.is_empty())
        {
            self.count.push(c);
            return Vec::new();
        }
        let count = self.take_count();
        match key {
            Key::Escape => {
                self.enter_normal(doc);
                self.rec_discard();
            }
            // Mode switches keep the anchor: `v`/`V`/Ctrl-V reshape the
            // same selection instead of restarting it.
            Key::Char('v') => {
                self.rec_bit(Bit::Key(key));
                self.mode = Mode::Visual;
                self.clear_pending();
            }
            Key::Char('V') => {
                self.rec_bit(Bit::Key(key));
                self.mode = Mode::VisualLine;
                self.clear_pending();
            }
            Key::CtrlV => {
                self.rec_bit(Bit::Key(key));
                if blockwise {
                    self.enter_normal(doc);
                    self.rec_discard();
                } else {
                    self.mode = Mode::VisualBlock;
                    self.clear_pending();
                }
            }
            // `o` swaps the active end, so both rectangle corners (and both
            // charwise ends) stay adjustable without restarting.
            Key::Char('o') => {
                std::mem::swap(&mut self.anchor, &mut self.cursor);
                self.preferred_col = self.cursor.col;
                self.clear_pending();
            }
            Key::Char('h') | Key::Left => self.move_h(doc, count),
            Key::Char('j') | Key::Down => self.move_j(doc, count),
            Key::Char('k') | Key::Up => self.move_k(doc, count),
            Key::Char('l') | Key::Right => self.move_l(doc, count),
            Key::Char('w' | 'W' | 'b' | 'B') => {
                let kind = if matches!(key, Key::Char('w' | 'W')) {
                    'w'
                } else {
                    'b'
                };
                let (target, _) = self.motion_target(doc, kind, count);
                self.goto(doc, target.line, target.col, true);
                self.count.clear();
            }
            Key::Char('e' | 'E') => {
                let (target, _) = self.motion_target(doc, 'e', count);
                self.goto(doc, target.line, target.col, true);
                self.count.clear();
            }
            Key::Char('0') => {
                self.goto(doc, self.cursor.line, 0, true);
                self.count.clear();
            }
            Key::Char('$') => {
                let len = line_len(doc, self.cursor.line);
                self.goto(doc, self.cursor.line, len.saturating_sub(1), true);
                self.count.clear();
            }
            Key::Char('y') => {
                if blockwise {
                    self.yank_block(doc);
                } else {
                    self.yank_selection(doc, linewise);
                }
                self.enter_normal(doc);
                self.rec_discard();
            }
            // Vim parity: Backspace deletes the selection like `x`.
            Key::Char('d' | 'x') | Key::Backspace => {
                if blockwise {
                    // Capture the rectangle before deleting: a blockwise
                    // delete replays from dims, not from motion bits.
                    let span = self.block_span();
                    doc.begin_undo_group();
                    let cursor = self.delete_selection_block(doc);
                    doc.end_undo_group();
                    self.cursor = cursor;
                    self.preferred_col = cursor.col;
                    self.enter_normal(doc);
                    if !self.replaying {
                        self.last_change =
                            span.map(|(top, bottom, left, right)| Repeat::BlockDelete {
                                height: bottom.saturating_sub(top),
                                left,
                                right: right.saturating_sub(1),
                            });
                        self.recording.clear();
                    }
                } else {
                    self.rec_bit(Bit::Key(Key::Char('v')));
                    self.delete_selection(doc, linewise);
                    self.enter_normal(doc);
                    // Charwise visual changes are not repeatable yet: the
                    // repeat bells instead of replaying stale input.
                    if !self.replaying {
                        self.last_change = None;
                        self.recording.clear();
                    }
                }
            }
            Key::Char('c') => {
                if blockwise {
                    self.begin_change_group(doc);
                    let span = self.block_span();
                    if let Some((top, bottom, left, right)) = span {
                        self.delete_block_ranges(doc, top, bottom, left, right);
                    }
                    self.rec_bit(Bit::Key(Key::Char('c')));
                    self.start_block_insert(doc, false);
                } else {
                    self.rec_bit(Bit::Key(Key::Char('v')));
                    self.rec_bit(Bit::Key(Key::Char('c')));
                    self.delete_selection(doc, linewise);
                    self.begin_insert_session(doc);
                    self.clamp(doc);
                }
            }
            // Block insert: `I` at the left edge, `A` past the right edge.
            // Typing replays on every other row at Escape.
            Key::Char('I') if blockwise => {
                self.rec_bit(Bit::Key(key));
                self.start_block_insert(doc, false);
            }
            Key::Char('A') if blockwise => {
                self.rec_bit(Bit::Key(key));
                self.start_block_insert(doc, true);
            }
            _ => {
                self.rec_discard();
                return vec![Effect::Bell];
            }
        }
        Vec::new()
    }

    pub(super) fn selection_range<B: Buffer>(&self, doc: &B, linewise: bool) -> (usize, usize) {
        // Order-aware: after `o` the cursor can sit before the anchor, and
        // the inclusive end must extend the far end, not the cursor's.
        let a = self.char_idx(doc, self.anchor);
        let b = self.char_idx(doc, self.cursor);
        let (a, mut b) = (a.min(b), a.max(b));
        if linewise {
            let (al, _) = doc.line_col_at(a);
            let (bl, _) = doc.line_col_at(b);
            let (lo, hi) = (al.min(bl), al.max(bl));
            let from = doc.char_at_line_col(lo, 0);
            b = if hi
                .checked_add(1)
                .is_some_and(|next| next < doc.line_count())
            {
                doc.char_at_line_col(hi.checked_add(1).unwrap_or(hi), 0)
            } else {
                doc.len_chars()
            };
            return (from, b);
        }
        // Charwise selections include the cursor cell.
        b = b
            .checked_add(1)
            .map_or(doc.len_chars(), |next| next.min(doc.len_chars()));
        (a.min(b), a.max(b))
    }

    pub(super) fn yank_selection<B: Buffer>(&mut self, doc: &mut B, linewise: bool) {
        let (from, to) = self.selection_range(doc, linewise);
        let mut text = range_text(doc, from, to);
        if linewise && text.ends_with('\n') {
            text.pop();
        }
        self.set_register(text, linewise, None);
        self.cursor = self.anchor;
    }

    pub(super) fn delete_selection<B: Buffer>(&mut self, doc: &mut B, linewise: bool) {
        let (from, to) = self.selection_range(doc, linewise);
        let cursor = self.delete_range(doc, from, to);
        self.cursor = cursor;
    }

    pub(super) fn command_key<B: Buffer>(&mut self, doc: &mut B, key: Key) -> Vec<Effect> {
        match key {
            Key::Escape => {
                self.command.clear();
                self.enter_normal(doc);
                self.rec_discard();
            }
            Key::Enter => {
                // Search and ex commands never join the `.` recording.
                self.rec_discard();
                if self.command.starts_with('/') {
                    // Vim `/pattern`: jump to the next match, wrapping.
                    // An empty pattern repeats the last search.
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
                    if !self.jump_to_match(doc, &pattern, from, true) {
                        return vec![Effect::Bell];
                    }
                    return Vec::new();
                }
                // Ex commands are not `.`-repeatable; the in-progress
                // change (if any) is dropped, the last change keeps.
                self.rec_discard();
                if Self::is_substitute(&self.command) {
                    let command = self.command.clone();
                    let outcome = self.substitute(doc, &command);
                    self.command.clear();
                    self.enter_normal(doc);
                    return outcome;
                }
                let effects = match self.command.as_str() {
                    "w" => vec![Effect::Save],
                    "w!" => vec![Effect::SaveForce],
                    "q" => vec![Effect::Quit],
                    "q!" => vec![Effect::QuitForce],
                    // `:qw` is not real vim, but listed next to `:wq` so
                    // often that accepting it avoids a dead end.
                    "wq" | "qw" | "x" => vec![Effect::WriteQuit { force: false }],
                    "wq!" | "qw!" | "x!" => vec![Effect::WriteQuit { force: true }],
                    "qa" => vec![Effect::QuitAll { force: false }],
                    "qa!" => vec![Effect::QuitAll { force: true }],
                    // Workspace layout stays with the GUI: the engine only
                    // asks, via effects the session layer maps to tabs.
                    "split" | "sp" => vec![Effect::Split],
                    "vsplit" | "vs" => vec![Effect::Vsplit],
                    "tabnew" => vec![Effect::TabNew],
                    _ => vec![Effect::Bell],
                };
                let failed = effects == vec![Effect::Bell];
                self.command.clear();
                self.enter_normal(doc);
                if failed {
                    return vec![Effect::Bell];
                }
                return effects;
            }
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

    /// True for `:s/delim/…` and `:%s/delim/…`. Anything else starting
    /// with `s` (`split`, `sp`) belongs to the plain ex table.
    pub(super) fn is_substitute(command: &str) -> bool {
        let body = command.strip_prefix('%').unwrap_or(command);
        let mut chars = body.chars();
        matches!((chars.next(), chars.next()), (Some('s'), Some(d)) if !d.is_alphanumeric())
    }

    /// `:s/pat/rep/[g]` on the cursor line, `:%s/pat/rep/[g]` on the
    /// buffer. Patterns are literal (like `/` search); an empty pattern
    /// reuses the last search. `&` in the replacement splices the match,
    /// `\n` a newline. One undo unit; unknown flags and zero matches
    /// bell. Never `.`-repeatable (ex commands aren't).
    pub(super) fn substitute<B: Buffer>(&mut self, doc: &mut B, command: &str) -> Vec<Effect> {
        let all = command.starts_with('%');
        let mut body = command.strip_prefix('%').unwrap_or(command);
        body = body.strip_prefix('s').unwrap_or(body);
        let mut chars = body.chars();
        let Some(delim) = chars.next() else {
            return vec![Effect::Bell];
        };
        let rest = chars.as_str();
        // Split on unescaped delimiters: `\X` escapes to `X`.
        let mut parts: Vec<String> = Vec::new();
        let mut current = String::new();
        let mut escaped = false;
        for c in rest.chars() {
            if escaped {
                match c {
                    'n' => current.push('\n'),
                    't' => current.push('\t'),
                    _ => current.push(c),
                }
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == delim {
                parts.push(std::mem::take(&mut current));
            } else {
                current.push(c);
            }
        }
        if escaped {
            // A trailing backslash escapes nothing.
            return vec![Effect::Bell];
        }
        parts.push(current);
        if parts.len() < 2 || parts.len() > 3 {
            return vec![Effect::Bell];
        }
        let Some(first) = parts.first() else {
            return vec![Effect::Bell];
        };
        let pattern = if first.is_empty() {
            self.search_last.clone()
        } else {
            first.clone()
        };
        if pattern.is_empty() || pattern.contains('\n') {
            return vec![Effect::Bell];
        }
        let flags = parts.get(2).map_or("", String::as_str);
        if !flags.chars().all(|c| c == 'g') {
            return vec![Effect::Bell];
        }
        let global = flags.contains('g');
        self.search_last.clone_from(&pattern);

        let lines: Vec<usize> = if all {
            (0..doc.line_count()).collect()
        } else {
            vec![self.cursor.line.min(doc.line_count().saturating_sub(1))]
        };
        // Collect against the original text, then apply bottom-up so
        // offsets (including newline-bearing replacements) stay valid.
        struct Edit {
            line: usize,
            from: usize,
            to: usize,
            text: String,
        }
        let mut edits: Vec<Edit> = Vec::new();
        for line in lines {
            let text = doc.line_text(line);
            let matches: Vec<(usize, usize)> = text
                .match_indices(&pattern)
                .map(|(byte, m)| {
                    let col = text.get(..byte).map_or(0, |p| p.chars().count());
                    (col, col.saturating_add(m.chars().count()))
                })
                .collect();
            if matches.is_empty() {
                continue;
            }
            let base = doc.char_at_line_col(line, 0);
            let take = if global { matches.len() } else { 1 };
            let Some(replacement) = parts.get(1) else {
                return vec![Effect::Bell];
            };
            for (col, end) in matches.into_iter().take(take) {
                let mut rep = String::new();
                for c in replacement.chars() {
                    // The splitter already resolved escapes, so a bare `&`
                    // always means the whole match.
                    if c == '&' {
                        rep.push_str(&pattern);
                    } else {
                        rep.push(c);
                    }
                }
                edits.push(Edit {
                    line,
                    from: base.saturating_add(col),
                    to: base.saturating_add(end),
                    text: rep,
                });
            }
        }
        if edits.is_empty() {
            return vec![Effect::Bell];
        }
        doc.begin_undo_group();
        for edit in edits.iter().rev() {
            doc.delete(edit.from..edit.to);
            if !edit.text.is_empty() {
                doc.insert(edit.from, &edit.text);
            }
        }
        doc.end_undo_group();
        let Some(first) = edits.first() else {
            return vec![Effect::Bell];
        };
        self.cursor = Cursor {
            line: first.line,
            col: first
                .from
                .saturating_sub(doc.char_at_line_col(first.line, 0)),
        };
        self.scroll_request = true;
        self.clamp(doc);
        doc.end_run();
        Vec::new()
    }
}
