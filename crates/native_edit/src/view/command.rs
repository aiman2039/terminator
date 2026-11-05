//! Editor commands a host runs without their keys, such as the Find Action
//! palette: the `JetBrains` name, where each one applies, the chord that
//! runs it in a focused view, and a queue the view drains on its next frame.
use crate::highlight::Language;
use egui::{Key, Modifiers};
use std::time::{Duration, Instant};

/// How long a queued command waits for its view. The next frame runs it;
/// a view that stays hidden drops it, so it never runs much later.
const COMMAND_TTL: Duration = Duration::from_secs(2);

macro_rules! editor_commands {
    ($($variant:ident => $name:literal, $keywords:literal;)*) => {
        /// One native editor command. It does what its chord does in a
        /// focused view.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum EditorCommand {
            $($variant,)*
        }

        impl EditorCommand {
            /// Every command, in palette order.
            pub const ALL: &[Self] = &[$(Self::$variant,)*];

            /// The `JetBrains` action name.
            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $name,)*
                }
            }

            /// More words a search matches: the editor menu label and
            /// other names for the command.
            pub const fn keywords(self) -> &'static str {
                match self {
                    $(Self::$variant => $keywords,)*
                }
            }
        }
    };
}

editor_commands! {
    DuplicateLine => "Duplicate Line or Selection", "copy line";
    DeleteLine => "Delete Line", "remove line";
    MoveLineUp => "Move Line Up", "statement";
    MoveLineDown => "Move Line Down", "statement";
    CommentLine => "Comment with Line Comment", "toggle uncomment";
    JoinLines => "Join Lines", "merge";
    ExtendSelection => "Extend Selection", "expand grow select word";
    ShrinkSelection => "Shrink Selection", "reduce select word";
    SelectAllOccurrences => "Select All Occurrences", "multiple carets cursors";
    AddNextOccurrence => "Add Selection for Next Occurrence", "multiple carets cursors";
    UnselectOccurrence => "Unselect Occurrence", "multiple carets cursors";
    Fold => "Fold", "collapse folding";
    Unfold => "Unfold", "expand folding";
    FoldAll => "Fold All", "collapse all folding";
    UnfoldAll => "Unfold All", "expand all folding";
    GotoLine => "Go to Line", "column jump";
    Find => "Find", "search in file";
    Replace => "Replace", "replace in file";
    NextProblem => "Next Highlighted Error", "problem diagnostic";
    PreviousProblem => "Previous Highlighted Error", "problem diagnostic";
    ContextActions => "Show Context Actions", "intention quick fix code actions";
    CopyReference => "Copy Reference", "path line";
    Undo => "Undo", "history";
    Redo => "Redo", "history";
    Reformat => "Reformat Code", "format";
    GotoDeclaration => "Go to Declaration", "definition";
    GotoImplementation => "Go to Implementation", "implementations";
    GotoTypeDeclaration => "Go to Type Declaration", "type definition";
    FindUsages => "Find Usages", "references";
    Rename => "Rename", "refactor symbol";
    QuickDocumentation => "Quick Documentation", "hover";
    CallHierarchy => "Call Hierarchy", "callers";
    TypeHierarchy => "Type Hierarchy", "supertypes subtypes";
    Completion => "Basic Code Completion", "autocomplete suggest";
}

impl EditorCommand {
    /// Needs a language server for the file's language.
    pub const fn needs_server(self) -> bool {
        matches!(
            self,
            Self::Reformat
                | Self::GotoDeclaration
                | Self::GotoImplementation
                | Self::GotoTypeDeclaration
                | Self::FindUsages
                | Self::Rename
                | Self::QuickDocumentation
                | Self::CallHierarchy
                | Self::TypeHierarchy
                | Self::Completion
        )
    }

