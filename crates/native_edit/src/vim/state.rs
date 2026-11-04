use super::input::mapping::Mappings;
use super::mode::{
    Bit, BlockPending, Cursor, Input, Mode, PendingOp, Place, Prefix, Register, Repeat,
};
use crate::lsp::FormatOptions;
use std::collections::HashMap;
use std::sync::Arc;

/// What the screen shows, as the view last reported it: first and last
/// fully visible buffer lines (`H`/`M`/`L`, Ctrl-D/U/F/B).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Viewport {
    pub(crate) top: usize,
    pub(crate) bottom: usize,
}

/// A scroll the view should apply after keys (`zz`, Ctrl-E, Ctrl-D, …).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewScroll {
    /// Put the cursor row at the top (`zt`), center (`zz`) or bottom (`zb`).
    Top,
    Center,
    Bottom,
    /// Scroll the view by this many lines (positive is down).
    Lines(isize),
}

/// Vim state beyond the core engine: counts, pending prefixes, registers,
/// marks, jumps, macros, `.` keys and the view handshake.
#[derive(Default)]
pub(crate) struct Extra {
    /// Count typed before an operator (`2` of `2d3w`): multiplies the
    /// motion count, never concatenates with it.
    pub(crate) op_count: Option<usize>,
    pub(crate) prefix: Option<Prefix>,
    /// Register named by `"x` for the next command.
    pub(crate) reg_sel: Option<char>,
    /// Named (`a`-`z`), yank (`0`) and small-delete (`-`) registers.
    pub(crate) registers: HashMap<char, Register>,
    /// Last `f`/`t`/`F`/`T` and its char, for `;` and `,`.
    pub(crate) last_find: Option<(char, char)>,
    /// The last search went backward (`?`, `#`): `n` keeps that way.
    pub(crate) search_backward: bool,
    pub(crate) marks: HashMap<char, Cursor>,
    pub(crate) jumps: Vec<Cursor>,
    pub(crate) jump_idx: usize,
    pub(crate) recording_macro: Option<(char, Vec<Input>)>,
    pub(crate) macros: HashMap<char, Vec<Input>>,
    pub(crate) last_macro: Option<char>,
    /// Depth of macro / mapping replay; typed-input capture stops inside.
    pub(crate) replay_depth: usize,
    pub(crate) last_ex: Option<String>,
    /// Inputs of the command in progress, for [`Repeat::Keys`].
    pub(crate) raw: Vec<Input>,
    /// The command in progress repeats key by key.
    pub(crate) keys_change: bool,
    /// Keys held while they may still start a longer mapping.
    pub(crate) map_pending: Vec<Input>,
    pub(crate) viewport: Viewport,
    pub(crate) view_scroll: Option<ViewScroll>,
    /// Last visual selection (`gv`, `'<`, `'>`).
    pub(crate) last_visual: Option<(Mode, Cursor, Cursor)>,
    /// Visual block extends to every line end (`$` in block mode).
    pub(crate) block_eol: bool,
    /// Count for `3ihi<Esc>`: the typed text repeats on Escape.
    pub(crate) insert_repeat: Option<(usize, Place)>,
    /// `R` mode: chars overwritten so far (`None` = appended), for
    /// Backspace to restore.
    pub(crate) replaced: Option<Vec<Option<char>>>,
    /// Insert Ctrl-O: one Normal command, then back to Insert. Holds the
    /// insert cursor, where Normal clamped it, and whether the command began.
    pub(crate) oneshot: Option<(Cursor, Cursor, bool)>,
}
pub struct VimEngine {
    pub(crate) mode: Mode,
    pub(crate) cursor: Cursor,
    pub(crate) anchor: Cursor,
    pub(crate) preferred_col: usize,
    pub(crate) count: String,
    pub(crate) pending: Option<PendingOp>,
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
    /// System clipboard text as last known (written by a yank or read by
    /// the host): `p` adopts the clipboard only when it differs, so a
    /// newer `d`/`c`/`x` register is not lost to older clipboard text.
    pub(crate) clipboard_seen: Option<String>,
    /// A delete wrote the register without the clipboard; the host should
    /// report the current clipboard text ([`VimEngine::note_clipboard`]).
    pub(crate) clipboard_snapshot: bool,
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
    /// Options from `~/.vimrc` and later `:set` commands. View-only
    /// switches (`wrap`) stay here so every pane of one buffer agrees.
    pub(crate) options: VimrcOptions,
    /// Vim off: IDE caret and selection semantics (see `vim/ide.rs`).
    pub(crate) modeless: bool,
    /// Selections extend-selection grew from, innermost last, for shrink.
    pub(crate) extend_stack: Vec<(usize, usize)>,
    /// Modeless multiple carets besides the primary one (`IntelliJ`
    /// Alt+Shift+click, Add Selection for Next Occurrence).
    pub(crate) extra_carets: Vec<super::carets::Caret>,
    pub(crate) x: Extra,
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
            command: String::new(),
            register: Register::default(),
            search_last: String::new(),
            scroll_request: false,
            insert_group: false,
            block_pending: None,
            clipboard: None,
            clipboard_seen: None,
            clipboard_snapshot: false,
            recording: Vec::new(),
            replaying: false,
            last_change: None,
            typed_exact: true,
            typed_start: 0,
            typed_end: 0,
            options: VimrcOptions::default(),
            modeless: false,
            extend_stack: Vec::new(),
            extra_carets: Vec::new(),
            x: Extra::default(),
        }
    }
}

