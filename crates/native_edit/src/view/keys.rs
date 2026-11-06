use super::super::highlight::Language;
use super::super::vim::{Effect, IdeMotion, Key as VKey, ModalEngine, Mode};
use egui;
use egui::Color32;
pub fn egui_key(key: egui::Key, modifiers: egui::Modifiers) -> Option<VKey> {
    if modifiers.command || modifiers.ctrl {
        return None;
    }
    Some(match key {
        egui::Key::ArrowLeft => VKey::Left,
        egui::Key::ArrowRight => VKey::Right,
        egui::Key::ArrowUp => VKey::Up,
        egui::Key::ArrowDown => VKey::Down,
        egui::Key::Home => VKey::Home,
        egui::Key::End => VKey::End,
        egui::Key::Enter => VKey::Enter,
        egui::Key::Backspace => VKey::Backspace,
        egui::Key::Delete => VKey::Delete,
        egui::Key::Escape => VKey::Escape,
        egui::Key::Tab if modifiers.shift => VKey::BackTab,
        egui::Key::Tab => VKey::Tab,
        // Punctuation arrives as dedicated physical keys; pass the character
        // through so current and future engine commands stay reachable.
        egui::Key::Colon => VKey::Char(':'),
        egui::Key::Semicolon => VKey::Char(';'),
        egui::Key::Slash => VKey::Char('/'),
        egui::Key::Period => VKey::Char('.'),
        egui::Key::Comma => VKey::Char(','),
        egui::Key::Minus => VKey::Char('-'),
        egui::Key::Equals => VKey::Char('='),
        egui::Key::Space => VKey::Char(' '),
        egui::Key::Quote => VKey::Char('\''),
        egui::Key::OpenBracket => VKey::Char('['),
        egui::Key::CloseBracket => VKey::Char(']'),
        egui::Key::Backslash => VKey::Char('\\'),
        egui::Key::Backtick => VKey::Char('`'),
        _ => return key_name_char(key, modifiers),
    })
}

fn key_name_char(key: egui::Key, modifiers: egui::Modifiers) -> Option<VKey> {
    let name = key.name();
    let mut chars = name.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphabetic() => Some(VKey::Char(if modifiers.shift {
            c.to_ascii_uppercase()
        } else {
            c.to_ascii_lowercase()
        })),
        // Shifted digits are their US symbols (`1`→`!`, …, `0`→`)`); a bare
        // digit is a count. Positional like the rest of this table.
        (Some(c), None) if c.is_ascii_digit() => Some(VKey::Char(if modifiers.shift {
            const SYMBOLS: &[u8; 10] = b")!@#$%^&*(";
            let index = (c as u8).checked_sub(b'0').map(usize::from).unwrap_or(0);
            char::from(SYMBOLS.get(index).copied().unwrap_or(b')'))
        } else {
            c
        })),
        _ => {
            // Backends that report the symbol itself (`!` instead of shifted
            // `1`) expose it via `symbol_or_name`; pass printable punctuation
            // through so `:q!` stays typeable everywhere.
            let symbol = key.symbol_or_name();
            let mut chars = symbol.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_punctuation() => Some(VKey::Char(c)),
                _ => None,
            }
        }
    }
}

