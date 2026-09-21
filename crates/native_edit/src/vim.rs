//! Minimal vim-modal engine over [`crate::doc::Buffer`].
//!
//! Scope (the Minimal tier): Normal / Insert / Visual / VisualLine / Command
//! modes; motions `hjkl w b e 0 $ ^ G gg` with counts; operators `d c y`
//! (plus `dd yy cc x p u`); `:w :q :q! :wq :x`. Anything richer (text
//! objects beyond `iw`/`aw`, macros, `hjkl` crate adoption) builds on the
//! [`ModalEngine`] seam. The engine never touches I/O: the widget layer maps
//! egui input to [`Key`] and turns [`Effect`] into app actions.

use super::doc::Buffer;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    #[default]
    Normal,
    Insert,
    Visual,
    VisualLine,
    Command,
}

/// UI-toolkit-neutral key. The egui adapter maps `Key`/`Event` to this.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Key {
    Char(char),
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Enter,
    Backspace,
    Delete,
    Escape,
    Tab,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    Bell,
    Save,
    SaveForce,
    Quit,
    QuitForce,
    /// Save, then close once the write settles (`:wq`, `:x`).
    WriteQuit {
        force: bool,
    },
    /// Close every native editor tab (`:qa`); dirty tabs block unless forced.
    QuitAll {
        force: bool,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Cursor {
    pub line: usize,
    pub col: usize,
}

pub trait ModalEngine {
    fn mode(&self) -> Mode;
    fn cursor(&self) -> Cursor;
    /// Ordered (anchor, cursor) ends while selecting.
    fn selection(&self) -> Option<(Cursor, Cursor)>;
    fn command_line(&self) -> &str;
    fn press_key<B: Buffer>(&mut self, doc: &mut B, key: Key) -> Vec<Effect>;
    fn type_text<B: Buffer>(&mut self, doc: &mut B, text: &str) -> Vec<Effect>;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PendingOp {
    Delete,
    Change,
    Yank,
}

#[derive(Clone, Debug, Default)]
struct Register {
    text: String,
    linewise: bool,
}

pub struct VimEngine {
    mode: Mode,
    cursor: Cursor,
    anchor: Cursor,
    preferred_col: usize,
    count: String,
    pending: Option<PendingOp>,
    pending_g: bool,
    command: String,
    register: Register,
    search_last: String,
    scroll_request: bool,
}

impl Default for VimEngine {
    fn default() -> Self {
        Self {
            mode: Mode::Normal,
            cursor: Cursor::default(),
            anchor: Cursor::default(),
            preferred_col: 0,
            count: String::new(),
            pending: None,
            pending_g: false,
            command: String::new(),
            register: Register::default(),
            search_last: String::new(),
            scroll_request: false,
        }
    }
}

/// Supported `~/.vimrc` options (the rest of the file is ignored).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VimrcOptions {
    pub ignorecase: bool,
}

fn apply_set_str(opts: &mut VimrcOptions, option: &str) -> bool {
    match option {
        "ic" | "ignorecase" => opts.ignorecase = true,
        "noic" | "noignorecase" => opts.ignorecase = false,
        "ic!" | "ignorecase!" => opts.ignorecase = !opts.ignorecase,
        _ => return false,
    }
    true
}

/// Parse a vimrc buffer: apply the supported `set` options, silently ignore
/// everything else (mappings, plugins, comments, unknown options). A vimrc
/// must never fail to load because of something we do not implement.
pub fn parse_vimrc(content: &str) -> VimrcOptions {
    let mut opts = VimrcOptions::default();
    for raw in content.lines() {
        let mut line = raw.trim();
        if line.is_empty() || line.starts_with('"') {
            continue;
        }
        // A `"` after whitespace starts a trailing comment.
        if let Some(pos) = line.find('"')
            && pos > 0
            && line[..pos].ends_with([' ', '\t'])
        {
            line = line[..pos].trim_end();
        }
        let args = line
            .strip_prefix("set ")
            .or_else(|| line.strip_prefix("setlocal "));
        let Some(args) = args else { continue };
        for arg in args.split_whitespace() {
            apply_set_str(&mut opts, arg);
        }
    }
    opts
}

impl VimEngine {
    pub fn new() -> Self {
        Self::default()
    }

    fn take_count(&mut self) -> usize {
        if self.count.is_empty() {
            1
        } else {
            self.count.parse().unwrap_or(1).clamp(1, 1_000_000)
        }
    }

    fn clear_pending(&mut self) {
        self.pending = None;
        self.pending_g = false;
        self.count.clear();
    }
}

// --- flat-stream word helpers (operate on char indices, cross lines) ---

#[derive(Clone, Copy, PartialEq, Eq)]
enum WordClass {
    Blank,
    Word,
    Punct,
}

fn word_class(c: char) -> WordClass {
    if c.is_whitespace() {
        WordClass::Blank
    } else if c.is_alphanumeric() || c == '_' {
        WordClass::Word
    } else {
        WordClass::Punct
    }
}

fn word_forward(text: &[char], mut i: usize, count: usize) -> usize {
    for _ in 0..count {
        if i >= text.len() {
            break;
        }
        let cls = word_class(text[i]);
        if cls != WordClass::Blank {
            while i < text.len() && word_class(text[i]) == cls {
                i += 1;
            }
        }
        while i < text.len() && word_class(text[i]) == WordClass::Blank {
            i += 1;
        }
    }
    i.min(text.len())
}

fn word_backward(text: &[char], mut i: usize, count: usize) -> usize {
    for _ in 0..count {
        if i == 0 {
            break;
        }
        let mut j = i - 1;
        while j > 0 && word_class(text[j]) == WordClass::Blank {
            j -= 1;
        }
        let cls = word_class(text[j]);
        while j > 0 && word_class(text[j - 1]) == cls {
            j -= 1;
        }
        i = j;
    }
    i
}

fn word_end(text: &[char], mut i: usize, count: usize) -> usize {
    for _ in 0..count {
        if text.is_empty() {
            break;
        }
        i = (i + 1).min(text.len().saturating_sub(1));
        while i < text.len() && word_class(text[i]) == WordClass::Blank {
            i += 1;
        }
        if i >= text.len() {
            break;
        }
        let cls = word_class(text[i]);
        while i + 1 < text.len() && word_class(text[i + 1]) == cls {
            i += 1;
        }
    }
    i.min(text.len().saturating_sub(1))
}

// --- cursor plumbing ---

fn line_len<B: Buffer>(doc: &B, line: usize) -> usize {
    doc.line_text(line).chars().count()
}

fn current_char<B: Buffer>(doc: &B, cursor: Cursor) -> char {
    doc.line_text(cursor.line)
        .chars()
        .nth(cursor.col)
        .unwrap_or('\n')
}

fn flat_text<B: Buffer>(doc: &B) -> Vec<char> {
    let mut out = Vec::new();
    for line in 0..doc.line_count() {
        if line > 0 {
            out.push('\n');
        }
        out.extend(doc.line_text(line).chars());
    }
    out
}

impl VimEngine {
    /// Last cursor-addressable line. A trailing empty line exists only to
    /// terminate the final newline (vim shows no line there), so it is
    /// excluded; every genuine line, empty or not, stays reachable.
    fn last_addressable<B: Buffer>(doc: &B) -> usize {
        let lines = doc.line_count().max(1);
        if lines > 1 && doc.line_text(lines - 1).is_empty() {
            lines - 2
        } else {
            lines - 1
        }
    }

    fn clamp<B: Buffer>(&mut self, doc: &B) {
        let last = Self::last_addressable(doc);
        self.cursor.line = self.cursor.line.min(last);
        let len = line_len(doc, self.cursor.line);
        let max_col = if self.mode == Mode::Insert || len == 0 {
            len
        } else {
            len.saturating_sub(1)
        };
        self.cursor.col = self.cursor.col.min(max_col);
        self.anchor.line = self.anchor.line.min(last);
        let alen = line_len(doc, self.anchor.line);
        self.anchor.col = self.anchor.col.min(alen);
    }

    fn char_idx<B: Buffer>(&self, doc: &B, cursor: Cursor) -> usize {
        doc.char_at_line_col(cursor.line, cursor.col)
    }

    fn goto<B: Buffer>(&mut self, doc: &B, line: usize, col: usize, preferred: bool) {
        self.cursor.line = line;
        self.cursor.col = col;
        if preferred {
            self.preferred_col = col;
        }
        self.clamp(doc);
    }

    fn first_non_blank<B: Buffer>(doc: &B, line: usize) -> usize {
        doc.line_text(line)
            .chars()
            .take_while(|c| c.is_whitespace())
            .count()
    }

    fn move_h<B: Buffer>(&mut self, doc: &B, count: usize) {
        let col = self.cursor.col.saturating_sub(count);
        self.goto(doc, self.cursor.line, col, true);
    }

    fn move_l<B: Buffer>(&mut self, doc: &B, count: usize) {
        let col = self.cursor.col.saturating_add(count);
        self.goto(doc, self.cursor.line, col, true);
    }

    fn move_j<B: Buffer>(&mut self, doc: &B, count: usize) {
        let line = self.cursor.line.saturating_add(count);
        self.cursor.line = line;
        self.cursor.col = self.preferred_col;
        self.clamp(doc);
    }

    fn move_k<B: Buffer>(&mut self, doc: &B, count: usize) {
        let line = self.cursor.line.saturating_sub(count);
        self.cursor.line = line;
        self.cursor.col = self.preferred_col;
        self.clamp(doc);
    }

    fn enter_insert(&mut self) {
        self.mode = Mode::Insert;
        self.clear_pending();
    }

    fn enter_normal<B: Buffer>(&mut self, doc: &mut B) {
        self.mode = Mode::Normal;
        self.clear_pending();
        self.command.clear();
        doc.end_run();
        self.clamp(doc);
    }

    /// Pointer click: move the cursor (or the active end of a selection).
    /// Never edits, so it returns no effects.
    pub fn place_cursor<B: Buffer>(&mut self, doc: &mut B, line: usize, col: usize) -> Vec<Effect> {
        if self.mode == Mode::Command {
            return Vec::new();
        }
        self.clear_pending();
        doc.end_run();
        self.cursor = Cursor { line, col };
        if !matches!(self.mode, Mode::Visual | Mode::VisualLine) {
            self.anchor = self.cursor;
        }
        self.preferred_col = col;
        self.clamp(doc);
        Vec::new()
    }

    fn delete_range<B: Buffer>(&mut self, doc: &mut B, from: usize, to: usize) -> Cursor {
        let (from, to) = (from.min(to), from.max(to));
        doc.delete(from..to);
        doc.end_run();
        let (line, col) = doc.line_col_at(from.min(doc.len_chars()));
        Cursor { line, col }
    }

    /// Whole-buffer visual selection (IDE select-all, Cmd/Ctrl+A).
    pub fn select_all<B: Buffer>(&mut self, doc: &B) {
        let last = doc.line_count().saturating_sub(1);
        self.anchor = Cursor { line: 0, col: 0 };
        self.cursor = Cursor {
            line: last,
            col: line_len(doc, last),
        };
        self.preferred_col = self.cursor.col;
        self.mode = Mode::Visual;
        self.clear_pending();
    }

    /// IDE copy source: the selection, else the cursor line. Pure, so the
    /// view can copy without disturbing mode, cursor, or undo.
    pub fn copy_text<B: Buffer>(&self, doc: &B) -> String {
        if self.selection().is_some() {
            let linewise = self.mode == Mode::VisualLine;
            let (from, to) = self.selection_range(doc, linewise);
            return range_text(doc, from, to);
        }
        let line = self.cursor.line.min(doc.line_count().saturating_sub(1));
        doc.line_text(line)
    }

    /// IDE cut: remove the copy source and park the cursor on the cut point.
    /// A cut selection collapses back to Normal, mirroring `d`.
    pub fn cut<B: Buffer>(&mut self, doc: &mut B) -> String {
        let linewise = self.mode == Mode::VisualLine;
        let (from, to) = if self.selection().is_some() {
            self.selection_range(doc, linewise)
        } else {
            let line = self.cursor.line.min(doc.line_count().saturating_sub(1));
            let from = doc.char_at_line_col(line, 0);
            let to = if line + 1 < doc.line_count() {
                doc.char_at_line_col(line + 1, 0)
            } else {
                doc.len_chars()
            };
            (from, to)
        };
        let text = range_text(doc, from, to);
        let cursor = self.delete_range(doc, from, to);
        self.cursor = cursor;
        self.preferred_col = cursor.col;
        if linewise || matches!(self.mode, Mode::Visual) {
            self.enter_normal(doc);
        } else {
            self.clear_pending();
            self.clamp(doc);
        }
        text
    }

    /// IDE paste: insert plain text at the cursor in any mode (Cmd/Ctrl+V).
    /// One undo unit; mode and selection are left alone.
    pub fn paste_text<B: Buffer>(&mut self, doc: &mut B, text: &str) {
        if text.is_empty() {
            return;
        }
        self.clear_pending();
        let at = self.char_idx(doc, self.cursor).min(doc.len_chars());
        doc.insert(at, text);
        doc.end_run();
        let end = at + text.chars().count().min(doc.len_chars().saturating_sub(at));
        let (line, col) = doc.line_col_at(end);
        self.cursor = Cursor { line, col };
        self.preferred_col = col;
        self.clamp(doc);
    }

    /// Consume a pending cursor-reveal request (search jumps, `G`/`gg`).
    /// The view scrolls the cursor row into sight when this returns true.
    pub fn take_scroll_request(&mut self) -> bool {
        std::mem::replace(&mut self.scroll_request, false)
    }

    /// All match starts (char indices) of a literal pattern, in buffer
    /// order. Literal and case-sensitive (no vim regex); single-line
    /// patterns only, so a pattern containing `\n` never matches.
    fn search_matches<B: Buffer>(doc: &B, pattern: &str) -> Vec<usize> {
        if pattern.is_empty() || pattern.contains('\n') {
            return Vec::new();
        }
        let mut out = Vec::new();
        for line in 0..doc.line_count() {
            let text = doc.line_text(line);
            for (byte, _) in text.match_indices(pattern) {
                let col = text[..byte].chars().count();
                out.push(doc.char_at_line_col(line, col));
            }
        }
        out
    }

    /// Jump to the next (`forward`) or previous match from `from`, wrapping
    /// around the buffer. Forward is inclusive of `from` (landing on the
    /// match under the cursor); backward is exclusive (so `N` steps back).
    /// Returns false — and leaves the cursor alone — when nothing matches.
    fn jump_to_match<B: Buffer>(
        &mut self,
        doc: &mut B,
        pattern: &str,
        from: usize,
        forward: bool,
    ) -> bool {
        let matches = Self::search_matches(doc, pattern);
        if matches.is_empty() {
            return false;
        }
        let next = if forward {
            matches
                .iter()
                .find(|m| **m >= from)
                .or(matches.first())
                .copied()
        } else {
            matches
                .iter()
                .rev()
                .find(|m| **m < from)
                .or(matches.last())
                .copied()
        };
        let Some(at) = next else {
            return false;
        };
        let (line, col) = doc.line_col_at(at);
        self.cursor = Cursor { line, col };
        self.preferred_col = col;
        self.clamp(doc);
        self.scroll_request = true;
        true
    }

    /// IDE undo/redo: same as `u`/Ctrl-R but callable in any mode, with the
    /// cursor clamped back into range afterwards.
    pub fn undo<B: Buffer>(&mut self, doc: &mut B) {
        doc.undo();
        self.clear_pending();
        self.clamp(doc);
    }

    /// See [`Self::undo`].
    pub fn redo<B: Buffer>(&mut self, doc: &mut B) {
        doc.redo();
        self.clear_pending();
        self.clamp(doc);
    }

    fn apply_operator<B: Buffer>(
        &mut self,
        doc: &mut B,
        op: PendingOp,
        target: Cursor,
        inclusive: bool,
    ) -> Vec<Effect> {
        let from = self.char_idx(doc, self.cursor);
        let mut to = self.char_idx(doc, target);
        if inclusive {
            to = (to + 1).min(doc.len_chars());
        }
        let (from, to) = (from.min(to), from.max(to));
        match op {
            PendingOp::Delete => {
                let cursor = self.delete_range(doc, from, to);
                self.cursor = cursor;
                self.enter_normal(doc);
            }
            PendingOp::Change => {
                let cursor = self.delete_range(doc, from, to);
                self.cursor = cursor;
                self.enter_insert();
                self.clamp(doc);
            }
            PendingOp::Yank => {
                self.register.text = range_text(doc, from, to);
                self.register.linewise = false;
                self.cursor = target;
                self.enter_normal(doc);
            }
        }
        Vec::new()
    }

    fn delete_lines<B: Buffer>(&mut self, doc: &mut B, count: usize) {
        let lines = doc.line_count();
        let start = self.cursor.line.min(lines.saturating_sub(1));
        let end = (start + count).min(lines);
        if lines == 0 {
            return;
        }
        let from = doc.char_at_line_col(start, 0);
        let to = if end < lines {
            doc.char_at_line_col(end, 0)
        } else {
            doc.len_chars()
        };
        let mut yanked = range_text(doc, from, to);
        if end < lines && yanked.ends_with('\n') {
            yanked.pop();
        }
        self.register.text = yanked;
        self.register.linewise = true;
        doc.delete(from..to);
        doc.end_run();
        self.cursor.line = start.min(doc.line_count().saturating_sub(1));
        self.cursor.col = Self::first_non_blank(doc, self.cursor.line);
        self.enter_normal(doc);
    }

    fn yank_lines<B: Buffer>(&mut self, doc: &mut B, count: usize) {
        let lines = doc.line_count();
        let start = self.cursor.line.min(lines.saturating_sub(1));
        let end = (start + count).min(lines);
        let mut yanked = Vec::new();
        for line in start..end {
            yanked.push(doc.line_text(line));
        }
        self.register.text = yanked.join("\n");
        self.register.linewise = true;
        self.enter_normal(doc);
    }

    fn paste<B: Buffer>(&mut self, doc: &mut B) {
        if self.register.text.is_empty() {
            return;
        }
        if self.register.linewise {
            let line = (self.cursor.line + 1).min(doc.line_count());
            let at = if line < doc.line_count() {
                doc.char_at_line_col(line, 0)
            } else {
                doc.len_chars()
            };
            let prefix = if line == doc.line_count() && !doc_is_empty(doc) {
                "\n"
            } else {
                ""
            };
            let text = format!("{}{}\n", prefix, self.register.text);
            doc.insert(at, &text);
            doc.end_run();
            self.cursor.line = line.min(doc.line_count().saturating_sub(1));
            self.cursor.col = Self::first_non_blank(doc, self.cursor.line);
        } else {
            let at = (self.char_idx(doc, self.cursor) + 1).min(doc.len_chars());
            doc.insert(at, &self.register.text);
            doc.end_run();
            let (line, col) = doc.line_col_at(at + self.register.text.chars().count() - 1);
            self.cursor = Cursor { line, col };
        }
        self.enter_normal(doc);
    }

    fn change_line<B: Buffer>(&mut self, doc: &mut B, count: usize) {
        let lines = doc.line_count();
        let start = self.cursor.line.min(lines.saturating_sub(1));
        let end = (start + count).min(lines);
        let from = doc.char_at_line_col(start, 0);
        // Full lines including their newlines; deleting every line leaves the
        // single empty final line, so structure is always preserved.
        let to = if end < lines {
            doc.char_at_line_col(end, 0)
        } else {
            doc.len_chars()
        };
        doc.delete(from..to);
        doc.end_run();
        self.cursor = Cursor {
            line: start,
            col: 0,
        };
        self.enter_insert();
        self.clamp(doc);
    }

    fn word_target<B: Buffer>(&self, doc: &B, kind: char, count: usize) -> (Cursor, bool) {
        let text = flat_text(doc);
        let from = self.char_idx(doc, self.cursor);
        let (idx, inclusive) = match kind {
            'w' => (word_forward(&text, from, count), false),
            'b' => (word_backward(&text, from, count), false),
            'e' => (word_end(&text, from, count), true),
            _ => (from, false),
        };
        let total: usize = text.len();
        let idx = idx.min(total);
        // Map the flat index back to line/col by walking line lengths.
        let mut rest = idx;
        let mut line = 0;
        while line < doc.line_count() {
            let len = line_len(doc, line);
            if rest <= len || line + 1 == doc.line_count() {
                break;
            }
            rest -= len + 1;
            line += 1;
        }
        (Cursor { line, col: rest }, inclusive)
    }

    fn normal_key<B: Buffer>(&mut self, doc: &mut B, key: Key) -> Vec<Effect> {
        // Vim parity: Space moves right like `l`, including under operators.
        let key = if key == Key::Char(' ') {
            Key::Char('l')
        } else {
            key
        };
        // Digits accumulate a count; a bare 0 is the line-start motion.
        if let Key::Char(c) = key
            && c.is_ascii_digit()
            && !(c == '0' && self.count.is_empty())
        {
            self.count.push(c);
            self.pending_g = false;
            return Vec::new();
        }
        if self.pending_g && key != Key::Char('g') {
            self.clear_pending();
            return vec![Effect::Bell];
        }
        let count = self.take_count();
        match key {
            Key::Char('h') | Key::Left => {
                self.apply_motion(doc, 'h', count);
            }
            Key::Char('j') | Key::Down => {
                self.apply_motion(doc, 'j', count);
            }
            Key::Char('k') | Key::Up => {
                self.apply_motion(doc, 'k', count);
            }
            Key::Char('l') | Key::Right => {
                self.apply_motion(doc, 'l', count);
            }
            Key::Char('w') | Key::Char('W') => {
                self.apply_motion(doc, 'w', count);
            }
            Key::Char('b') | Key::Char('B') => {
                self.apply_motion(doc, 'b', count);
            }
            Key::Char('e') | Key::Char('E') => {
                self.apply_motion(doc, 'e', count);
            }
            Key::Char('0') => {
                self.apply_motion(doc, '0', 1);
            }
            Key::Char('$') => {
                self.apply_motion(doc, '$', 1);
            }
            Key::Char('^') => {
                let col = Self::first_non_blank(doc, self.cursor.line);
                self.apply_motion_to(
                    doc,
                    Cursor {
                        line: self.cursor.line,
                        col,
                    },
                );
            }
            Key::Char('G') => {
                let line = if self.count.is_empty() {
                    doc.line_count().saturating_sub(1)
                } else {
                    count.saturating_sub(1)
                };
                let col = Self::first_non_blank(doc, line.min(doc.line_count().saturating_sub(1)));
                self.apply_motion_to(doc, Cursor { line, col });
                self.scroll_request = true;
            }
            Key::Char('g') => {
                if self.pending_g {
                    self.clear_pending();
                    let col = Self::first_non_blank(doc, 0);
                    self.apply_motion_to(doc, Cursor { line: 0, col });
                    self.scroll_request = true;
                } else {
                    self.pending_g = true;
                    return Vec::new();
                }
            }
            Key::Char('x') => {
                let from = self.char_idx(doc, self.cursor);
                let len = line_len(doc, self.cursor.line);
                if self.cursor.col < len {
                    let to = (from + count).min(from + (len - self.cursor.col));
                    let cursor = self.delete_range(doc, from, to);
                    self.cursor = cursor;
                }
                self.clear_pending();
                self.clamp(doc);
            }
            Key::Char('d') => {
                return self.operator_key(doc, PendingOp::Delete);
            }
            Key::Char('c') => {
                return self.operator_key(doc, PendingOp::Change);
            }
            Key::Char('y') => {
                return self.operator_key(doc, PendingOp::Yank);
            }
            Key::Char('p') => {
                self.paste(doc);
                self.clear_pending();
            }
            Key::Char('u') => {
                self.undo(doc);
            }
            Key::Char('i') => {
                self.enter_insert();
                self.clamp(doc);
            }
            Key::Char('a') => {
                let len = line_len(doc, self.cursor.line);
                if len > 0 {
                    self.cursor.col = (self.cursor.col + 1).min(len);
                }
                self.preferred_col = self.cursor.col;
                self.enter_insert();
                self.clamp(doc);
            }
            Key::Char('A') => {
                self.cursor.col = line_len(doc, self.cursor.line);
                self.preferred_col = self.cursor.col;
                self.enter_insert();
                self.clamp(doc);
            }
            Key::Char('I') => {
                self.cursor.col = Self::first_non_blank(doc, self.cursor.line);
                self.preferred_col = self.cursor.col;
                self.enter_insert();
                self.clamp(doc);
            }
            Key::Char('o') => {
                // Split at the start of the next line (or append): the new
                // blank line is always `cursor.line + 1`.
                let insert_at = if self.cursor.line + 1 < doc.line_count() {
                    doc.char_at_line_col(self.cursor.line + 1, 0)
                } else {
                    doc.len_chars()
                };
                doc.insert(insert_at, "\n");
                doc.end_run();
                self.cursor = Cursor {
                    line: self.cursor.line + 1,
                    col: 0,
                };
                self.enter_insert();
                self.clamp(doc);
            }
            Key::Char('O') => {
                let at = doc.char_at_line_col(self.cursor.line, 0);
                doc.insert(at, "\n");
                doc.end_run();
                let (line, _) = doc.line_col_at(at);
                self.cursor = Cursor { line, col: 0 };
                self.enter_insert();
                self.clamp(doc);
            }
            Key::Char('v') => {
                self.mode = Mode::Visual;
                self.anchor = self.cursor;
                self.clear_pending();
            }
            Key::Char('V') => {
                self.mode = Mode::VisualLine;
                self.anchor = self.cursor;
                self.clear_pending();
            }
            Key::Char(':') => {
                self.mode = Mode::Command;
                self.command.clear();
                self.clear_pending();
            }
            Key::Char('/') => {
                self.mode = Mode::Command;
                self.command.clear();
                self.command.push('/');
                self.clear_pending();
            }
            Key::Char('n') => {
                if self.search_last.is_empty() {
                    return vec![Effect::Bell];
                }
                let pattern = self.search_last.clone();
                let from = self.char_idx(doc, self.cursor).saturating_add(1);
                self.clear_pending();
                if !self.jump_to_match(doc, &pattern, from, true) {
                    return vec![Effect::Bell];
                }
            }
            Key::Char('N') => {
                if self.search_last.is_empty() {
                    return vec![Effect::Bell];
                }
                let pattern = self.search_last.clone();
                let from = self.char_idx(doc, self.cursor);
                self.clear_pending();
                if !self.jump_to_match(doc, &pattern, from, false) {
                    return vec![Effect::Bell];
                }
            }
            Key::Home => self.apply_motion(doc, '0', 1),
            Key::End => self.apply_motion(doc, '$', 1),
            Key::Escape => self.clear_pending(),
            _ => return vec![Effect::Bell],
        }
        Vec::new()
    }

    fn operator_key<B: Buffer>(&mut self, doc: &mut B, op: PendingOp) -> Vec<Effect> {
        if self.pending == Some(op) {
            let count = self.take_count();
            self.clear_pending();
            match op {
                PendingOp::Delete => self.delete_lines(doc, count),
                PendingOp::Yank => self.yank_lines(doc, count),
                PendingOp::Change => self.change_line(doc, count),
            }
            return Vec::new();
        }
        self.pending = Some(op);
        self.pending_g = false;
        Vec::new()
    }

    fn apply_motion<B: Buffer>(&mut self, doc: &mut B, kind: char, count: usize) {
        // Vim special case: `cw` on a word behaves like `ce`.
        let kind = if kind == 'w'
            && self.pending == Some(PendingOp::Change)
            && word_class(current_char(doc, self.cursor)) == WordClass::Word
        {
            'e'
        } else {
            kind
        };
        if let Some(op) = self.pending.take() {
            self.pending_g = false;
            self.count.clear();
            match kind {
                'h' | 'l' | '0' | '$' | 'w' | 'b' | 'e' => {
                    let (target, inclusive) = self.motion_target(doc, kind, count);
                    self.apply_operator(doc, op, target, inclusive);
                }
                'j' | 'k' => {
                    // Vertical motions under an operator stay charwise in v1;
                    // dd/yy/cc cover the linewise case.
                    let saved = self.cursor;
                    if kind == 'j' {
                        self.move_j(doc, count);
                    } else {
                        self.move_k(doc, count);
                    }
                    let target = self.cursor;
                    self.cursor = saved;
                    let inclusive = target.line != saved.line;
                    self.apply_operator(doc, op, target, inclusive);
                }
                _ => {}
            }
            return;
        }
        self.pending_g = false;
        self.count.clear();
        match kind {
            'h' => self.move_h(doc, count),
            'j' => self.move_j(doc, count),
            'k' => self.move_k(doc, count),
            'l' => self.move_l(doc, count),
            'w' | 'b' | 'e' => {
                let (target, _) = self.motion_target(doc, kind, count);
                self.goto(doc, target.line, target.col, true);
            }
            '0' => self.goto(doc, self.cursor.line, 0, true),
            '$' => {
                let len = line_len(doc, self.cursor.line);
                self.goto(doc, self.cursor.line, len.saturating_sub(1), true);
            }
            _ => {}
        }
        doc.end_run();
    }

    fn apply_motion_to<B: Buffer>(&mut self, doc: &mut B, target: Cursor) {
        if let Some(op) = self.pending.take() {
            self.pending_g = false;
            self.count.clear();
            self.apply_operator(doc, op, target, false);
            return;
        }
        self.pending_g = false;
        self.count.clear();
        self.goto(doc, target.line, target.col, true);
        doc.end_run();
    }

    fn motion_target<B: Buffer>(&self, doc: &B, kind: char, count: usize) -> (Cursor, bool) {
        match kind {
            'h' => (
                Cursor {
                    line: self.cursor.line,
                    col: self.cursor.col.saturating_sub(count),
                },
                false,
            ),
            'l' => {
                let len = line_len(doc, self.cursor.line);
                (
                    Cursor {
                        line: self.cursor.line,
                        col: (self.cursor.col + count).min(len.saturating_sub(1)),
                    },
                    false,
                )
            }
            'w' | 'b' | 'e' => self.word_target(doc, kind, count),
            '0' => (
                Cursor {
                    line: self.cursor.line,
                    col: 0,
                },
                false,
            ),
            '$' => {
                let len = line_len(doc, self.cursor.line);
                (
                    Cursor {
                        line: self.cursor.line,
                        col: len.saturating_sub(1),
                    },
                    true,
                )
            }
            _ => (self.cursor, false),
        }
    }
}

fn doc_is_empty<B: Buffer>(doc: &B) -> bool {
    doc.len_chars() == 0
}

fn range_text<B: Buffer>(doc: &B, from: usize, to: usize) -> String {
    let (from, to) = (from.min(to), from.max(to).min(doc.len_chars()));
    // Reconstruct from lines to avoid another full-text accessor on the trait.
    let mut out = String::new();
    let mut idx = 0;
    for line in 0..doc.line_count() {
        if line > 0 {
            if idx >= from && idx < to {
                out.push('\n');
            }
            idx += 1;
        }
        for c in doc.line_text(line).chars() {
            if idx >= from && idx < to {
                out.push(c);
            }
            idx += 1;
            if idx >= to {
                return out;
            }
        }
    }
    out
}

impl ModalEngine for VimEngine {
    fn mode(&self) -> Mode {
        self.mode
    }

    fn cursor(&self) -> Cursor {
        self.cursor
    }

    fn selection(&self) -> Option<(Cursor, Cursor)> {
        match self.mode {
            Mode::Visual | Mode::VisualLine => {
                let a = self.anchor;
                let b = self.cursor;
                Some(if (a.line, a.col) <= (b.line, b.col) {
                    (a, b)
                } else {
                    (b, a)
                })
            }
            _ => None,
        }
    }

    fn command_line(&self) -> &str {
        &self.command
    }

    fn press_key<B: Buffer>(&mut self, doc: &mut B, key: Key) -> Vec<Effect> {
        match self.mode {
            Mode::Normal => self.normal_key(doc, key),
            Mode::Insert => self.insert_key(doc, key),
            Mode::Visual | Mode::VisualLine => self.visual_key(doc, key),
            Mode::Command => self.command_key(doc, key),
        }
    }

    fn type_text<B: Buffer>(&mut self, doc: &mut B, text: &str) -> Vec<Effect> {
        if self.mode != Mode::Insert || text.is_empty() {
            return Vec::new();
        }
        let at = self.char_idx(doc, self.cursor);
        doc.insert(at, text);
        let (line, col) =
            doc.line_col_at(at + text.chars().count().min(doc.len_chars().saturating_sub(at)));
        self.cursor = Cursor { line, col };
        self.preferred_col = col;
        self.clamp(doc);
        Vec::new()
    }
}

impl VimEngine {
    fn insert_key<B: Buffer>(&mut self, doc: &mut B, key: Key) -> Vec<Effect> {
        match key {
            Key::Escape => {
                self.enter_normal(doc);
            }
            Key::Enter => {
                let at = self.char_idx(doc, self.cursor);
                doc.insert(at, "\n");
                let (line, col) = doc.line_col_at(at + 1);
                self.cursor = Cursor { line, col };
                self.preferred_col = col;
                self.clamp(doc);
            }
            Key::Backspace => {
                let at = self.char_idx(doc, self.cursor);
                if at > 0 {
                    doc.delete(at - 1..at);
                    let (line, col) = doc.line_col_at(at - 1);
                    self.cursor = Cursor { line, col };
                    self.preferred_col = col;
                }
                self.clamp(doc);
            }
            Key::Delete => {
                let at = self.char_idx(doc, self.cursor);
                if at < doc.len_chars() {
                    doc.delete(at..at + 1);
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
            Key::Char(_) => return vec![Effect::Bell],
        }
        Vec::new()
    }

    fn visual_key<B: Buffer>(&mut self, doc: &mut B, key: Key) -> Vec<Effect> {
        let linewise = self.mode == Mode::VisualLine;
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
            }
            Key::Char('h') | Key::Left => self.move_h(doc, count),
            Key::Char('j') | Key::Down => self.move_j(doc, count),
            Key::Char('k') | Key::Up => self.move_k(doc, count),
            Key::Char('l') | Key::Right => self.move_l(doc, count),
            Key::Char('w') | Key::Char('W') | Key::Char('b') | Key::Char('B') => {
                let kind = if matches!(key, Key::Char('w') | Key::Char('W')) {
                    'w'
                } else {
                    'b'
                };
                let (target, _) = self.motion_target(doc, kind, count);
                self.goto(doc, target.line, target.col, true);
                self.count.clear();
            }
            Key::Char('e') | Key::Char('E') => {
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
                self.yank_selection(doc, linewise);
                self.enter_normal(doc);
            }
            Key::Char('d') | Key::Char('x') => {
                self.delete_selection(doc, linewise);
                self.enter_normal(doc);
            }
            Key::Char('c') => {
                self.delete_selection(doc, linewise);
                self.enter_insert();
                self.clamp(doc);
            }
            _ => return vec![Effect::Bell],
        }
        Vec::new()
    }

    fn selection_range<B: Buffer>(&self, doc: &B, linewise: bool) -> (usize, usize) {
        let a = self.char_idx(doc, self.anchor);
        let mut b = self.char_idx(doc, self.cursor);
        if linewise {
            let (al, _) = doc.line_col_at(a);
            let (bl, _) = doc.line_col_at(b);
            let (lo, hi) = (al.min(bl), al.max(bl));
            let from = doc.char_at_line_col(lo, 0);
            b = if hi + 1 < doc.line_count() {
                doc.char_at_line_col(hi + 1, 0)
            } else {
                doc.len_chars()
            };
            return (from, b);
        }
        // Charwise selections include the cursor cell.
        b = (b + 1).min(doc.len_chars());
        (a.min(b), a.max(b))
    }

    fn yank_selection<B: Buffer>(&mut self, doc: &mut B, linewise: bool) {
        let (from, to) = self.selection_range(doc, linewise);
        let mut text = range_text(doc, from, to);
        if linewise && text.ends_with('\n') {
            text.pop();
        }
        self.register.text = text;
        self.register.linewise = linewise;
        self.cursor = self.anchor;
    }

    fn delete_selection<B: Buffer>(&mut self, doc: &mut B, linewise: bool) {
        let (from, to) = self.selection_range(doc, linewise);
        let cursor = self.delete_range(doc, from, to);
        self.cursor = cursor;
    }

    fn command_key<B: Buffer>(&mut self, doc: &mut B, key: Key) -> Vec<Effect> {
        match key {
            Key::Escape => {
                self.command.clear();
                self.enter_normal(doc);
            }
            Key::Enter => {
                if self.command.starts_with('/') {
                    // Vim `/pattern`: jump to the next match, wrapping.
                    // An empty pattern repeats the last search.
                    let pattern = if self.command.len() > 1 {
                        self.command[1..].to_string()
                    } else {
                        self.search_last.clone()
                    };
                    if pattern.is_empty() {
                        return vec![Effect::Bell];
                    }
                    self.search_last = pattern.clone();
                    let from = self.char_idx(doc, self.cursor);
                    self.command.clear();
                    self.enter_normal(doc);
                    if !self.jump_to_match(doc, &pattern, from, true) {
                        return vec![Effect::Bell];
                    }
                    return Vec::new();
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Doc;

    fn engine(text: &str) -> (VimEngine, Doc) {
        (VimEngine::new(), Doc::new(text))
    }

    fn keys<B: Buffer>(eng: &mut VimEngine, doc: &mut B, seq: &str) -> Vec<Effect> {
        let mut out = Vec::new();
        for c in seq.chars() {
            out.extend(eng.press_key(doc, Key::Char(c)));
        }
        out
    }

    #[test]
    fn hjkl_moves_with_counts_and_clamps() {
        // "ab\ncdef\n" is two vim lines; the trailing empty row is the file's
        // final newline, not an addressable line.
        let (mut eng, mut doc) = engine("ab\ncdef\n");
        keys(&mut eng, &mut doc, "l");
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 1 });
        keys(&mut eng, &mut doc, "2j");
        assert_eq!(eng.cursor().line, 1);
        keys(&mut eng, &mut doc, "10k");
        assert_eq!(eng.cursor().line, 0);
    }

    #[test]
    fn word_motions_cross_words() {
        let (mut eng, mut doc) = engine("foo bar  baz");
        keys(&mut eng, &mut doc, "w");
        assert_eq!(eng.cursor().col, 4);
        keys(&mut eng, &mut doc, "e");
        assert_eq!(eng.cursor().col, 6);
        keys(&mut eng, &mut doc, "b");
        assert_eq!(eng.cursor().col, 4);
    }

    #[test]
    fn delete_word_removes_through_blank() {
        let (mut eng, mut doc) = engine("foo bar");
        keys(&mut eng, &mut doc, "dw");
        assert_eq!(doc.text(), "bar");
    }

    #[test]
    fn change_word_enters_insert() {
        let (mut eng, mut doc) = engine("foo bar");
        keys(&mut eng, &mut doc, "cw");
        assert_eq!(eng.mode(), Mode::Insert);
        assert_eq!(doc.text(), " bar");
        eng.type_text(&mut doc, "baz");
        assert_eq!(doc.text(), "baz bar");
    }

    #[test]
    fn line_operators_delete_yank_paste() {
        let (mut eng, mut doc) = engine("one\ntwo\nthree\n");
        keys(&mut eng, &mut doc, "j");
        keys(&mut eng, &mut doc, "dd");
        assert_eq!(doc.text(), "one\nthree\n");
        keys(&mut eng, &mut doc, "p");
        assert_eq!(doc.text(), "one\nthree\ntwo\n");
    }

    #[test]
    fn undo_restores_deleted_line() {
        let (mut eng, mut doc) = engine("one\ntwo\n");
        keys(&mut eng, &mut doc, "dd");
        assert_eq!(doc.text(), "two\n");
        keys(&mut eng, &mut doc, "u");
        assert_eq!(doc.text(), "one\ntwo\n");
    }

    #[test]
    fn open_line_below_and_escape() {
        let (mut eng, mut doc) = engine("a\nb\n");
        keys(&mut eng, &mut doc, "o");
        assert_eq!(eng.mode(), Mode::Insert);
        eng.type_text(&mut doc, "x");
        eng.press_key(&mut doc, Key::Escape);
        assert_eq!(doc.text(), "a\nx\nb\n");
        assert_eq!(eng.mode(), Mode::Normal);
    }

    #[test]
    fn visual_yank_and_paste() {
        let (mut eng, mut doc) = engine("hello");
        keys(&mut eng, &mut doc, "vll");
        assert!(eng.selection().is_some());
        keys(&mut eng, &mut doc, "y");
        keys(&mut eng, &mut doc, "$p");
        assert_eq!(doc.text(), "hellohel");
    }

    #[test]
    fn ex_save_quit_and_unknown() {
        let (mut eng, mut doc) = engine("x");
        keys(&mut eng, &mut doc, ":qw");
        let fx = eng.press_key(&mut doc, Key::Enter);
        assert_eq!(fx, vec![Effect::WriteQuit { force: false }]);
        keys(&mut eng, &mut doc, ":nope");
        let fx = eng.press_key(&mut doc, Key::Enter);
        assert_eq!(fx, vec![Effect::Bell]);
        assert_eq!(eng.mode(), Mode::Normal);
    }

    #[test]
    fn ex_write_quit_matrix() {
        fn ex(command: &str) -> Vec<Effect> {
            let (mut eng, mut doc) = engine("x");
            keys(&mut eng, &mut doc, &format!(":{command}"));
            eng.press_key(&mut doc, Key::Enter)
        }
        assert_eq!(ex("w"), vec![Effect::Save]);
        assert_eq!(ex("w!"), vec![Effect::SaveForce]);
        assert_eq!(ex("q"), vec![Effect::Quit]);
        assert_eq!(ex("q!"), vec![Effect::QuitForce]);
        assert_eq!(ex("wq"), vec![Effect::WriteQuit { force: false }]);
        assert_eq!(ex("qw"), vec![Effect::WriteQuit { force: false }]);
        assert_eq!(ex("x"), vec![Effect::WriteQuit { force: false }]);
        assert_eq!(ex("wq!"), vec![Effect::WriteQuit { force: true }]);
        assert_eq!(ex("x!"), vec![Effect::WriteQuit { force: true }]);
        assert_eq!(ex("qa"), vec![Effect::QuitAll { force: false }]);
        assert_eq!(ex("qa!"), vec![Effect::QuitAll { force: true }]);
        assert_eq!(ex("wqa"), vec![Effect::Bell]);
    }

    #[test]
    fn goto_lines_with_g() {
        let (mut eng, mut doc) = engine("a\nb\nc\n");
        keys(&mut eng, &mut doc, "G");
        assert_eq!(eng.cursor().line, 2);
        keys(&mut eng, &mut doc, "gg");
        assert_eq!(eng.cursor().line, 0);
        keys(&mut eng, &mut doc, "2G");
        assert_eq!(eng.cursor().line, 1);
    }

    #[test]
    fn space_moves_like_l() {
        let (mut eng, mut doc) = engine("ab");
        keys(&mut eng, &mut doc, " ");
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 1 });
    }

    #[test]
    fn dollar_reaches_end_of_line() {
        let (mut eng, mut doc) = engine("ab\ncdef\n");
        keys(&mut eng, &mut doc, "$");
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 1 });
    }

    #[test]
    fn insert_typing_and_dollar_append() {
        let (mut eng, mut doc) = engine("hi");
        keys(&mut eng, &mut doc, "A");
        eng.type_text(&mut doc, "!");
        assert_eq!(doc.text(), "hi!");
        eng.press_key(&mut doc, Key::Escape);
        keys(&mut eng, &mut doc, "x");
        assert_eq!(doc.text(), "hi");
    }

    #[test]
    fn select_all_covers_whole_buffer() {
        let (mut eng, doc) = engine("a\nb\n");
        eng.select_all(&doc);
        assert_eq!(eng.mode(), Mode::Visual);
        assert_eq!(
            eng.selection(),
            Some((Cursor { line: 0, col: 0 }, Cursor { line: 2, col: 0 }))
        );
    }

    #[test]
    fn copy_without_selection_takes_cursor_line() {
        let (mut eng, mut doc) = engine("ab\ncd\n");
        eng.place_cursor(&mut doc, 1, 0);
        assert_eq!(eng.copy_text(&doc), "cd");
        // Pure: mode, cursor, and undo are untouched.
        assert_eq!(eng.mode(), Mode::Normal);
        assert_eq!(eng.cursor(), Cursor { line: 1, col: 0 });
    }

    #[test]
    fn copy_selection_is_charwise() {
        let (mut eng, mut doc) = engine("ab\ncd\n");
        keys(&mut eng, &mut doc, "vl");
        assert_eq!(eng.copy_text(&doc), "ab");
    }

    #[test]
    fn cut_line_removes_newline_and_parks_cursor() {
        let (mut eng, mut doc) = engine("a\nb\n");
        let removed = eng.cut(&mut doc);
        assert_eq!(removed, "a\n");
        assert_eq!(doc.text(), "b\n");
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 0 });
    }

    #[test]
    fn cut_selection_collapses_to_normal() {
        let (mut eng, mut doc) = engine("ab\ncd\n");
        keys(&mut eng, &mut doc, "vl");
        let removed = eng.cut(&mut doc);
        assert_eq!(removed, "ab");
        assert_eq!(doc.text(), "\ncd\n");
        assert_eq!(eng.mode(), Mode::Normal);
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 0 });
    }

    #[test]
    fn paste_text_inserts_at_cursor_in_normal() {
        // Insert-at-cursor (not vim `p` after-cursor) so a copied line
        // pastes back as a line instead of splitting one.
        let (mut eng, mut doc) = engine("ac");
        eng.paste_text(&mut doc, "b");
        assert_eq!(doc.text(), "bac");
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 1 });
        assert_eq!(eng.mode(), Mode::Normal);
    }

    #[test]
    fn slash_search_jumps_and_n_cycles_with_wrap() {
        let (mut eng, mut doc) = engine("foo\nbar foo\n");
        keys(&mut eng, &mut doc, "/foo");
        assert_eq!(eng.mode(), Mode::Command);
        eng.press_key(&mut doc, Key::Enter);
        assert_eq!(eng.mode(), Mode::Normal);
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 0 });
        assert!(eng.take_scroll_request());
        keys(&mut eng, &mut doc, "n");
        assert_eq!(eng.cursor(), Cursor { line: 1, col: 4 });
        keys(&mut eng, &mut doc, "n");
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 0 });
        keys(&mut eng, &mut doc, "N");
        assert_eq!(eng.cursor(), Cursor { line: 1, col: 4 });
    }

    #[test]
    fn search_without_match_bells_and_stays_put() {
        let (mut eng, mut doc) = engine("foo\n");
        keys(&mut eng, &mut doc, "/zzz");
        let fx = eng.press_key(&mut doc, Key::Enter);
        assert_eq!(fx, vec![Effect::Bell]);
        assert_eq!(eng.mode(), Mode::Normal);
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 0 });
    }

    #[test]
    fn search_without_history_bells() {
        let (mut eng, mut doc) = engine("foo\n");
        assert_eq!(keys(&mut eng, &mut doc, "n"), vec![Effect::Bell]);
        keys(&mut eng, &mut doc, "/");
        let fx = eng.press_key(&mut doc, Key::Enter);
        assert_eq!(fx, vec![Effect::Bell]);
    }

    #[test]
    fn empty_search_pattern_repeats_last() {
        let (mut eng, mut doc) = engine("foo foo\n");
        keys(&mut eng, &mut doc, "/foo");
        eng.press_key(&mut doc, Key::Enter);
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 0 });
        keys(&mut eng, &mut doc, "n");
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 4 });
        keys(&mut eng, &mut doc, "/");
        eng.press_key(&mut doc, Key::Enter);
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 4 });
    }

    #[test]
    fn ide_undo_redo_round_trip() {
        let (mut eng, mut doc) = engine("");
        keys(&mut eng, &mut doc, "i");
        eng.type_text(&mut doc, "x");
        eng.press_key(&mut doc, Key::Escape);
        eng.undo(&mut doc);
        assert_eq!(doc.text(), "");
        eng.redo(&mut doc);
        assert_eq!(doc.text(), "x");
    }

    #[test]
    fn vimrc_set_applies_ignorecase_and_strips_trailing_comments() {
        assert!(parse_vimrc("set ignorecase\n").ignorecase);
        assert!(parse_vimrc("set ignorecase \" trailing comment\n").ignorecase);
        assert!(parse_vimrc("set ignorecase\t\" tab comment\n").ignorecase);
        assert!(!parse_vimrc("\" full-line comment\nset noignorecase\n").ignorecase);
        assert!(!parse_vimrc("set ignorecase\"glued quote is not a comment\n").ignorecase);
    }
}
