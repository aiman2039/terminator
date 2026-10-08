//! Minimal vim-modal engine over [`crate::doc::Buffer`].
//!
//! Scope (the Minimal tier): Normal / Insert / Visual / `VisualLine` / Command
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
    VisualBlock,
    Command,
}

impl Mode {
    /// Any selection mode (charwise, linewise, or block).
    #[must_use]
    pub fn is_visual(self) -> bool {
        matches!(self, Mode::Visual | Mode::VisualLine | Mode::VisualBlock)
    }
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
    /// Visual-block toggle. The view routes Ctrl-V here in vim
    /// Normal/Visual modes (where vim owns it, IdeaVim-style) and keeps
    /// it as paste everywhere else, so modeless editing never loses paste.
    CtrlV,
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
    /// Open the workspace split below (`:split`); the GUI owns layout.
    Split,
    /// Open the workspace split beside (`:vsplit`); the GUI owns layout.
    Vsplit,
    /// Open a new workspace tab (`:tabnew`); the GUI owns layout.
    TabNew,
    /// Show language-server hover for the cursor (`K` in Normal,
    /// Ctrl+K modeless); the GUI owns servers and popups.
    Hover,
    /// Jump to the definition under the cursor (`gd`, F12); the GUI
    /// owns servers and opening the target.
    GotoDefinition,
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
    /// Set by blockwise yank: column count of the yanked rectangle, so a
    /// later `p` pastes a block instead of sequential lines.
    block_width: Option<usize>,
}

/// One recorded input bit for `.` repeat: the change replays by parsing
/// these, never by re-reading the buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Bit {
    Key(Key),
    /// Pending operator (`d`, `c`, `y`).
    Op(char),
    /// Motion completing an operator (`w`, `$`, `G`, `g` for `gg`, …).
    Motion(char),
    /// Text typed (or Enter as `"\n"`) during an insert session.
    Type(String),
}

/// A buffer-mutating change, replayed by `.` at the current cursor.
/// Charwise visual changes are not repeatable yet and clear this (Bell).
#[derive(Clone, Debug, PartialEq, Eq)]
enum Repeat {
    Insert {
        text: String,
        place: Place,
    },
    DeleteChars {
        count: usize,
    },
    DeleteMotion {
        motion: char,
        count: usize,
    },
    DeleteLines {
        count: usize,
    },
    /// Blockwise delete replays from rectangle dims (motions cannot
    /// reconstruct ragged columns): `right` is the inclusive max corner.
    BlockDelete {
        height: usize,
        left: usize,
        right: usize,
    },
    ChangeMotion {
        motion: char,
        count: usize,
        text: String,
    },
    ChangeLines {
        count: usize,
        text: String,
    },
    BlockInsert {
        text: String,
        height: usize,
        left: usize,
        append: bool,
    },
    BlockChange {
        text: String,
        height: usize,
        left: usize,
        right: usize,
        append: bool,
    },
    Paste {
        count: usize,
    },
}

/// Where an insert session parks the cursor before typing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    AtCursor,
    AfterCursor,
    LineEnd,
    FirstBlank,
    LineBelow,
    LineAbove,
}

/// Live visual-block insert (`I`/`A`/`c`): typed text replays on the
/// remaining lines when the session ends. `right` is the inclusive max
/// corner column (append inserts just past it per row); `skip_first`
/// leaves the top row alone — it holds the live session's typing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BlockPending {
    top: usize,
    bottom: usize,
    left: usize,
    right: usize,
    append: bool,
    skip_first: bool,
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
    /// An explicit undo unit is open (an insert session, possibly with its
    /// preceding delete for `c`/`cc`): one vim change, one undo step.
    insert_group: bool,
    /// Live block insert awaiting its replay on Escape.
    block_pending: Option<BlockPending>,
    /// Yanked text awaiting host clipboard sync (`clipboard=unnamed`):
    /// every unnamed-register write stages here; the view forwards it to
    /// the system clipboard and clears it. Never affects editing.
    clipboard: Option<String>,
    /// Bits of the change in progress; parsed into [`Repeat`] when it
    /// completes. Skipped while replaying.
    recording: Vec<Bit>,
    replaying: bool,
    last_change: Option<Repeat>,
    /// The recorded typed text is exactly the session's net insertion.
    /// Cleared by unattributable edits (typing mid-text after a cursor
    /// move, forward delete, backspacing past session text, IDE cut):
    /// with it false, no insert repeat or block replay is built, so `.`
    /// bells instead of replaying wrong text.
    typed_exact: bool,
    /// Session insertion point range: typing at `typed_end` extends the
    /// recording exactly; anywhere else ends attribution. Both are char
    /// indices captured when the session opens.
    typed_start: usize,
    typed_end: usize,
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
            insert_group: false,
            block_pending: None,
            clipboard: None,
            recording: Vec::new(),
            replaying: false,
            last_change: None,
            typed_exact: true,
            typed_start: 0,
            typed_end: 0,
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
#[must_use]
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
            && let Some(prefix) = line.get(..pos)
            && prefix.ends_with([' ', '\t'])
        {
            line = prefix.trim_end();
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
    #[must_use]
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
        let Some(&ch) = text.get(i) else {
            break;
        };
        let cls = word_class(ch);
        if cls != WordClass::Blank {
            while text.get(i).is_some_and(|ch| word_class(*ch) == cls) {
                i = i.checked_add(1).unwrap_or(text.len());
            }
        }
        while text
            .get(i)
            .is_some_and(|ch| word_class(*ch) == WordClass::Blank)
        {
            i = i.checked_add(1).unwrap_or(text.len());
        }
    }
    i.min(text.len())
}