/// A Ctrl chord vim owns in its current mode, as an engine key: Ctrl-V /
/// Ctrl-Q block select and Ctrl-6 alternate file (Normal and Visual),
/// redo, scrolling, jumps, increment and Ctrl-W panes (Normal), scrolling (Visual) and
/// Ctrl-W / Ctrl-U / Ctrl-O (Insert). These win over IDE chords such as
/// Ctrl-A select all, so Linux (where Ctrl is Command) matches macOS.
#[must_use]
pub fn vim_ctrl_key(
    key: egui::Key,
    modifiers: egui::Modifiers,
    vim: bool,
    mode: Mode,
) -> Option<VKey> {
    if !(vim && modifiers.ctrl && !modifiers.mac_cmd && !modifiers.alt) {
        return None;
    }
    let block = ctrl_v_is_block(vim, mode);
    match key {
        egui::Key::V | egui::Key::Q if block && !modifiers.shift => return Some(VKey::CtrlV),
        // Ctrl-6 / Ctrl-^ (Shift may be held): the alternate file.
        egui::Key::Num6 if block => return Some(VKey::CtrlCaret),
        _ => {}
    }
    if modifiers.shift {
        return None;
    }
    let mut name = key.name().chars();
    let letter = match (name.next(), name.next()) {
        (Some(c), None) if c.is_ascii_alphabetic() => c.to_ascii_lowercase(),
        _ => return None,
    };
    let owned = match mode {
        Mode::Normal => "rdufbeyoiaxw",
        Mode::Visual | Mode::VisualLine | Mode::VisualBlock => "dufbey",
        Mode::Insert => "wuo",
        Mode::Command => "",
    };
    owned.contains(letter).then_some(VKey::Ctrl(letter))
}

/// True when a key press also arrives as a Text event (a printable char
/// without Ctrl or Command): the text carries the char, in the user's
/// keyboard layout, so the key event itself is skipped.
#[must_use]
pub fn arrives_as_text(key: egui::Key, modifiers: egui::Modifiers) -> bool {
    !modifiers.ctrl && !modifiers.command && matches!(egui_key(key, modifiers), Some(VKey::Char(_)))
}

/// The events a real keyboard sends for one press: the key, then its
/// text when the view takes the char from Text ([`arrives_as_text`]).
/// For test fixtures that synthesize input.
#[must_use]
pub fn keyboard_events(key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
    let mut events = vec![egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }];
    if arrives_as_text(key, modifiers)
        && let Some(VKey::Char(c)) = egui_key(key, modifiers)
    {
        events.push(egui::Event::Text(c.to_string()));
    }
    events
}

/// IntelliJ-style shortcut the file view owns in every mode (vim included):
/// the editor keeps copy/cut/paste/select/undo keys instead of yielding them.
/// Ctrl-V and Ctrl-H are conditional (see [`ctrl_v_is_block`] and
/// [`ctrl_h_is_motion`]): vim owns them in its modes, the IDE elsewhere.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IdeAction {
    Copy,
    Cut,
    SelectAll,
    Undo,
    Redo,
    /// Consume only; the pasted text arrives as a separate Paste event.
    PasteKey,
    Save,
    Find,
    Replace,
    /// Language-server hover for the cursor (Ctrl+K; vim `K`).
    Hover,
}

/// Resolve a Command/Ctrl-held key to an IDE action. Plain keys and the Vim
/// engine's own keys return `None` and keep their existing routing.
/// (`command` mirrors Ctrl on Linux/Windows, so only Alt is excluded.)
#[must_use]
pub fn ide_shortcut(key: egui::Key, modifiers: egui::Modifiers) -> Option<IdeAction> {
    if !(modifiers.command && !modifiers.alt) {
        return None;
    }
    Some(match key {
        egui::Key::C => IdeAction::Copy,
        egui::Key::X => IdeAction::Cut,
        egui::Key::A => IdeAction::SelectAll,
        egui::Key::Z if modifiers.shift => IdeAction::Redo,
        egui::Key::Z => IdeAction::Undo,
        egui::Key::V => IdeAction::PasteKey,
        egui::Key::S => IdeAction::Save,
        // Cmd/Ctrl+Shift+F and +R are Find and Replace in Files (see
        // [`ide_navigation_shortcut`]).
        egui::Key::F | egui::Key::R if modifiers.shift => return None,
        egui::Key::F => IdeAction::Find,
        egui::Key::H | egui::Key::R => IdeAction::Replace,
        egui::Key::K => IdeAction::Hover,
        _ => return None,
    })
}