/// Supported `~/.vimrc` options (the rest of the file is ignored).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VimrcOptions {
    pub ignorecase: bool,
    /// `set smartcase`: an uppercase letter in a pattern makes it
    /// case-sensitive (with `ignorecase`).
    pub smartcase: bool,
    /// `map`/`noremap` family from the vimrc, shared by every engine.
    pub maps: Option<Arc<Mappings>>,
    /// `set wrap` / `set nowrap`; `None` keeps the host default (source
    /// wraps only for Markdown).
    pub wrap: Option<bool>,
    /// `shiftwidth` / `tabstop` / `expandtab`: the indentation for files
    /// with none to detect ([`VimrcOptions::format_options`]).
    pub shiftwidth: Option<u32>,
    pub tabstop: Option<u32>,
    pub expandtab: Option<bool>,
}

impl VimrcOptions {
    /// Indentation from `shiftwidth` (else `tabstop`) and `expandtab`,
    /// defaulting to four spaces.
    #[must_use]
    pub fn format_options(&self) -> FormatOptions {
        let default = FormatOptions::default();
        FormatOptions {
            tab_size: self.shiftwidth.or(self.tabstop).unwrap_or(default.tab_size),
            insert_spaces: self.expandtab.unwrap_or(default.insert_spaces),
        }
    }
}

pub(crate) fn apply_set_str(opts: &mut VimrcOptions, option: &str) -> bool {
    if let Some((name, value)) = option.split_once('=') {
        let Some(n) = value.parse::<u32>().ok().filter(|n| *n > 0) else {
            return false;
        };
        match name {
            "sw" | "shiftwidth" => opts.shiftwidth = Some(n),
            "ts" | "tabstop" => opts.tabstop = Some(n),
            _ => return false,
        }
        return true;
    }
    match option {
        "et" | "expandtab" => opts.expandtab = Some(true),
        "noet" | "noexpandtab" => opts.expandtab = Some(false),
        "ic" | "ignorecase" => opts.ignorecase = true,
        "noic" | "noignorecase" => opts.ignorecase = false,
        "ic!" | "ignorecase!" => opts.ignorecase = !opts.ignorecase,
        "scs" | "smartcase" => opts.smartcase = true,
        "noscs" | "nosmartcase" => opts.smartcase = false,
        // Accepted for vimrc compatibility; the view has no search
        // highlighting to switch.
        "hls" | "hlsearch" | "nohls" | "nohlsearch" | "is" | "incsearch" | "nois"
        | "noincsearch" => {}
        "wrap" => opts.wrap = Some(true),
        "nowrap" => opts.wrap = Some(false),
        "wrap!" => opts.wrap = Some(!opts.wrap.unwrap_or(false)),
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
    let mut maps = Mappings::default();
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
        if maps.parse_line(line) {
            continue;
        }
        let args = line
            .strip_prefix("set ")
            .or_else(|| line.strip_prefix("setlocal "));
        let Some(args) = args else { continue };
        for arg in args.split_whitespace() {
            apply_set_str(&mut opts, arg);
        }
    }
    if !maps.is_empty() {
        opts.maps = Some(Arc::new(maps));
    }
    opts
}

impl VimEngine {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adopt `~/.vimrc` options when the buffer's engine is created.
    /// Later `:set` commands change the same fields.
    pub fn apply_vimrc(&mut self, options: VimrcOptions) {
        self.options = options;
    }

    /// `Some(true)` wraps every source line, `Some(false)` never wraps,
    /// `None` leaves the host default (Markdown wraps, code does not).
    #[must_use]
    pub fn wrap(&self) -> Option<bool> {
        self.options.wrap
    }

    #[must_use]
    pub fn ignorecase(&self) -> bool {
        self.options.ignorecase
    }

    /// Apply `:set` / `:setlocal` arguments. True when at least one name
    /// was recognised; unknown names bell, like the rest of ex.
    pub(super) fn apply_set_args(&mut self, args: &str) -> bool {
        let mut known = false;
        for arg in args.split_whitespace() {
            known |= apply_set_str(&mut self.options, arg);
        }
        known
    }

    /// The effective count: the count typed before an operator times the
    /// one typed after it (`2d3w` is 6), 1 when neither was typed.
    pub(super) fn take_count(&mut self) -> usize {
        let typed: usize = if self.count.is_empty() {
            1
        } else {
            self.count.parse().unwrap_or(1).clamp(1, 1_000_000)
        };
        typed
            .saturating_mul(self.x.op_count.unwrap_or(1))
            .clamp(1, 1_000_000)
    }

    /// A count was typed for the command in progress (`5G` vs `G`).
    pub(super) fn has_count(&self) -> bool {
        !self.count.is_empty() || self.x.op_count.is_some()
    }

    /// No operator or `g` prefix is waiting for more keys (a count may be).
    #[must_use]
    pub(crate) fn is_idle(&self) -> bool {
        self.pending.is_none() && self.x.prefix.is_none()
    }

    /// Normal mode with nothing typed yet: the next key starts a command.
    pub(super) fn at_command_start(&self) -> bool {
        self.mode == Mode::Normal
            && self.is_idle()
            && self.count.is_empty()
            && self.x.reg_sel.is_none()
    }

    pub(super) fn clear_pending(&mut self) {
        self.pending = None;
        self.count.clear();
        self.x.op_count = None;
        self.x.prefix = None;
    }
}
