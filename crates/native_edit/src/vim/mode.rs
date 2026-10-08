use super::super::doc::Buffer;
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
pub(crate) enum PendingOp {
    Delete,
    Change,
    Yank,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Register {
    pub(super) text: String,
    pub(super) linewise: bool,
    /// Set by blockwise yank: column count of the yanked rectangle, so a
    /// later `p` pastes a block instead of sequential lines.
    pub(super) block_width: Option<usize>,
}

/// One recorded input bit for `.` repeat: the change replays by parsing
/// these, never by re-reading the buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Bit {
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
pub(crate) enum Repeat {
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
pub(crate) enum Place {
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
pub(crate) struct BlockPending {
    pub(super) top: usize,
    pub(super) bottom: usize,
    pub(super) left: usize,
    pub(super) right: usize,
    pub(super) append: bool,
    pub(super) skip_first: bool,
}