/// Project navigation chords (`IntelliJ` keymap) that queue an effect in
/// every mode, vim included: Cmd/Ctrl+Shift+F Find in Files,
/// Cmd/Ctrl+Shift+R Replace in Files, and with a language `server`,
/// Cmd/Ctrl+Alt+B Go to Implementation and Cmd/Ctrl+Shift+B Go to Type
/// Declaration.
#[must_use]
pub fn ide_navigation_shortcut(
    key: egui::Key,
    modifiers: egui::Modifiers,
    server: bool,
) -> Option<Effect> {
    if !modifiers.command {
        return None;
    }
    match key {
        egui::Key::F if modifiers.shift && !modifiers.alt => Some(Effect::FindInFiles),
        egui::Key::R if modifiers.shift && !modifiers.alt => Some(Effect::ReplaceInFiles),
        egui::Key::B if server && modifiers.alt && !modifiers.shift => {
            Some(Effect::GotoImplementation)
        }
        egui::Key::B if server && modifiers.shift && !modifiers.alt => {
            Some(Effect::GotoTypeDefinition)
        }
        _ => None,
    }
}

/// A `JetBrains` keymap chord in a focused editor (Settings → Editor →
/// Keymap). These shadow app chords such as Cmd+B (left sidebar).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JetBrainsKey {
    Effect(Effect),
    /// Go to Line: Cmd+L on macOS, Ctrl+G elsewhere.
    GotoLine,
    /// F2 / Shift+F2: next / previous problem.
    Problem {
        backward: bool,
    },
}

/// Resolve a `JetBrains` keymap chord. `server` gates language-server
/// actions; `mac` picks the macOS chord for Go to Line.
#[must_use]
pub fn jetbrains_shortcut(
    key: egui::Key,
    modifiers: egui::Modifiers,
    server: bool,
    mac: bool,
) -> Option<JetBrainsKey> {
    let plain_command = modifiers.command && !modifiers.alt && !modifiers.shift;
    let no_command = !modifiers.command && !modifiers.ctrl && !modifiers.alt;
    match key {
        egui::Key::B if plain_command && server => {
            Some(JetBrainsKey::Effect(Effect::GotoDefinition))
        }
        egui::Key::F6 if no_command && modifiers.shift && server => {
            Some(JetBrainsKey::Effect(Effect::RenameSymbol))
        }
        egui::Key::L if mac && plain_command => Some(JetBrainsKey::GotoLine),
        egui::Key::G if !mac && plain_command => Some(JetBrainsKey::GotoLine),
        egui::Key::F2 if no_command => Some(JetBrainsKey::Problem {
            backward: modifiers.shift,
        }),
        _ => None,
    }
}

/// IDE caret motion or line edit (`IntelliJ` keymap). `Move` carries the
/// motion and whether Shift extends the selection.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IdeEdit {
    Move(IdeMotion, bool),
    DeleteWordBack,
    DeleteLines,
    Duplicate,
    MoveLines {
        up: bool,
    },
    ExtendSelection,
    ShrinkSelection,
    /// Fold the region at the caret (Cmd/Ctrl+-); `all` with Shift.
    Fold {
        all: bool,
    },
    /// Unfold at the caret (Cmd/Ctrl+=); `all` with Shift.
    Unfold {
        all: bool,
    },
    ToggleComment,
    JoinLines,
    /// Start New Line (Shift+Enter); `above`: Start New Line Before
    /// Current (Cmd+Alt+Enter, Ctrl+Alt+Enter off macOS).
    StartNewLine {
        above: bool,
    },
}

/// Inputs for [`ide_edit_key`].
pub struct IdeEditKeyInput {
    pub key: egui::Key,
    pub modifiers: egui::Modifiers,
    pub mac: bool,
    pub page_rows: usize,
}

