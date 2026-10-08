use super::mode::{Bit, BlockPending, Cursor, Mode, PendingOp, Register, Repeat};
pub struct VimEngine {
    pub(crate) mode: Mode,
    pub(crate) cursor: Cursor,
    pub(crate) anchor: Cursor,
    pub(crate) preferred_col: usize,
    pub(crate) count: String,
    pub(crate) pending: Option<PendingOp>,
    pub(crate) pending_g: bool,
    pub(crate) command: String,
    pub(crate) register: Register,
    pub(crate) search_last: String,
    pub(crate) scroll_request: bool,
    /// An explicit undo unit is open (an insert session, possibly with its
    /// preceding delete for `c`/`cc`): one vim change, one undo step.
    pub(crate) insert_group: bool,
    /// Live block insert awaiting its replay on Escape.
    pub(crate) block_pending: Option<BlockPending>,
    /// Yanked text awaiting host clipboard sync (`clipboard=unnamed`):
    /// every unnamed-register write stages here; the view forwards it to
    /// the system clipboard and clears it. Never affects editing.
    pub(crate) clipboard: Option<String>,
    /// Bits of the change in progress; parsed into [`Repeat`] when it
    /// completes. Skipped while replaying.
    pub(crate) recording: Vec<Bit>,
    pub(crate) replaying: bool,
    pub(crate) last_change: Option<Repeat>,
    /// The recorded typed text is exactly the session's net insertion.
    /// Cleared by unattributable edits (typing mid-text after a cursor
    /// move, forward delete, backspacing past session text, IDE cut):
    /// with it false, no insert repeat or block replay is built, so `.`
    /// bells instead of replaying wrong text.
    pub(crate) typed_exact: bool,
    /// Session insertion point range: typing at `typed_end` extends the
    /// recording exactly; anywhere else ends attribution. Both are char
    /// indices captured when the session opens.
    pub(crate) typed_start: usize,
    pub(crate) typed_end: usize,
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

pub(crate) fn apply_set_str(opts: &mut VimrcOptions, option: &str) -> bool {
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

    pub(super) fn take_count(&mut self) -> usize {
        if self.count.is_empty() {
            1
        } else {
            self.count.parse().unwrap_or(1).clamp(1, 1_000_000)
        }
    }

    pub(super) fn clear_pending(&mut self) {
        self.pending = None;
        self.pending_g = false;
        self.count.clear();
    }
}