    /// Changes text (code actions apply edits), so a read-only buffer
    /// refuses it.
    pub const fn edits(self) -> bool {
        matches!(
            self,
            Self::DuplicateLine
                | Self::DeleteLine
                | Self::MoveLineUp
                | Self::MoveLineDown
                | Self::CommentLine
                | Self::JoinLines
                | Self::Replace
                | Self::ContextActions
                | Self::Undo
                | Self::Redo
                | Self::Reformat
                | Self::Rename
                | Self::Completion
        )
    }

    /// Whether the command works in a view: language-server commands need
    /// a server for `language`, multiple carets need vim off, and line
    /// comments need a comment token. A read-only view takes no edits and
    /// no carets, and reads no keys, so it opens no prompt to type in.
    #[must_use]
    pub fn available(self, language: Language, vim: bool, read_only: bool) -> bool {
        let carets = matches!(
            self,
            Self::SelectAllOccurrences | Self::AddNextOccurrence | Self::UnselectOccurrence
        );
        let prompt = self == Self::GotoLine || (vim && self == Self::Find);
        !(self.needs_server() && crate::lsp::server_language(language).is_none())
            && !(carets && vim)
            && !(read_only && (self.edits() || carets || prompt))
            && (self != Self::CommentLine || crate::view::comment_token(language).is_some())
    }

    /// The chord that runs this command in a focused view, as a keyboard
    /// reports it. `mac` picks the macOS keymap; `jetbrains` is Settings →
    /// Editor → Keymap. `None` when no key runs the command.
    #[must_use]
    pub fn chord(self, mac: bool, jetbrains: bool) -> Option<(Key, Modifiers)> {
        let command = if mac {
            Modifiers::MAC_CMD.plus(Modifiers::COMMAND)
        } else {
            Modifiers::CTRL.plus(Modifiers::COMMAND)
        };
        // The Control key: its own key on macOS, the command key elsewhere.
        let ctrl = if mac { Modifiers::CTRL } else { command };
        let alt_shift = Modifiers::ALT | Modifiers::SHIFT;
        let problem = if jetbrains { Key::F2 } else { Key::F8 };
        Some(match self {
            Self::DuplicateLine => (Key::D, command),
            Self::DeleteLine if mac => (Key::Backspace, command),
            Self::DeleteLine => (Key::Y, command),
            Self::MoveLineUp => (Key::ArrowUp, alt_shift),
            Self::MoveLineDown => (Key::ArrowDown, alt_shift),
            Self::CommentLine => (Key::Slash, command),
            Self::JoinLines => (Key::J, ctrl | Modifiers::SHIFT),
            Self::ExtendSelection => (Key::ArrowUp, Modifiers::ALT),
            Self::ShrinkSelection => (Key::ArrowDown, Modifiers::ALT),
            Self::SelectAllOccurrences | Self::AddNextOccurrence | Self::UnselectOccurrence => {
                self.caret_chord(mac, command)
            }
            Self::Fold => (Key::Minus, command),
            Self::Unfold => (Key::Equals, command),
            Self::FoldAll => (Key::Minus, command | Modifiers::SHIFT),
            Self::UnfoldAll => (Key::Equals, command | Modifiers::SHIFT),
            Self::GotoLine if !jetbrains => return None,
            Self::GotoLine => (if mac { Key::L } else { Key::G }, command),
            Self::Find => (Key::F, command),
            Self::Replace => (Key::R, command),
            Self::NextProblem => (problem, Modifiers::NONE),
            Self::PreviousProblem => (problem, Modifiers::SHIFT),
            Self::ContextActions => (Key::Enter, Modifiers::ALT),
            Self::CopyReference => (Key::C, command | alt_shift),
            Self::Undo => (Key::Z, command),
            Self::Redo => (Key::Z, command | Modifiers::SHIFT),
            Self::Reformat => (Key::L, command | Modifiers::ALT),
            Self::GotoDeclaration if jetbrains => (Key::B, command),
            Self::GotoDeclaration => (Key::F12, Modifiers::NONE),
            Self::GotoImplementation => (Key::B, command | Modifiers::ALT),
            Self::GotoTypeDeclaration => (Key::B, command | Modifiers::SHIFT),
            Self::FindUsages => (Key::F7, Modifiers::ALT),
            Self::Rename if jetbrains => (Key::F6, Modifiers::SHIFT),
            Self::Rename => (Key::F2, Modifiers::NONE),
            Self::QuickDocumentation => (Key::K, command),
            Self::CallHierarchy => (Key::H, ctrl | Modifiers::ALT),
            Self::TypeHierarchy => return None,
            Self::Completion => (Key::Space, ctrl),
        })
    }