/// Resolve an IDE navigation or line-edit chord. `mac` picks the macOS
/// keymap (Option for words, Command for line and document edges); the
/// others use Ctrl for words and Home/End with Ctrl for document edges.
/// `page_rows` is the visible row count for Page Up/Down.
#[must_use]
pub fn ide_edit_key(input: IdeEditKeyInput) -> Option<IdeEdit> {
    let IdeEditKeyInput {
        key,
        modifiers,
        mac,
        page_rows,
    } = input;
    let select = modifiers.shift;
    let word = if mac {
        modifiers.alt && !modifiers.command && !modifiers.ctrl
    } else {
        modifiers.command && !modifiers.alt
    };
    let line_edge = mac && modifiers.command && !modifiers.alt;
    let mv = |motion| Some(IdeEdit::Move(motion, select));
    match key {
        egui::Key::ArrowUp | egui::Key::ArrowDown
            if modifiers.alt && modifiers.shift && !modifiers.command =>
        {
            Some(IdeEdit::MoveLines {
                up: key == egui::Key::ArrowUp,
            })
        }
        egui::Key::ArrowUp | egui::Key::ArrowDown
            if modifiers.alt && !modifiers.shift && !modifiers.command && !modifiers.ctrl =>
        {
            Some(if key == egui::Key::ArrowUp {
                IdeEdit::ExtendSelection
            } else {
                IdeEdit::ShrinkSelection
            })
        }
        egui::Key::ArrowLeft if word => mv(IdeMotion::WordLeft),
        egui::Key::ArrowRight if word => mv(IdeMotion::WordRight),
        egui::Key::ArrowLeft if line_edge => mv(IdeMotion::LineStart),
        egui::Key::ArrowRight if line_edge => mv(IdeMotion::LineEnd),
        egui::Key::ArrowUp if line_edge => mv(IdeMotion::DocStart),
        egui::Key::ArrowDown if line_edge => mv(IdeMotion::DocEnd),
        egui::Key::Home if modifiers.command => mv(IdeMotion::DocStart),
        egui::Key::End if modifiers.command => mv(IdeMotion::DocEnd),
        _ if modifiers.command || modifiers.alt || modifiers.ctrl => match key {
            egui::Key::Backspace if word => Some(IdeEdit::DeleteWordBack),
            egui::Key::Backspace if line_edge => Some(IdeEdit::DeleteLines),
            egui::Key::Y if !mac && modifiers.command && !select => Some(IdeEdit::DeleteLines),
            egui::Key::D if modifiers.command && !select && !modifiers.alt => {
                Some(IdeEdit::Duplicate)
            }
            egui::Key::Slash if modifiers.command && !modifiers.alt => Some(IdeEdit::ToggleComment),
            egui::Key::Minus if modifiers.command && !modifiers.alt => {
                Some(IdeEdit::Fold { all: select })
            }
            egui::Key::Equals | egui::Key::Plus if modifiers.command && !modifiers.alt => {
                Some(IdeEdit::Unfold { all: select })
            }
            egui::Key::J if modifiers.ctrl && select && !modifiers.alt => Some(IdeEdit::JoinLines),
            egui::Key::Enter if modifiers.command && modifiers.alt && !select => {
                Some(IdeEdit::StartNewLine { above: true })
            }
            _ => None,
        },
        egui::Key::Enter if select => Some(IdeEdit::StartNewLine { above: false }),
        egui::Key::ArrowLeft => mv(IdeMotion::Left),
        egui::Key::ArrowRight => mv(IdeMotion::Right),
        egui::Key::ArrowUp => mv(IdeMotion::Up),
        egui::Key::ArrowDown => mv(IdeMotion::Down),
        egui::Key::Home => mv(IdeMotion::LineStart),
        egui::Key::End => mv(IdeMotion::LineEnd),
        egui::Key::PageUp => mv(IdeMotion::PageUp(page_rows)),
        egui::Key::PageDown => mv(IdeMotion::PageDown(page_rows)),
        _ => None,
    }
}

/// Whether an IDE chord applies with vim on. Vim keeps plain, Shift, and
/// word-modifier navigation; chords no vim key uses (Command or Ctrl
/// edits, line moves, paging) work in every mode. Start New Line works
/// in Insert mode only (Shift+Enter stays vim's Enter elsewhere).
#[must_use]
pub fn ide_edit_in_vim(edit: IdeEdit, modifiers: egui::Modifiers, mode: Mode) -> bool {
    match edit {
        IdeEdit::Move(IdeMotion::PageUp(_) | IdeMotion::PageDown(_), _) => true,
        IdeEdit::Move(..) | IdeEdit::DeleteWordBack => modifiers.command,
        IdeEdit::StartNewLine { .. } => mode == Mode::Insert,
        _ => true,
    }
}