fn word_backward(text: &[char], mut i: usize, count: usize) -> usize {
    for _ in 0..count {
        if i == 0 {
            break;
        }
        let mut j = i.saturating_sub(1);
        while j > 0
            && text
                .get(j)
                .is_some_and(|ch| word_class(*ch) == WordClass::Blank)
        {
            j = j.saturating_sub(1);
        }
        let cls = word_class(text.get(j).copied().unwrap_or(' '));
        while j > 0
            && j.checked_sub(1)
                .and_then(|prev| text.get(prev).copied())
                .is_some_and(|ch| word_class(ch) == cls)
        {
            j = j.saturating_sub(1);
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
        i = i
            .checked_add(1)
            .unwrap_or(i)
            .min(text.len().saturating_sub(1));
        while text
            .get(i)
            .is_some_and(|ch| word_class(*ch) == WordClass::Blank)
        {
            i = i.checked_add(1).unwrap_or(text.len());
        }
        if i >= text.len() {
            break;
        }
        let Some(ch) = text.get(i).copied() else {
            break;
        };
        let cls = word_class(ch);
        while i
            .checked_add(1)
            .and_then(|next| text.get(next).copied())
            .is_some_and(|ch| word_class(ch) == cls)
        {
            i = i.checked_add(1).unwrap_or(i);
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
        let prev = lines.saturating_sub(1);
        if lines > 1 && doc.line_text(prev).is_empty() {
            lines.saturating_sub(2)
        } else {
            prev
        }
    }

    fn clamp<B: Buffer>(&mut self, doc: &B) {
        let last = if self.mode == Mode::Insert {
            // Insert may sit just past a trailing newline (the cursor stays
            // where editing left it), so backspace can delete that newline
            // and keep going into the previous line.
            doc.line_count().saturating_sub(1)
        } else {
            Self::last_addressable(doc)
        };
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

    /// Editing can leave the cursor just past a trailing newline, a spot only
    /// insert mode holds. Other modes land at the end of the last real line
    /// (vim has no cursor past the final newline) instead of line start.
    fn settle_cursor<B: Buffer>(&mut self, doc: &B) {
        if self.mode == Mode::Insert {
            return;
        }
        let last = Self::last_addressable(doc);
        if self.cursor.line > last {
            self.cursor = Cursor {
                line: last,
                col: line_len(doc, last),
            };
        }
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

    /// Begin an insert session: one undo unit covering the session (and a
    /// preceding operator delete for `c`/`cc`, whose caller opens the unit
    /// first so delete-plus-insert undoes as one change). Re-entering
    /// while a unit is open (operator delete, then typing) joins it.
    fn begin_insert_session<B: Buffer>(&mut self, doc: &mut B) {
        if !self.insert_group {
            doc.begin_undo_group();
            self.insert_group = true;
        }
        self.mode = Mode::Insert;
        self.clear_pending();
        self.typed_exact = true;
        self.typed_start = self.char_idx(doc, self.cursor);
        self.typed_end = self.typed_start;
    }

    /// Open the unit for an operator delete that flows into insert
    /// (`c{motion}`, `cc`): the delete banks into the session's unit.
    fn begin_change_group<B: Buffer>(&mut self, doc: &mut B) {
        if !self.insert_group {
            doc.begin_undo_group();
            self.insert_group = true;
        }
    }

    fn enter_normal<B: Buffer>(&mut self, doc: &mut B) {
        let ending_insert = self.mode == Mode::Insert;
        self.mode = Mode::Normal;
        self.clear_pending();
        self.command.clear();
        if ending_insert {
            // Block insert replays the session's net typed text on the
            // remaining rows, inside the same undo unit. Only pure typing
            // on the top row replays: cursor moves or newlines end the
            // session without replicating (buffer stays correct).
            let pending = self.block_pending.take();
            if let Some(block) = pending
                && self.cursor.line == block.top
                && self.typed_exact
            {
                let text = Self::typed_text(&self.recording);
                if !text.is_empty() {
                    self.apply_block_insert(doc, &block, &text);
                }
            }
            if !self.replaying {
                self.last_change = if self.typed_exact {
                    Self::parse_repeat(&self.recording, pending)
                } else {
                    None
                };
            }
            self.recording.clear();
            if self.insert_group {
                doc.end_undo_group();
                self.insert_group = false;
            }
        }
        doc.end_run();
        self.settle_cursor(doc);
        self.clamp(doc);
    }

    /// Record one input bit for `.` repeat. Skipped while replaying, so a
    /// repeat never overwrites the change it replays.
    fn rec_bit(&mut self, bit: Bit) {
        if !self.replaying {
            self.recording.push(bit);
        }
    }

    /// Drop the change in progress, keeping the last completed change:
    /// navigation, yank, undo, ex commands, and search are not repeatable.
    fn rec_discard(&mut self) {
        if !self.replaying {
            self.recording.clear();
        }
    }

    /// Complete a Normal-mode change with no insert session (`x`, `dd`,
    /// `dw`, `p`): parse the recorded bits into the last change.
    fn rec_commit(&mut self) {
        if !self.replaying {
            self.last_change = Self::parse_repeat(&self.recording, None);
            self.recording.clear();
        }
    }

    /// Net typed text of the current recording: concatenated `Type` bits.
    /// Backspace pops the last typed char when it can only erase session
    /// text; anything else (arrows, forward delete, clicks, IDE paste)
    /// ends attribution, so replay and block insert stay exact.
    fn typed_text(recording: &[Bit]) -> String {
        let mut out = String::new();
        for bit in recording {
            if let Bit::Type(text) = bit {
                out.push_str(text);
            }
        }
        out
    }

    /// Pointer click: move the cursor (or the active end of a selection).
    /// Never edits, so it returns no effects. Outside an insert session a
    /// click is navigation and drops the change in progress; inside one
    /// the typed text stays exactly attributable, so recording continues.
    pub fn place_cursor<B: Buffer>(&mut self, doc: &mut B, line: usize, col: usize) -> Vec<Effect> {
        if self.mode == Mode::Command {
            return Vec::new();
        }
        self.clear_pending();
        doc.end_run();
        self.cursor = Cursor { line, col };
        if !matches!(
            self.mode,
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock
        ) {
            self.anchor = self.cursor;
        }
        if self.mode != Mode::Insert {
            self.rec_discard();
        }
        self.preferred_col = col;
        self.clamp(doc);
        Vec::new()
    }

    fn delete_range<B: Buffer>(&mut self, doc: &mut B, from: usize, to: usize) -> Cursor {
        let (from, to) = (from.min(to), from.max(to));
        // A charwise delete invalidates any yanked rectangle: the kept
        // text (if any) pastes sequentially from here on.
        self.register.block_width = None;
        doc.delete(from..to);
        doc.end_run();
        let (line, col) = doc.line_col_at(from.min(doc.len_chars()));
        Cursor { line, col }
    }

    /// Whole-buffer visual selection (IDE select-all, Cmd/Ctrl+A).
    /// An IDE action, not a vim change: the last repeatable change keeps.
    pub fn select_all<B: Buffer>(&mut self, doc: &B) {
        self.rec_discard();
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
        if self.mode == Mode::VisualBlock {
            return self.block_text(doc).unwrap_or_default();
        }
        if self.selection().is_some() {
            let linewise = self.mode == Mode::VisualLine;
            let (from, to) = self.selection_range(doc, linewise);
            return range_text(doc, from, to);
        }
        let line = self.cursor.line.min(doc.line_count().saturating_sub(1));
        doc.line_text(line)
    }

    /// IDE cut: remove the copy source and park the cursor on the cut point.
    /// A cut selection collapses back to Normal, mirroring `d`. An IDE
    /// action, not a vim change: recording continues only inside an insert
    /// session, where the cut ends typed-text attribution.
    pub fn cut<B: Buffer>(&mut self, doc: &mut B) -> String {
        if self.mode == Mode::Insert {
            self.typed_exact = false;
        } else {
            self.rec_discard();
        }
        // A stale blockwise flag must not survive an IDE cut: the next `p`
        // would otherwise paste a rectangle from unrelated text.
        self.register.block_width = None;
        if self.mode == Mode::VisualBlock {
            let text = self.block_text(doc).unwrap_or_default();
            doc.begin_undo_group();
            let cursor = self.delete_selection_block(doc);
            doc.end_undo_group();
            self.cursor = cursor;
            self.preferred_col = cursor.col;
            self.enter_normal(doc);
            return text;
        }
        let linewise = self.mode == Mode::VisualLine;
        let (from, to) = if self.selection().is_some() {
            self.selection_range(doc, linewise)
        } else {
            let line = self.cursor.line.min(doc.line_count().saturating_sub(1));
            let from = doc.char_at_line_col(line, 0);
            let to = if line
                .checked_add(1)
                .is_some_and(|next| next < doc.line_count())
            {
                doc.char_at_line_col(line.checked_add(1).unwrap_or(line), 0)
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
    /// One undo unit; mode and selection are left alone. In Normal mode
    /// the paste becomes the repeatable change (insert at cursor); inside
    /// an insert session its text joins the session recording exactly when
    /// it lands at the session end.
    pub fn paste_text<B: Buffer>(&mut self, doc: &mut B, text: &str) {
        if text.is_empty() {
            return;
        }
        self.clear_pending();
        let at = self.char_idx(doc, self.cursor).min(doc.len_chars());
        if !self.replaying {
            if self.mode == Mode::Insert {
                self.recording.push(Bit::Type(text.to_string()));
                if at == self.typed_end {
                    self.typed_end = self.typed_end.saturating_add(text.chars().count());
                } else {
                    self.typed_exact = false;
                }
            } else if self.mode == Mode::Normal {
                self.last_change = Some(Repeat::Insert {
                    text: text.to_string(),
                    place: Place::AtCursor,
                });
                self.recording.clear();
            } else {
                self.rec_discard();
            }
        }
        doc.insert(at, text);
        doc.end_run();
        let end = at
            .checked_add(text.chars().count().min(doc.len_chars().saturating_sub(at)))
            .unwrap_or(at);
        let (line, col) = doc.line_col_at(end);
        self.cursor = Cursor { line, col };
        self.settle_cursor(doc);
        self.clamp(doc);
        self.preferred_col = self.cursor.col;
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
                let col = text.get(..byte).map_or(0, |prefix| prefix.chars().count());
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
    /// cursor clamped back into range afterwards. Undo is never repeatable.
    pub fn undo<B: Buffer>(&mut self, doc: &mut B) {
        doc.undo();
        self.clear_pending();
        self.rec_discard();
        self.clamp(doc);
    }

    /// See [`Self::undo`].
    pub fn redo<B: Buffer>(&mut self, doc: &mut B) {
        doc.redo();
        self.clear_pending();
        self.rec_discard();
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
            to = to
                .checked_add(1)
                .map_or(doc.len_chars(), |next| next.min(doc.len_chars()));
        }
        let (from, to) = (from.min(to), from.max(to));
        match op {
            PendingOp::Delete => {
                let cursor = self.delete_range(doc, from, to);
                self.cursor = cursor;
                self.enter_normal(doc);
                self.rec_commit();
            }
            PendingOp::Change => {
                self.begin_change_group(doc);
                let cursor = self.delete_range(doc, from, to);
                self.cursor = cursor;
                self.begin_insert_session(doc);
                self.clamp(doc);
            }
            PendingOp::Yank => {
                self.set_register(range_text(doc, from, to), false, None);
                self.cursor = target;
                self.enter_normal(doc);
                // Yank mutates nothing: the last change survives.
                self.rec_discard();
            }
        }
        Vec::new()
    }

    fn delete_lines<B: Buffer>(&mut self, doc: &mut B, count: usize) {
        let lines = doc.line_count();
        let start = self.cursor.line.min(lines.saturating_sub(1));
        let end = start.checked_add(count).map_or(lines, |end| end.min(lines));
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
        self.set_register(yanked, true, None);
        doc.delete(from..to);
        doc.end_run();
        self.cursor.line = start.min(doc.line_count().saturating_sub(1));
        self.cursor.col = Self::first_non_blank(doc, self.cursor.line);
        self.enter_normal(doc);
    }

    fn yank_lines<B: Buffer>(&mut self, doc: &mut B, count: usize) {
        let lines = doc.line_count();
        let start = self.cursor.line.min(lines.saturating_sub(1));
        let end = start.checked_add(count).map_or(lines, |end| end.min(lines));
        let mut yanked = Vec::new();
        for line in start..end {
            yanked.push(doc.line_text(line));
        }
        self.set_register(yanked.join("\n"), true, None);
        self.enter_normal(doc);
    }

    fn paste<B: Buffer>(&mut self, doc: &mut B) {
        if self.register.text.is_empty() {
            return;
        }
        if !self.register.linewise && self.register.block_width.is_some() {
            self.paste_block(doc);
            return;
        }
        if self.register.linewise {
            let line = self
                .cursor
                .line
                .checked_add(1)
                .map_or(doc.line_count(), |line| line.min(doc.line_count()));
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
            let at = self
                .char_idx(doc, self.cursor)
                .checked_add(1)
                .map_or(doc.len_chars(), |at| at.min(doc.len_chars()));
            doc.insert(at, &self.register.text);
            doc.end_run();
            let landed = at
                .checked_add(self.register.text.chars().count())
                .and_then(|sum| sum.checked_sub(1))
                .unwrap_or(at);
            let (line, col) = doc.line_col_at(landed);
            self.cursor = Cursor { line, col };
        }
        self.enter_normal(doc);
    }

    fn change_line<B: Buffer>(&mut self, doc: &mut B, count: usize) {
        let lines = doc.line_count();
        let start = self.cursor.line.min(lines.saturating_sub(1));
        let end = start.checked_add(count).map_or(lines, |end| end.min(lines));
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
        self.begin_insert_session(doc);
        self.clamp(doc);
    }

    /// Visual-block rectangle as (top line, bottom line, left column,
    /// right-exclusive column), unclamped: short rows clamp at each use.
    /// `None` outside visual-block mode.
    pub fn block_span(&self) -> Option<(usize, usize, usize, usize)> {
        if self.mode != Mode::VisualBlock {
            return None;
        }
        let top = self.anchor.line.min(self.cursor.line);
        let bottom = self.anchor.line.max(self.cursor.line);
        let left = self.anchor.col.min(self.cursor.col);
        let right = self.anchor.col.max(self.cursor.col);
        Some((top, bottom, left, right.saturating_add(1)))
    }

    /// One entry per row in `[top..=bottom]`: short rows contribute what
    /// they reach (possibly empty), so ragged text never shifts columns.
    fn block_rows<B: Buffer>(&self, doc: &B) -> Option<(usize, usize, Vec<String>)> {
        let (top, bottom, left, right) = self.block_span()?;
        let mut out = Vec::with_capacity(bottom.saturating_sub(top).saturating_add(1));
        for line in top..=bottom {
            let len = line_len(doc, line);
            if left >= len {
                out.push(String::new());
            } else {
                let text = doc.line_text(line);
                out.push(
                    text.chars()
                        .skip(left)
                        .take(right.saturating_sub(left))
                        .collect(),
                );
            }
        }
        Some((top, bottom, out))
    }

    /// Rectangle text, rows joined with newlines (for yank and IDE copy).
    fn block_text<B: Buffer>(&self, doc: &B) -> Option<String> {
        self.block_rows(doc).map(|(_, _, rows)| rows.join("\n"))
    }

    /// Delete explicit rectangle rows. The caller owns the undo unit, so
    /// plain deletes and change-into-insert share one code path.
    fn delete_block_ranges<B: Buffer>(
        &mut self,
        doc: &mut B,
        top: usize,
        bottom: usize,
        left: usize,
        right_excl: usize,
    ) {
        for line in top..=bottom {
            if line >= doc.line_count() {
                break;
            }
            let len = line_len(doc, line);
            if left < len {
                let from = doc.char_at_line_col(line, left);
                let to = doc.char_at_line_col(line, right_excl.min(len));
                if from < to {
                    doc.delete(from..to);
                }
            }
        }
    }

    /// Delete the live block selection; the caller owns the undo unit.
    /// Returns the rest position (top row, left edge clamped).
    fn delete_selection_block<B: Buffer>(&mut self, doc: &mut B) -> Cursor {
        let Some((top, bottom, left, right)) = self.block_span() else {
            return self.cursor;
        };
        self.delete_block_ranges(doc, top, bottom, left, right);
        Cursor {
            line: top,
            col: left.min(line_len(doc, top)),
        }
    }

    /// Write the unnamed register and stage the text for host clipboard
    /// sync. All register writes funnel through here — never assign the
    /// fields directly (except invalidation, which writes nothing).
    fn set_register(&mut self, text: String, linewise: bool, block_width: Option<usize>) {
        if !text.is_empty() {
            self.clipboard = Some(text.clone());
        }
        self.register.text = text;
        self.register.linewise = linewise;
        self.register.block_width = block_width;
    }

    /// Take staged clipboard text, if a yank or register write happened
    /// since the last take. Pure host plumbing.
    #[must_use]
    pub fn take_clipboard(&mut self) -> Option<String> {
        self.clipboard.take()
    }

    fn yank_block<B: Buffer>(&mut self, doc: &mut B) {
        let Some((_, _, rows)) = self.block_rows(doc) else {
            return;
        };
        let width = self
            .block_span()
            .map_or(0, |(_, _, left, right)| right.saturating_sub(left));
        self.set_register(rows.join("\n"), false, Some(width));
        self.cursor = self.anchor;
    }

    /// Paste a blockwise register: each yanked row inserts at the cursor
    /// column of successive lines, padding short lines with spaces the way
    /// vim does. Extends the buffer with blank lines past EOF.
    fn paste_block<B: Buffer>(&mut self, doc: &mut B) {
        let rows: Vec<&str> = self.register.text.split('\n').collect();
        if rows.iter().all(|row| row.is_empty()) {
            return;
        }
        let col = self.cursor.col;
        doc.begin_undo_group();
        for (i, part) in rows.iter().enumerate() {
            let line = self.cursor.line.saturating_add(i);
            while line >= doc.line_count() {
                doc.insert(doc.len_chars(), "\n");
            }
            let len = line_len(doc, line);
            if len >= col {
                doc.insert(doc.char_at_line_col(line, col), part);
            } else {
                let mut padded = " ".repeat(col.saturating_sub(len));
                padded.push_str(part);
                doc.insert(doc.char_at_line_col(line, len), &padded);
            }
        }
        doc.end_undo_group();
        let last = (self
            .cursor
            .line
            .saturating_add(rows.len())
            .saturating_sub(1))
        .min(doc.line_count().saturating_sub(1));
        self.cursor = Cursor {
            line: last,
            col: col.saturating_add(rows.last().map_or(0, |row| row.chars().count())),
        };
        self.enter_normal(doc);
    }

    /// Insert `text` on every row of `block`: `I` at the left edge, `A`
    /// just past the right edge, each clamped to its own row length (short
    /// rows take text at end-of-line, never padded). `skip_first` leaves
    /// the top row alone — it already holds the live session's typing.
    fn apply_block_insert<B: Buffer>(&mut self, doc: &mut B, block: &BlockPending, text: &str) {
        if text.is_empty() {
            return;
        }
        let first = if block.skip_first {
            block.top.saturating_add(1)
        } else {
            block.top
        };
        for line in first..=block.bottom {
            if line >= doc.line_count() {
                break;
            }
            let len = line_len(doc, line);
            let col = if block.append {
                block.right.saturating_add(1).min(len)
            } else {
                block.left.min(len)
            };
            doc.insert(doc.char_at_line_col(line, col), text);
        }
    }

    /// Parse recorded bits into the last change. Returns `None` for
    /// not-yet-repeatable shapes (charwise visual ops): the repeat then
    /// bells instead of replaying stale input.
    fn parse_repeat(bits: &[Bit], block: Option<BlockPending>) -> Option<Repeat> {
        #[derive(PartialEq)]
        enum Tok {
            Count(usize),
            Op(char),
            Motion(char),
            Key(Key),
        }
        let mut toks: Vec<Tok> = Vec::new();
        // Typed text joins separately below; only key order matters here.
        let mut texts: Vec<String> = Vec::new();
        for bit in bits {
            match bit {
                Bit::Key(Key::Char(c)) if c.is_ascii_digit() => {
                    // A leading `0` is the line-start motion (the count
                    // parser never emits one); digits after an operator or
                    // motion are a second count. Both multiply vim-style.
                    if *c == '0' && !matches!(toks.last(), Some(Tok::Count(_))) {
                        toks.push(Tok::Motion('0'));
                    } else {
                        let digit = (*c as usize).saturating_sub('0' as usize);
                        if let Some(Tok::Count(n)) = toks.last_mut() {
                            *n = n.saturating_mul(10).saturating_add(digit);
                        } else {
                            toks.push(Tok::Count(digit));
                        }
                    }
                }
                Bit::Key(key) => toks.push(Tok::Key(*key)),
                Bit::Op(op) => toks.push(Tok::Op(*op)),
                Bit::Motion(motion) => toks.push(Tok::Motion(*motion)),
                Bit::Type(chunk) => texts.push(chunk.clone()),
            }
        }
        let typed = texts.concat();
        let mut pos = 0usize;
        let mut count = 1usize;
        while let Some(Tok::Count(n)) = toks.get(pos) {
            count = count.saturating_mul(*n).max(1);
            pos = pos.saturating_add(1);
        }
        // Tokens after the leading count use indexed `.get()`, never slicing.
        let tail_len = toks.len().saturating_sub(pos);
        // A block session carries the Ctrl-V toggle; dims ride along from
        // the live pending state. The LAST visual marker wins, so entering
        // the block from charwise visual still replays as a block.
        let marker = toks
            .iter()
            .rfind(|tok| matches!(tok, Tok::Key(Key::CtrlV | Key::Char('v' | 'V'))));
        if marker.is_some() && !matches!(marker, Some(Tok::Key(Key::CtrlV))) {
            return None;
        }
        if matches!(marker, Some(Tok::Key(Key::CtrlV))) {
            let block = block?;
            let height = block.bottom.saturating_sub(block.top);
            let is_change = toks
                .iter()
                .any(|tok| matches!(tok, Tok::Key(Key::Char('c'))));
            if typed.is_empty() {
                return None;
            }
            return if is_change {
                Some(Repeat::BlockChange {
                    text: typed,
                    height,
                    left: block.left,
                    right: block.right,
                    append: block.append,
                })
            } else {
                Some(Repeat::BlockInsert {
                    text: typed,
                    height,
                    left: block.left,
                    append: block.append,
                })
            };
        }
        let head = toks.get(pos)?;
        match head {
            Tok::Key(Key::Char('x')) if tail_len == 1 => Some(Repeat::DeleteChars { count }),
            Tok::Key(Key::Char('p')) if tail_len == 1 => Some(Repeat::Paste { count }),
            Tok::Key(Key::Char(place @ ('i' | 'a' | 'A' | 'I' | 'o' | 'O'))) => {
                if tail_len != 1 || typed.is_empty() {
                    return None;
                }
                let place = match place {
                    'i' => Place::AtCursor,
                    'a' => Place::AfterCursor,
                    'A' => Place::LineEnd,
                    'I' => Place::FirstBlank,
                    'o' => Place::LineBelow,
                    _ => Place::LineAbove,
                };
                Some(Repeat::Insert { text: typed, place })
            }
            Tok::Op(op @ ('d' | 'c')) => {
                let mut off = 1usize;
                let mut count2 = 1usize;
                while let Some(Tok::Count(n)) = toks.get(pos.saturating_add(off)) {
                    count2 = count2.saturating_mul(*n).max(1);
                    off = off.saturating_add(1);
                }
                let total = count.saturating_mul(count2).max(1);
                // The change must end exactly here: no trailing tokens.
                let ends_here = toks.len() == pos.saturating_add(off).saturating_add(1);
                match toks.get(pos.saturating_add(off)) {
                    Some(Tok::Op(second)) if second == op && ends_here => {
                        if *op == 'd' {
                            Some(Repeat::DeleteLines { count: total })
                        } else {
                            // `cc` + immediate Escape deletes the lines; an
                            // empty insert replays as nothing to type.
                            Some(Repeat::ChangeLines {
                                count: total,
                                text: typed,
                            })
                        }
                    }
                    Some(Tok::Motion(motion)) if ends_here => {
                        // A bare `G` (count 0) means last line; every other
                        // motion defaults an absent count to 1.
                        let for_g = if *motion == 'G' && count == 1 && count2 == 1 {
                            0
                        } else {
                            total
                        };
                        if *op == 'd' {
                            Some(Repeat::DeleteMotion {
                                motion: *motion,
                                count: for_g,
                            })
                        } else {
                            Some(Repeat::ChangeMotion {
                                motion: *motion,
                                count: for_g,
                                text: typed,
                            })
                        }
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Replay a parsed change at the current cursor. Recording stays off
    /// (`replaying`), so the replay never becomes the new last change;
    /// undo grouping still applies, so a replayed change undoes as one.
    fn exec_repeat<B: Buffer>(&mut self, doc: &mut B, change: Repeat) -> Vec<Effect> {
        self.replaying = true;
        let out = match change {
            Repeat::Insert { text, place } => {
                self.start_insert_place(doc, place);
                self.type_text(doc, &text);
                self.enter_normal(doc);
                Vec::new()
            }
            Repeat::DeleteChars { count } => {
                self.delete_chars_forward(doc, count);
                Vec::new()
            }
            Repeat::DeleteMotion { motion, count } => {
                self.pending = Some(PendingOp::Delete);
                self.replay_motion(doc, motion, count);
                Vec::new()
            }
            Repeat::DeleteLines { count } => {
                self.delete_lines(doc, count);
                Vec::new()
            }
            Repeat::BlockDelete {
                height,
                left,
                right,
            } => {
                let top = self.cursor.line;
                let bottom = top
                    .saturating_add(height)
                    .min(doc.line_count().saturating_sub(1));
                doc.begin_undo_group();
                self.delete_block_ranges(doc, top, bottom, left, right.saturating_add(1));
                doc.end_undo_group();
                self.cursor = Cursor {
                    line: top,
                    col: left.min(line_len(doc, top)),
                };
                self.preferred_col = self.cursor.col;
                self.clamp(doc);
                Vec::new()
            }
            Repeat::ChangeMotion {
                motion,
                count,
                text,
            } => {
                self.begin_change_group(doc);
                self.pending = Some(PendingOp::Change);
                self.replay_motion(doc, motion, count);
                self.type_text(doc, &text);
                self.enter_normal(doc);
                Vec::new()
            }
            Repeat::ChangeLines { count, text } => {
                self.begin_change_group(doc);
                self.change_line(doc, count);
                self.type_text(doc, &text);
                self.enter_normal(doc);
                Vec::new()
            }
            Repeat::BlockInsert {
                text,
                height,
                left,
                append,
            } => {
                let top = self.cursor.line;
                let bottom = top
                    .saturating_add(height)
                    .min(doc.line_count().saturating_sub(1));
                let block = BlockPending {
                    top,
                    bottom,
                    left,
                    right: left,
                    append,
                    skip_first: false,
                };
                doc.begin_undo_group();
                self.apply_block_insert(doc, &block, &text);
                doc.end_undo_group();
                self.cursor = Cursor {
                    line: top,
                    col: left.min(line_len(doc, top)),
                };
                self.clamp(doc);
                Vec::new()
            }
            Repeat::BlockChange {
                text,
                height,
                left,
                right,
                append,
            } => {
                let top = self.cursor.line;
                let bottom = top
                    .saturating_add(height)
                    .min(doc.line_count().saturating_sub(1));
                let block = BlockPending {
                    top,
                    bottom,
                    left,
                    right,
                    append,
                    skip_first: false,
                };
                doc.begin_undo_group();
                self.delete_block_ranges(doc, top, bottom, left, right.saturating_add(1));
                self.apply_block_insert(doc, &block, &text);
                doc.end_undo_group();
                self.cursor = Cursor {
                    line: top,
                    col: left.min(line_len(doc, top)),
                };
                self.clamp(doc);
                Vec::new()
            }
            Repeat::Paste { count } => {
                for _ in 0..count.max(1) {
                    self.paste(doc);
                }
                Vec::new()
            }
        };
        self.replaying = false;
        self.clear_pending();
        out
    }

    /// Drive an operator motion the way the live keys do (including the
    /// `cw`→`ce` special case), for `.` replay.
    fn replay_motion<B: Buffer>(&mut self, doc: &mut B, motion: char, count: usize) {
        match motion {
            '^' | 'G' | 'g' => {
                let target = self.goto_target(doc, motion, count);
                self.apply_motion_to(doc, target, motion);
            }
            _ => {
                self.apply_motion(doc, motion, count.max(1));
            }
        }
    }

    /// Cursor target for the count-aware motions (`^`, `G`, `gg`), shared
    /// by the live keys and `.` replay. A `G` count of 0 means bare `G`
    /// (last line); every other motion treats 0 as 1.
    fn goto_target<B: Buffer>(&self, doc: &B, motion: char, count: usize) -> Cursor {
        match motion {
            '^' => Cursor {
                line: self.cursor.line,
                col: Self::first_non_blank(doc, self.cursor.line),
            },
            'G' => {
                let last = doc.line_count().saturating_sub(1);
                let line = if count == 0 {
                    last
                } else {
                    count.saturating_sub(1).min(last)
                };
                Cursor {
                    line,
                    col: Self::first_non_blank(doc, line),
                }
            }
            _ => Cursor {
                line: 0,
                col: Self::first_non_blank(doc, 0),
            },
        }
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
            if rest <= len || line.checked_add(1) == Some(doc.line_count()) {
                break;
            }
            let step = len.checked_add(1).unwrap_or(len);
            rest = rest.saturating_sub(step);
            line = line.checked_add(1).unwrap_or(line);
        }
        (Cursor { line, col: rest }, inclusive)
    }

    /// Park the cursor for an insert session and open its undo unit.
    /// Used by the live keys and `.` replay alike. The unit opens before
    /// positioning edits (`o`/`O` newlines), so the whole change undoes
    /// as one; typed-text tracking still starts from the parked cursor.
    fn start_insert_place<B: Buffer>(&mut self, doc: &mut B, place: Place) {
        self.begin_change_group(doc);
        match place {
            Place::AtCursor => {}
            Place::AfterCursor => {
                let len = line_len(doc, self.cursor.line);
                if len > 0 {
                    self.cursor.col = self
                        .cursor
                        .col
                        .checked_add(1)
                        .map_or(len, |col| col.min(len));
                }
            }
            Place::LineEnd => {
                self.cursor.col = line_len(doc, self.cursor.line);
            }
            Place::FirstBlank => {
                self.cursor.col = Self::first_non_blank(doc, self.cursor.line);
            }
            Place::LineBelow => {
                // Split at the start of the next line (or append): the new
                // blank line is always `cursor.line + 1`.
                let next = self.cursor.line.saturating_add(1);
                let at = if next < doc.line_count() {
                    doc.char_at_line_col(next, 0)
                } else {
                    doc.len_chars()
                };
                doc.insert(at, "\n");
                self.cursor = Cursor { line: next, col: 0 };
            }
            Place::LineAbove => {
                let at = doc.char_at_line_col(self.cursor.line, 0);
                doc.insert(at, "\n");
                let (line, _) = doc.line_col_at(at);
                self.cursor = Cursor { line, col: 0 };
            }
        }
        self.preferred_col = self.cursor.col;
        self.begin_insert_session(doc);
        self.clamp(doc);
    }

    /// Begin a visual-block insert (`I` left edge, `A` past the right
    /// edge, `c` after deleting): parks on the top row and stages the
    /// replay dims. The caller owns the undo unit.
    fn start_block_insert<B: Buffer>(&mut self, doc: &mut B, append: bool) {
        let (top, bottom, left, right) = match self.block_span() {
            Some(span) => span,
            None => return,
        };
        let right = right.saturating_sub(1);
        self.block_pending = Some(BlockPending {
            top,
            bottom,
            left,
            right,
            append,
            skip_first: true,
        });
        let col = if append {
            right.saturating_add(1)
        } else {
            left
        };
        self.cursor = Cursor {
            line: top,
            col: col.min(line_len(doc, top)),
        };
        self.preferred_col = self.cursor.col;
        self.begin_insert_session(doc);
        self.clamp(doc);
    }

    /// Forward character delete (`x` with count), shared by the live key
    /// and `.` replay. One buffer op: already a single undo step.
    fn delete_chars_forward<B: Buffer>(&mut self, doc: &mut B, count: usize) {
        let from = self.char_idx(doc, self.cursor);
        let len = line_len(doc, self.cursor.line);
        if self.cursor.col < len {
            let room = len.saturating_sub(self.cursor.col);
            let to = from
                .checked_add(count.max(1))
                .unwrap_or(from)
                .min(from.checked_add(room).unwrap_or(from));
            let cursor = self.delete_range(doc, from, to);
            self.cursor = cursor;
        }
        self.clear_pending();
        self.clamp(doc);
    }

    /// Open find (`/`) or an ex prompt from any mode (IDE Ctrl+F/Ctrl+H):
    /// the engine owns the prompt, the view only asks.
    pub fn begin_search(&mut self) {
        self.mode = Mode::Command;
        self.command.clear();
        self.command.push('/');
        self.clear_pending();
        self.rec_discard();
    }

    /// Open an ex prompt prefilled with `initial` (e.g. `"%s/"`).
    pub fn begin_ex(&mut self, initial: &str) {
        self.mode = Mode::Command;
        self.command.clear();
        self.command.push_str(initial);
        self.clear_pending();
        self.rec_discard();
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
            self.rec_bit(Bit::Key(key));
            return Vec::new();
        }
        // `gd` is goto-definition; any other non-`g` key after `g`
        // cancels the prefix.
        if self.pending_g && key != Key::Char('g') && key != Key::Char('d') {
            self.clear_pending();
            self.rec_discard();
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
            Key::Char('w' | 'W') => {
                self.apply_motion(doc, 'w', count);
            }
            Key::Char('b' | 'B') => {
                self.apply_motion(doc, 'b', count);
            }
            Key::Char('e' | 'E') => {
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
                    '^',
                );
            }
            Key::Char('G') => {
                let line = if self.count.is_empty() {
                    doc.line_count().saturating_sub(1)
                } else {
                    count.saturating_sub(1)
                };
                let col = Self::first_non_blank(doc, line.min(doc.line_count().saturating_sub(1)));
                self.apply_motion_to(doc, Cursor { line, col }, 'G');
                self.scroll_request = true;
            }
            Key::Char('g') => {
                if self.pending_g {
                    self.clear_pending();
                    let col = Self::first_non_blank(doc, 0);
                    self.apply_motion_to(doc, Cursor { line: 0, col }, 'g');
                    self.scroll_request = true;
                } else {
                    self.pending_g = true;
                    return Vec::new();
                }
            }
            Key::Char('x') => {
                self.rec_bit(Bit::Key(key));
                self.delete_chars_forward(doc, count);
                self.rec_commit();
            }
            Key::Char('d') => {
                if self.pending_g {
                    self.clear_pending();
                    self.rec_discard();
                    self.scroll_request = true;
                    return vec![Effect::GotoDefinition];
                }
                return self.operator_key(doc, PendingOp::Delete);
            }
            Key::Char('c') => {
                return self.operator_key(doc, PendingOp::Change);
            }
            Key::Char('y') => {
                return self.operator_key(doc, PendingOp::Yank);
            }
            Key::Char('p') => {
                self.rec_bit(Bit::Key(key));
                self.paste(doc);
                self.clear_pending();
                self.rec_commit();
            }
            Key::Char('.') => {
                if self.pending.is_some() {
                    self.clear_pending();
                    self.rec_discard();
                    return vec![Effect::Bell];
                }
                let Some(change) = self.last_change.clone() else {
                    return vec![Effect::Bell];
                };
                return self.exec_repeat(doc, change);
            }
            Key::Char('u') => {
                self.undo(doc);
            }
            Key::Char('i') => {
                self.rec_bit(Bit::Key(key));
                self.start_insert_place(doc, Place::AtCursor);
            }
            Key::Char('a') => {
                self.rec_bit(Bit::Key(key));
                self.start_insert_place(doc, Place::AfterCursor);
            }
            Key::Char('A') => {
                self.rec_bit(Bit::Key(key));
                self.start_insert_place(doc, Place::LineEnd);
            }
            Key::Char('I') => {
                self.rec_bit(Bit::Key(key));
                self.start_insert_place(doc, Place::FirstBlank);
            }
            Key::Char('o') => {
                self.rec_bit(Bit::Key(key));
                self.start_insert_place(doc, Place::LineBelow);
            }
            Key::Char('O') => {
                self.rec_bit(Bit::Key(key));
                self.start_insert_place(doc, Place::LineAbove);
            }
            Key::Char('v') => {
                self.rec_bit(Bit::Key(key));
                self.mode = Mode::Visual;
                self.anchor = self.cursor;
                self.clear_pending();
            }
            Key::Char('V') => {
                self.rec_bit(Bit::Key(key));
                self.mode = Mode::VisualLine;
                self.anchor = self.cursor;
                self.clear_pending();
            }
            Key::CtrlV => {
                self.rec_bit(Bit::Key(key));
                self.mode = Mode::VisualBlock;
                self.anchor = self.cursor;
                self.clear_pending();
            }
            Key::Char(':') => {
                self.mode = Mode::Command;
                self.command.clear();
                self.clear_pending();
                self.rec_discard();
            }
            Key::Char('/') => {
                self.mode = Mode::Command;
                self.command.clear();
                self.command.push('/');
                self.clear_pending();
                self.rec_discard();
            }
            Key::Char('n') => {
                if self.search_last.is_empty() {
                    self.rec_discard();
                    return vec![Effect::Bell];
                }
                let pattern = self.search_last.clone();
                let from = self.char_idx(doc, self.cursor).saturating_add(1);
                self.clear_pending();
                self.rec_discard();
                if !self.jump_to_match(doc, &pattern, from, true) {
                    return vec![Effect::Bell];
                }
            }
            Key::Char('N') => {
                if self.search_last.is_empty() {
                    self.rec_discard();
                    return vec![Effect::Bell];
                }
                let pattern = self.search_last.clone();
                let from = self.char_idx(doc, self.cursor);
                self.clear_pending();
                self.rec_discard();
                if !self.jump_to_match(doc, &pattern, from, false) {
                    return vec![Effect::Bell];
                }
            }
            // `K` looks up the cursor word, like vim with `keywordprg`
            // on an LSP: no buffer change, nothing to repeat or record.
            Key::Char('K') => {
                self.clear_pending();
                self.rec_discard();
                return vec![Effect::Hover];
            }
            // Vim parity: Backspace moves left like `h` in Normal mode.
            Key::Backspace => self.apply_motion(doc, 'h', 1),
            // The Delete key deletes like `x`, including under counts.
            Key::Delete => {
                self.rec_bit(Bit::Key(Key::Char('x')));
                self.delete_chars_forward(doc, count);
                self.rec_commit();
            }
            Key::Home => self.apply_motion(doc, '0', 1),
            Key::End => self.apply_motion(doc, '$', 1),
            Key::Escape => {
                self.clear_pending();
                self.rec_discard();
            }
            _ => {
                self.rec_discard();
                return vec![Effect::Bell];
            }
        }
        Vec::new()
    }

    fn operator_key<B: Buffer>(&mut self, doc: &mut B, op: PendingOp) -> Vec<Effect> {
        let letter = match op {
            PendingOp::Delete => 'd',
            PendingOp::Change => 'c',
            PendingOp::Yank => 'y',
        };
        if self.pending == Some(op) {
            self.rec_bit(Bit::Op(letter));
            let count = self.take_count();
            self.clear_pending();
            match op {
                PendingOp::Delete => {
                    self.delete_lines(doc, count);
                    self.rec_commit();
                }
                PendingOp::Yank => {
                    self.yank_lines(doc, count);
                    // Yank mutates nothing: the last change survives.
                    self.rec_discard();
                }
                PendingOp::Change => {
                    self.begin_change_group(doc);
                    self.change_line(doc, count);
                }
            }
            return Vec::new();
        }
        self.rec_bit(Bit::Op(letter));
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
            self.rec_bit(Bit::Motion(kind));
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
                _ => {
                    self.rec_discard();
                }
            }
            return;
        }
        self.pending_g = false;
        self.count.clear();
        self.rec_discard();
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

    fn apply_motion_to<B: Buffer>(&mut self, doc: &mut B, target: Cursor, motion: char) {
        if let Some(op) = self.pending.take() {
            self.pending_g = false;
            self.count.clear();
            self.rec_bit(Bit::Motion(motion));
            self.apply_operator(doc, op, target, false);
            return;
        }
        self.pending_g = false;
        self.count.clear();
        self.rec_discard();
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
                        col: self
                            .cursor
                            .col
                            .checked_add(count)
                            .map_or(len.saturating_sub(1), |col| col.min(len.saturating_sub(1))),
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
            idx = idx.checked_add(1).unwrap_or(idx);
        }
        for c in doc.line_text(line).chars() {
            if idx >= from && idx < to {
                out.push(c);
            }
            idx = idx.checked_add(1).unwrap_or(idx);
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
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock => {
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
            Mode::Visual | Mode::VisualLine | Mode::VisualBlock => self.visual_key(doc, key),
            Mode::Command => self.command_key(doc, key),
        }
    }

    fn type_text<B: Buffer>(&mut self, doc: &mut B, text: &str) -> Vec<Effect> {
        if self.mode != Mode::Insert || text.is_empty() {
            return Vec::new();
        }
        let at = self.char_idx(doc, self.cursor);
        if !self.replaying {
            self.recording.push(Bit::Type(text.to_string()));
            if at == self.typed_end {
                self.typed_end = self.typed_end.saturating_add(text.chars().count());
            } else {
                // Typing mid-text after a move: the joined recording no
                // longer replays in order.
                self.typed_exact = false;
            }
        }
        doc.insert(at, text);
        let (line, col) = doc.line_col_at(
            at.checked_add(text.chars().count().min(doc.len_chars().saturating_sub(at)))
                .unwrap_or(at),
        );
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

    fn visual_key<B: Buffer>(&mut self, doc: &mut B, key: Key) -> Vec<Effect> {
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

    fn selection_range<B: Buffer>(&self, doc: &B, linewise: bool) -> (usize, usize) {
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

    fn yank_selection<B: Buffer>(&mut self, doc: &mut B, linewise: bool) {
        let (from, to) = self.selection_range(doc, linewise);
        let mut text = range_text(doc, from, to);
        if linewise && text.ends_with('\n') {
            text.pop();
        }
        self.set_register(text, linewise, None);
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
    fn is_substitute(command: &str) -> bool {
        let body = command.strip_prefix('%').unwrap_or(command);
        let mut chars = body.chars();
        matches!((chars.next(), chars.next()), (Some('s'), Some(d)) if !d.is_alphanumeric())
    }

    /// `:s/pat/rep/[g]` on the cursor line, `:%s/pat/rep/[g]` on the
    /// buffer. Patterns are literal (like `/` search); an empty pattern
    /// reuses the last search. `&` in the replacement splices the match,
    /// `\n` a newline. One undo unit; unknown flags and zero matches
    /// bell. Never `.`-repeatable (ex commands aren't).
    fn substitute<B: Buffer>(&mut self, doc: &mut B, command: &str) -> Vec<Effect> {
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
        assert_eq!(doc.text().as_str(), "bar");
    }

    #[test]
    fn change_word_enters_insert() {
        let (mut eng, mut doc) = engine("foo bar");
        keys(&mut eng, &mut doc, "cw");
        assert_eq!(eng.mode(), Mode::Insert);
        assert_eq!(doc.text().as_str(), " bar");
        eng.type_text(&mut doc, "baz");
        assert_eq!(doc.text().as_str(), "baz bar");
    }

    #[test]
    fn line_operators_delete_yank_paste() {
        let (mut eng, mut doc) = engine("one\ntwo\nthree\n");
        keys(&mut eng, &mut doc, "j");
        keys(&mut eng, &mut doc, "dd");
        assert_eq!(doc.text().as_str(), "one\nthree\n");
        keys(&mut eng, &mut doc, "p");
        assert_eq!(doc.text().as_str(), "one\nthree\ntwo\n");
    }

    #[test]
    fn undo_restores_deleted_line() {
        let (mut eng, mut doc) = engine("one\ntwo\n");
        keys(&mut eng, &mut doc, "dd");
        assert_eq!(doc.text().as_str(), "two\n");
        keys(&mut eng, &mut doc, "u");
        assert_eq!(doc.text().as_str(), "one\ntwo\n");
    }

    #[test]
    fn open_line_below_and_escape() {
        let (mut eng, mut doc) = engine("a\nb\n");
        keys(&mut eng, &mut doc, "o");
        assert_eq!(eng.mode(), Mode::Insert);
        eng.type_text(&mut doc, "x");
        eng.press_key(&mut doc, Key::Escape);
        assert_eq!(doc.text().as_str(), "a\nx\nb\n");
        assert_eq!(eng.mode(), Mode::Normal);
    }

    #[test]
    fn visual_yank_and_paste() {
        let (mut eng, mut doc) = engine("hello");
        keys(&mut eng, &mut doc, "vll");
        assert!(eng.selection().is_some());
        keys(&mut eng, &mut doc, "y");
        keys(&mut eng, &mut doc, "$p");
        assert_eq!(doc.text().as_str(), "hellohel");
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
    fn visual_block_delete_rectangle() {
        let (mut eng, mut doc) = engine("abcd\nefgh\nijkl\n");
        eng.press_key(&mut doc, Key::CtrlV);
        assert_eq!(eng.mode(), Mode::VisualBlock);
        keys(&mut eng, &mut doc, "ljj");
        keys(&mut eng, &mut doc, "d");
        assert_eq!(eng.mode(), Mode::Normal);
        assert_eq!(doc.text().as_str(), "cd\ngh\nkl\n");
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 0 });
    }

    #[test]
    fn visual_block_delete_touches_short_rows() {
        // Verified against real Neovim: the rectangle covers column 0 of
        // the short row, so its character goes too.
        let (mut eng, mut doc) = engine("abcd\nx\nijkl\n");
        eng.press_key(&mut doc, Key::CtrlV);
        keys(&mut eng, &mut doc, "lljj");
        keys(&mut eng, &mut doc, "d");
        assert_eq!(doc.text().as_str(), "d\n\nl\n");
    }

    #[test]
    fn visual_block_ctrl_v_toggles_back_to_normal() {
        let (mut eng, mut doc) = engine("ab\ncd\n");
        eng.press_key(&mut doc, Key::CtrlV);
        assert_eq!(eng.mode(), Mode::VisualBlock);
        eng.press_key(&mut doc, Key::CtrlV);
        assert_eq!(eng.mode(), Mode::Normal);
        assert!(eng.selection().is_none());
    }

    #[test]
    fn visual_block_yank_pastes_rectangle() {
        let (mut eng, mut doc) = engine("abcd\nefgh\n");
        eng.press_key(&mut doc, Key::CtrlV);
        keys(&mut eng, &mut doc, "lj");
        keys(&mut eng, &mut doc, "y");
        keys(&mut eng, &mut doc, "ll");
        keys(&mut eng, &mut doc, "p");
        assert_eq!(doc.text().as_str(), "ababcd\nefefgh\n");
    }

    #[test]
    fn visual_block_insert_replays_on_other_rows() {
        let (mut eng, mut doc) = engine("ab\ncd\n");
        eng.press_key(&mut doc, Key::CtrlV);
        keys(&mut eng, &mut doc, "jI");
        eng.type_text(&mut doc, "X");
        eng.press_key(&mut doc, Key::Escape);
        assert_eq!(eng.mode(), Mode::Normal);
        assert_eq!(doc.text().as_str(), "Xab\nXcd\n");
    }

    #[test]
    fn visual_block_append_uses_right_edge() {
        let (mut eng, mut doc) = engine("ab\ncd\n");
        eng.press_key(&mut doc, Key::CtrlV);
        keys(&mut eng, &mut doc, "ljA");
        eng.type_text(&mut doc, "Y");
        eng.press_key(&mut doc, Key::Escape);
        assert_eq!(doc.text().as_str(), "abY\ncdY\n");
    }

    #[test]
    fn visual_block_insert_is_one_undo() {
        let (mut eng, mut doc) = engine("ab\ncd\n");
        eng.press_key(&mut doc, Key::CtrlV);
        keys(&mut eng, &mut doc, "jI");
        eng.type_text(&mut doc, "X");
        eng.press_key(&mut doc, Key::Escape);
        assert_eq!(doc.text().as_str(), "Xab\nXcd\n");
        eng.press_key(&mut doc, Key::Char('u'));
        assert_eq!(doc.text().as_str(), "ab\ncd\n");
    }

    #[test]
    fn visual_o_swaps_selection_ends() {
        let (mut eng, mut doc) = engine("abc");
        keys(&mut eng, &mut doc, "vl");
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 1 });
        keys(&mut eng, &mut doc, "o");
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 0 });
        assert_eq!(eng.copy_text(&doc), "ab");
    }

    #[test]
    fn dot_repeats_insert_at_new_spot() {
        let (mut eng, mut doc) = engine("hi");
        keys(&mut eng, &mut doc, "i");
        eng.type_text(&mut doc, "ab");
        eng.press_key(&mut doc, Key::Escape);
        assert_eq!(doc.text().as_str(), "abhi");
        keys(&mut eng, &mut doc, "$.");
        assert_eq!(doc.text().as_str(), "abhabi");
        assert_eq!(eng.mode(), Mode::Normal);
    }

    #[test]
    fn dot_without_change_bells() {
        let (mut eng, mut doc) = engine("hi");
        assert_eq!(keys(&mut eng, &mut doc, "."), vec![Effect::Bell]);
        assert_eq!(doc.text().as_str(), "hi");
    }

    #[test]
    fn dot_repeats_delete_word() {
        let (mut eng, mut doc) = engine("foo bar\nbaz qux\n");
        keys(&mut eng, &mut doc, "dw");
        assert_eq!(doc.text().as_str(), "bar\nbaz qux\n");
        keys(&mut eng, &mut doc, "j.");
        assert_eq!(doc.text().as_str(), "bar\nqux\n");
    }

    #[test]
    fn dot_repeats_delete_line() {
        let (mut eng, mut doc) = engine("one\ntwo\nthree\n");
        keys(&mut eng, &mut doc, "jdd");
        assert_eq!(doc.text().as_str(), "one\nthree\n");
        keys(&mut eng, &mut doc, ".");
        assert_eq!(doc.text().as_str(), "one\n");
    }

    #[test]
    fn dot_repeats_change_with_typed_text() {
        let (mut eng, mut doc) = engine("foo bar");
        keys(&mut eng, &mut doc, "cw");
        eng.type_text(&mut doc, "baz");
        eng.press_key(&mut doc, Key::Escape);
        assert_eq!(doc.text().as_str(), "baz bar");
        keys(&mut eng, &mut doc, "w.");
        assert_eq!(doc.text().as_str(), "baz baz");
    }

    #[test]
    fn dot_repeats_block_insert() {
        let (mut eng, mut doc) = engine("ab\ncd\nef\n");
        eng.press_key(&mut doc, Key::CtrlV);
        keys(&mut eng, &mut doc, "jI");
        eng.type_text(&mut doc, "X");
        eng.press_key(&mut doc, Key::Escape);
        assert_eq!(doc.text().as_str(), "Xab\nXcd\nef\n");
        keys(&mut eng, &mut doc, "j.");
        assert_eq!(doc.text().as_str(), "Xab\nXXcd\nXef\n");
    }

    #[test]
    fn yank_does_not_disturb_dot() {
        let (mut eng, mut doc) = engine("foo bar baz");
        keys(&mut eng, &mut doc, "dw");
        assert_eq!(doc.text().as_str(), "bar baz");
        keys(&mut eng, &mut doc, "yy");
        keys(&mut eng, &mut doc, "w.");
        assert_eq!(doc.text().as_str(), "bar ");
    }

    #[test]
    fn change_word_undoes_as_one_step() {
        let (mut eng, mut doc) = engine("foo bar");
        keys(&mut eng, &mut doc, "cw");
        eng.type_text(&mut doc, "baz");
        eng.press_key(&mut doc, Key::Escape);
        assert_eq!(doc.text().as_str(), "baz bar");
        keys(&mut eng, &mut doc, "u");
        assert_eq!(doc.text().as_str(), "foo bar");
        eng.redo(&mut doc);
        assert_eq!(doc.text().as_str(), "baz bar");
    }

    #[test]
    fn open_line_typing_undoes_as_one_step() {
        let (mut eng, mut doc) = engine("a\n");
        keys(&mut eng, &mut doc, "o");
        eng.type_text(&mut doc, "x");
        eng.press_key(&mut doc, Key::Escape);
        assert_eq!(doc.text().as_str(), "a\nx\n");
        keys(&mut eng, &mut doc, "u");
        assert_eq!(doc.text().as_str(), "a\n");
    }

    #[test]
    fn substitute_first_match_on_line() {
        let (mut eng, mut doc) = engine("foo foo\n");
        keys(&mut eng, &mut doc, ":s/foo/bar");
        assert_eq!(eng.press_key(&mut doc, Key::Enter), Vec::new());
        assert_eq!(doc.text().as_str(), "bar foo\n");
        assert_eq!(eng.mode(), Mode::Normal);
    }

    #[test]
    fn substitute_global_and_percent_range() {
        let (mut eng, mut doc) = engine("foo foo\nfoo\n");
        keys(&mut eng, &mut doc, ":%s/foo/bar/g");
        assert_eq!(eng.press_key(&mut doc, Key::Enter), Vec::new());
        assert_eq!(doc.text().as_str(), "bar bar\nbar\n");
        // One change, one undo step.
        keys(&mut eng, &mut doc, "u");
        assert_eq!(doc.text().as_str(), "foo foo\nfoo\n");
    }

    #[test]
    fn substitute_ampersand_splices_match() {
        let (mut eng, mut doc) = engine("foo\n");
        keys(&mut eng, &mut doc, ":s/foo/<&>/");
        eng.press_key(&mut doc, Key::Enter);
        assert_eq!(doc.text().as_str(), "<foo>\n");
    }

    #[test]
    fn substitute_escaped_delimiter_and_alt_delimiter() {
        let (mut eng, mut doc) = engine("a/b\n");
        keys(&mut eng, &mut doc, ":s/\\//X/");
        eng.press_key(&mut doc, Key::Enter);
        assert_eq!(doc.text().as_str(), "aXb\n");
        let (mut eng, mut doc) = engine("a\n");
        keys(&mut eng, &mut doc, ":s#a#b#");
        eng.press_key(&mut doc, Key::Enter);
        assert_eq!(doc.text().as_str(), "b\n");
    }

    #[test]
    fn substitute_empty_pattern_reuses_last_search() {
        let (mut eng, mut doc) = engine("foo foo\n");
        keys(&mut eng, &mut doc, "/foo");
        eng.press_key(&mut doc, Key::Enter);
        keys(&mut eng, &mut doc, ":s//bar/");
        eng.press_key(&mut doc, Key::Enter);
        assert_eq!(doc.text().as_str(), "bar foo\n");
    }

    #[test]
    fn substitute_no_match_and_bad_flags_bell() {
        let (mut eng, mut doc) = engine("foo\n");
        keys(&mut eng, &mut doc, ":s/zzz/q");
        assert_eq!(eng.press_key(&mut doc, Key::Enter), vec![Effect::Bell]);
        assert_eq!(doc.text().as_str(), "foo\n");
        keys(&mut eng, &mut doc, ":s/foo/bar/z");
        assert_eq!(eng.press_key(&mut doc, Key::Enter), vec![Effect::Bell]);
        assert_eq!(doc.text().as_str(), "foo\n");
    }

    #[test]
    fn ex_split_vsplit_tabnew_effects() {
        fn ex(command: &str) -> Vec<Effect> {
            let (mut eng, mut doc) = engine("x");
            keys(&mut eng, &mut doc, &format!(":{command}"));
            eng.press_key(&mut doc, Key::Enter)
        }
        assert_eq!(ex("split"), vec![Effect::Split]);
        assert_eq!(ex("sp"), vec![Effect::Split]);
        assert_eq!(ex("vsplit"), vec![Effect::Vsplit]);
        assert_eq!(ex("vs"), vec![Effect::Vsplit]);
        assert_eq!(ex("tabnew"), vec![Effect::TabNew]);
    }

    #[test]
    fn yank_stages_clipboard_and_take_clears() {
        let (mut eng, mut doc) = engine("foo\nbar\n");
        assert_eq!(eng.take_clipboard(), None);
        keys(&mut eng, &mut doc, "yy");
        assert_eq!(eng.take_clipboard().as_deref(), Some("foo"));
        assert_eq!(eng.take_clipboard(), None);
    }

    #[test]
    fn delete_lines_stage_clipboard_too() {
        // `clipboard=unnamed`: register writes sync, deletes included.
        let (mut eng, mut doc) = engine("foo\nbar\n");
        keys(&mut eng, &mut doc, "dd");
        assert_eq!(eng.take_clipboard().as_deref(), Some("foo"));
    }

    #[test]
    fn normal_backspace_moves_and_delete_deletes() {
        let (mut eng, mut doc) = engine("ab");
        keys(&mut eng, &mut doc, "l");
        eng.press_key(&mut doc, Key::Backspace);
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 0 });
        eng.press_key(&mut doc, Key::Delete);
        assert_eq!(doc.text().as_str(), "b");
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
        assert_eq!(doc.text().as_str(), "hi!");
        eng.press_key(&mut doc, Key::Escape);
        keys(&mut eng, &mut doc, "x");
        assert_eq!(doc.text().as_str(), "hi");
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
        assert_eq!(doc.text().as_str(), "b\n");
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 0 });
    }

    #[test]
    fn cut_selection_collapses_to_normal() {
        let (mut eng, mut doc) = engine("ab\ncd\n");
        keys(&mut eng, &mut doc, "vl");
        let removed = eng.cut(&mut doc);
        assert_eq!(removed, "ab");
        assert_eq!(doc.text().as_str(), "\ncd\n");
        assert_eq!(eng.mode(), Mode::Normal);
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 0 });
    }

    #[test]
    fn paste_text_inserts_at_cursor_in_normal() {
        // Insert-at-cursor (not vim `p` after-cursor) so a copied line
        // pastes back as a line instead of splitting one.
        let (mut eng, mut doc) = engine("ac");
        eng.paste_text(&mut doc, "b");
        assert_eq!(doc.text().as_str(), "bac");
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
        assert_eq!(doc.text().as_str(), "");
        eng.redo(&mut doc);
        assert_eq!(doc.text().as_str(), "x");
    }

    #[test]
    fn vimrc_set_applies_ignorecase_and_strips_trailing_comments() {
        assert!(parse_vimrc("set ignorecase\n").ignorecase);
        assert!(parse_vimrc("set ignorecase \" trailing comment\n").ignorecase);
        assert!(parse_vimrc("set ignorecase\t\" tab comment\n").ignorecase);
        assert!(!parse_vimrc("\" full-line comment\nset noignorecase\n").ignorecase);
        assert!(!parse_vimrc("set ignorecase\"glued quote is not a comment\n").ignorecase);
    }

    #[test]
    fn backspace_after_a_multiline_paste_deletes_across_the_newlines() {
        let (mut eng, mut doc) = engine("");
        eng.press_key(&mut doc, Key::Char('i'));
        eng.paste_text(&mut doc, "hello\nworld");
        assert_eq!(doc.text().as_str(), "hello\nworld");
        for _ in 0..11 {
            eng.press_key(&mut doc, Key::Backspace);
        }
        assert_eq!(doc.text().as_str(), "");
    }

    #[test]
    fn enter_then_typing_continues_on_the_next_line() {
        let (mut eng, mut doc) = engine("");
        eng.press_key(&mut doc, Key::Char('i'));
        eng.type_text(&mut doc, "ab");
        eng.press_key(&mut doc, Key::Enter);
        eng.type_text(&mut doc, "c");
        assert_eq!(doc.text().as_str(), "ab\nc");
        assert_eq!(eng.cursor(), Cursor { line: 1, col: 1 });
    }

    #[test]
    fn esc_from_just_past_a_newline_lands_on_the_previous_line_end() {
        let (mut eng, mut doc) = engine("");
        eng.press_key(&mut doc, Key::Char('i'));
        eng.paste_text(&mut doc, "hello\nworld");
        for _ in 0..5 {
            eng.press_key(&mut doc, Key::Backspace);
        }
        assert_eq!(doc.text().as_str(), "hello\n");
        assert_eq!(eng.cursor(), Cursor { line: 1, col: 0 });
        eng.press_key(&mut doc, Key::Escape);
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 4 });
    }

    #[test]
    fn paste_ending_with_a_newline_leaves_the_cursor_on_the_last_line() {
        let (mut eng, mut doc) = engine("");
        eng.paste_text(&mut doc, "hello\nworld\n");
        assert_eq!(doc.text().as_str(), "hello\nworld\n");
        assert_eq!(eng.cursor(), Cursor { line: 1, col: 4 });
    }

    #[test]
    fn capital_k_requests_hover_without_moving() {
        let (mut eng, mut doc) = engine("fn main() {}");
        let before = eng.cursor();
        let out = keys(&mut eng, &mut doc, "K");
        assert_eq!(out, vec![Effect::Hover]);
        assert_eq!(eng.cursor(), before);
        assert_eq!(doc.text().as_str(), "fn main() {}");
    }

    #[test]
    fn gd_requests_goto_and_leaves_dd_alone() {
        let (mut eng, mut doc) = engine("one\ntwo\n");
        let out = keys(&mut eng, &mut doc, "gd");
        assert_eq!(out, vec![Effect::GotoDefinition]);
        assert_eq!(doc.text().as_str(), "one\ntwo\n");
        // Bare `dd` still deletes a line, and `gg` still goes to the top.
        let out = keys(&mut eng, &mut doc, "dd");
        assert!(!out.contains(&Effect::GotoDefinition));
        assert_eq!(doc.text().as_str(), "two\n");
        let (mut eng, mut doc) = engine("one\ntwo\n");
        keys(&mut eng, &mut doc, "j");
        keys(&mut eng, &mut doc, "gg");
        assert_eq!(eng.cursor(), Cursor { line: 0, col: 0 });
    }

    #[test]
    fn g_then_other_key_bells_and_cancels() {
        let (mut eng, mut doc) = engine("one\ntwo\n");
        let out = keys(&mut eng, &mut doc, "gx");
        assert_eq!(out, vec![Effect::Bell]);
        // The cancelled prefix leaves no residue: `d` deletes again.
        let (mut eng, mut doc) = engine("one\ntwo\n");
        keys(&mut eng, &mut doc, "g");
        keys(&mut eng, &mut doc, "x");
        let out = keys(&mut eng, &mut doc, "dd");
        assert!(!out.contains(&Effect::GotoDefinition));
        assert_eq!(doc.text().as_str(), "two\n");
    }
}