    /// Occurrence carets: Ctrl+G, Ctrl+Shift+G, Ctrl+Cmd+G on macOS;
    /// Alt+J, Alt+Shift+J, Ctrl+Alt+Shift+J elsewhere.
    fn caret_chord(self, mac: bool, command: Modifiers) -> (Key, Modifiers) {
        let (key, base) = if mac {
            (Key::G, Modifiers::CTRL)
        } else {
            (Key::J, Modifiers::ALT)
        };
        match self {
            Self::AddNextOccurrence => (key, base),
            Self::UnselectOccurrence => (key, base | Modifiers::SHIFT),
            _ if mac => (key, base | command),
            _ => (key, base | command | Modifiers::SHIFT),
        }
    }
}

/// A chord as menus show it: `⌥⇧⌘C` on macOS, `Ctrl+Alt+Shift+C` elsewhere.
#[must_use]
pub fn chord_label(key: Key, modifiers: Modifiers, mac: bool) -> String {
    let name = match key {
        Key::ArrowUp => "↑",
        Key::ArrowDown => "↓",
        Key::ArrowLeft => "←",
        Key::ArrowRight => "→",
        Key::Backspace if mac => "⌫",
        Key::Enter if mac => "↩",
        Key::Slash => "/",
        Key::Minus => "-",
        Key::Equals => "=",
        other => other.name(),
    };
    if mac {
        let symbols = [
            (modifiers.ctrl, "⌃"),
            (modifiers.alt, "⌥"),
            (modifiers.shift, "⇧"),
            (modifiers.mac_cmd || modifiers.command, "⌘"),
        ];
        let held = symbols.iter().filter(|(on, _)| *on).map(|(_, part)| *part);
        held.chain([name]).collect()
    } else {
        let words = [
            (modifiers.ctrl || modifiers.command, "Ctrl"),
            (modifiers.alt, "Alt"),
            (modifiers.shift, "Shift"),
        ];
        let held = words.iter().filter(|(on, _)| *on).map(|(_, part)| *part);
        held.chain([name]).collect::<Vec<_>>().join("+")
    }
}

pub(super) fn queue_id(id: &str) -> egui::Id {
    egui::Id::unique(("native-editor-command", id))
}

/// Queue `command` for the source view `id`. The view runs it on its next
/// frame and takes keyboard focus, as if its chord was pressed there.
pub fn queue_command(ctx: &egui::Context, id: &str, command: EditorCommand) {
    ctx.data_mut(|data| data.insert_temp(queue_id(id), (command, Instant::now())));
    ctx.request_repaint();
}

/// The command waiting for view `id`, if any.
#[must_use]
pub fn queued_command(ctx: &egui::Context, id: &str) -> Option<EditorCommand> {
    ctx.data(|data| data.get_temp::<(EditorCommand, Instant)>(queue_id(id)))
        .map(|(command, _)| command)
}

/// Take the command waiting for view `id`. One older than [`COMMAND_TTL`]
/// is dropped instead.
pub(super) fn take_command(ctx: &egui::Context, id: &str) -> Option<EditorCommand> {
    let (command, queued) = ctx.data_mut(|data| {
        let queued = data.get_temp::<(EditorCommand, Instant)>(queue_id(id));
        data.remove::<(EditorCommand, Instant)>(queue_id(id));
        queued
    })?;
    (queued.elapsed() < COMMAND_TTL).then_some(command)
}