/// Line-comment token for a language, when it has one.
#[must_use]
pub fn comment_token(language: Language) -> Option<&'static str> {
    Some(match language {
        Language::Rust
        | Language::JavaScript
        | Language::TypeScript
        | Language::Go
        | Language::C
        | Language::Cpp
        | Language::Java
        | Language::Scss
        | Language::Less => "//",
        Language::Python | Language::Shell | Language::Toml | Language::Yaml => "#",
        Language::Syntax(name) => match name {
            "Ruby" | "Perl" | "R" | "Makefile" | "Dockerfile" | "Elixir" | "Nix" | "CMake" => "#",
            "Lua" | "SQL" | "Haskell" => "--",
            "Swift" | "Kotlin" | "Scala" | "C#" | "Dart" | "Zig" | "PHP" | "Protocol Buffer"
            | "Objective-C" | "Groovy" => "//",
            "Lisp" | "Clojure" => ";",
            _ => return None,
        },
        Language::Json | Language::Markdown | Language::Html | Language::Css | Language::Plain => {
            return None;
        }
    })
}

/// Who owns Ctrl-V: vim's visual block (vim on, in a vim mode) or the IDE
/// paste. Insert and Command always paste, so modeless editing — and typing
/// `:w` — never loses paste to a mode the user is not in.
#[must_use]
pub fn ctrl_v_is_block(vim: bool, mode: Mode) -> bool {
    vim && matches!(
        mode,
        Mode::Normal | Mode::Visual | Mode::VisualLine | Mode::VisualBlock
    )
}

/// Who owns Ctrl-H: vim's Backspace motion or the IDE replace prompt.
/// Mirrors [`ctrl_v_is_block`]: vim owns it exactly where vim keys live.
#[must_use]
pub fn ctrl_h_is_motion(vim: bool, mode: Mode) -> bool {
    vim && mode != Mode::Insert && mode != Mode::Command
}

/// Stable keyboard-focus identity for one source view. The painter response
/// id is layout-derived, so panes use this to return focus to the file after
/// header-button clicks instead of stranding keys on Save/Reload.
#[must_use]
pub fn source_focus_id(id: &str) -> egui::Id {
    egui::Id::unique(("native-source-focus", id))
}

/// Vim footer prompt: `:ex` commands and `/search` share the command line.
pub fn command_prompt(engine: &impl ModalEngine) -> Option<String> {
    if engine.mode() != Mode::Command {
        return None;
    }
    let line = engine.command_line();
    if line.starts_with('/') {
        Some(line.to_string())
    } else {
        Some(format!(":{line}"))
    }
}

/// Header badge text for the current vim mode.
#[must_use]
pub fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Normal => "NORMAL",
        Mode::Insert => "INSERT",
        Mode::Visual => "VISUAL",
        Mode::VisualLine => "V-LINE",
        Mode::VisualBlock => "V-BLOCK",
        Mode::Command => "COMMAND",
    }
}

/// Vim mode badge colors, from the host theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeColors {
    pub normal: Color32,
    pub insert: Color32,
    pub visual: Color32,
    pub command: Color32,
}

impl ModeColors {
    /// Theme-derived fallback: accent, text, warning and weak text.
    #[must_use]
    pub fn from_visuals(visuals: &egui::Visuals) -> Self {
        Self {
            normal: visuals.selection.stroke.color,
            insert: visuals.text_color(),
            visual: visuals.warn_fg_color,
            command: visuals.weak_text_color(),
        }
    }
}

/// Badge color for the current vim mode.
#[must_use]
pub fn mode_color(mode: Mode, colors: &ModeColors) -> Color32 {
    match mode {
        Mode::Normal => colors.normal,
        Mode::Insert => colors.insert,
        Mode::Visual | Mode::VisualLine | Mode::VisualBlock => colors.visual,
        Mode::Command => colors.command,
    }
}
