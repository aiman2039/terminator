//! Parse, display, and consume Settings keybinding strings.
mod jetbrains;
mod meta;
mod migrate;
mod shared;
#[cfg(test)]
mod tests;
mod tool_focus;

use eframe::egui::{self, Key, Modifiers};
pub use jetbrains::*;
pub use meta::terminal_keeps;
pub use migrate::{
    CMD_E_MOVES, RENAME_TERMINAL_MOVES, fill_moved_defaults, migrate_moved_defaults,
};
pub use shared::{fill_defaults, yields};
use std::collections::{BTreeMap, HashMap};
pub use tool_focus::*;

macro_rules! actions {
    ($($variant:ident => $id:literal, $label:literal, $icon:literal;)*) => {
        /// A shortcut or palette command. `id` is the persisted
        /// keybinding key; Settings stay string-keyed. `label` and `icon`
        /// are the shared presentation for menus, toolbars and the palette.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Action {
            $($variant,)*
        }

        impl Action {
            /// Every action, in Settings and dispatch order.
            pub const ALL: &[Action] = &[$(Action::$variant,)*];

            pub const fn id(self) -> &'static str {
                match self {
                    $(Action::$variant => $id,)*
                }
            }

            pub const fn label(self) -> &'static str {
                match self {
                    $(Action::$variant => $label,)*
                }
            }

            /// Lucide icon name from the shared UI icon table.
            pub const fn icon(self) -> &'static str {
                match self {
                    $(Action::$variant => $icon,)*
                }
            }
        }
    };
}

actions! {
    NewTerminal => "new_terminal", "New terminal", "Terminal";
    OpenFile => "open_file", "Open file", "File";
    SplitUp => "split_up", "Split up", "PanelTopClose";
    SplitDown => "split_down", "Split down", "PanelBottomClose";
    SplitLeft => "split_left", "Split left", "PanelLeftClose";
    SplitRight => "split_right", "Split right", "PanelRightClose";
    // Before Next pane: egui ignores an extra Shift, so Cmd+] would also
    // take Cmd+Shift+] on macOS.
    NextTab => "next_tab", "Next tab", "ChevronRight";
    PrevTab => "prev_tab", "Previous tab", "ChevronLeft";
    NextPane => "next_pane", "Next pane", "ArrowRight";
    SelectAll => "select_all", "Select all", "TextSelect";
    // Before Find in terminal: egui ignores an extra Shift, so Cmd+F
    // would also take Cmd+Shift+F on macOS. On Linux both are
    // Ctrl+Shift+F, and focus picks one (see `shared`).
    FindInFiles => "find_in_files", "Find in Files", "Search";
    ReplaceInFiles => "replace_in_files", "Replace in Files", "Pencil";
    FindInTerminal => "find_in_terminal", "Find in terminal", "Search";
    SearchScrollback => "search_scrollback", "Search scrollback", "Search";
    CopyWorkingDirectory => "copy_working_directory", "Copy working directory", "Folder";
    RenameTerminal => "rename_terminal", "Rename terminal", "Pencil";
    CloseSession => "close_session", "Close session", "X";
    ClearScrollback => "clear_scrollback", "Clear saved scrollback", "Eraser";
    EditorSave => "editor_save", "Save all", "Save";
    CompareDisk => "compare_disk", "Compare disk", "FileDiff";
    ToggleLeftSidebar => "toggle_left_sidebar", "Toggle left sidebar", "PanelLeft";
    ToggleRightSidebar => "toggle_right_sidebar", "Toggle right sidebar", "PanelRight";
    ToggleIdeMode => "toggle_ide_mode", "Toggle IDE mode", "LayoutDashboard";
    MoveToMain => "move_to_main", "Move terminal to main pane", "ArrowUp";
    MoveToStrip => "move_to_strip", "Move terminal to lower pane", "ArrowDown";
    ZoomPane => "zoom_pane", "Zoom pane", "Maximize2";
    OpenSettings => "open_settings", "Settings", "Settings";
    OpenPalette => "open_palette", "Search Everywhere", "Search";
    GotoSymbol => "goto_symbol", "Go to symbol", "ListTree";
    FileStructure => "file_structure", "File structure", "ListTree";
    NavigateBack => "navigate_back", "Back", "ArrowLeft";
    NavigateForward => "navigate_forward", "Forward", "ArrowRight";
    RecentFiles => "recent_files", "Recent files", "History";
    NextAttention => "next_attention", "Next agent needing attention", "Bell";
    AskAgent => "ask_agent", "Ask agent…", "Terminal";
    NewBrowser => "new_browser", "New browser tab", "Globe";
    NewVnc => "new_vnc", "New VNC viewer", "Monitor";
    DiffNextChange => "diff_next_change", "Diff: next change", "ArrowDown";
    DiffPrevChange => "diff_prev_change", "Diff: previous change", "ArrowUp";
    DiffNextFile => "diff_next_file", "Diff: next file", "ArrowRight";
    DiffPrevFile => "diff_prev_file", "Diff: previous file", "ArrowLeft";
    GitCommit => "git_commit", "Git: Commit…", "CircleCheck";
    GitCommitPush => "git_commit_push", "Git: Commit and Push", "Upload";
    GitRefresh => "git_refresh", "Git: Refresh", "RefreshCw";
    GitShowLog => "git_show_log", "Git: Show Log", "ListTree";
    GitCompareWith => "git_compare_with", "Git: Compare with…", "GitCompareArrows";
    GitCompareRevision => "git_compare_revision", "Git: Compare with Revision…", "History";
    GitShowHistory => "git_show_history", "Git: Show History", "History";
    GitAnnotate => "git_annotate", "Git: Annotate with Git Blame", "History";
    GitStageAll => "git_stage_all", "Git: Stage All", "Plus";
    GitShowAllChanges => "git_show_all_changes", "Git: Show All Changes", "FileDiff";
    // Backward first: egui ignores extra Shift, so Ctrl+Tab would also
    // consume Ctrl+Shift+Tab.
    SwitcherBack => "tab_switcher_back", "Recent tab switcher (backward)", "Files";
    SwitcherNext => "tab_switcher", "Recent tab switcher", "Files";
    // Tool window keys come last, so a chord a user already gave to an
    // older action keeps working for it.
    ToolExplorer => "tool_explorer", "Explorer tool window", "Files";
    ToolProblems => "tool_problems", "Problems tool window", "CircleAlert";
    ToolStructure => "tool_structure", "Structure tool window", "ListTree";
    ToolGit => "tool_git", "Git tool window", "GitBranch";
    ToolTerminal => "tool_terminal", "Terminal tool window", "Terminal";
    SelectIn => "select_in", "Select in Explorer", "LocateFixed";
    ReopenBrowserTab => "reopen_browser_tab", "Reopen closed browser tab", "Globe";
    NewAgentSession => "new_agent_session", "New agent session…", "AgentGeneric";
    NewDrawing => "new_drawing", "New drawing", "Pencil";
    RunTask => "run_task", "Run task…", "Play";
    RerunTask => "rerun_task", "Rerun last task", "RefreshCw";
    ShowWelcome => "show_welcome", "Welcome and setup", "Info";
    ShowWhatsNew => "show_whats_new", "What's New", "Info";
    // Shell integration (OSC 133) command navigation in a terminal.
    PrevPrompt => "prev_prompt", "Previous prompt", "ArrowUp";
    NextPrompt => "next_prompt", "Next prompt", "ArrowDown";
    SelectCommand => "select_command", "Select command with output", "TextSelect";
    CopyLastOutput => "copy_last_output", "Copy last command output", "Copy";
    // Keyboard selection modes; their keys never reach the PTY.
    HintsMode => "hints_mode", "Terminal hints (open or copy by label)", "ExternalLink";
    CopyMode => "copy_mode", "Terminal copy mode", "Keyboard";
    ReviewLastTurn => "review_last_turn", "Review last agent turn", "History";
    PromptComposer => "prompt_composer", "Prompt composer…", "Pencil";
    SearchAllScrollback => "search_all_scrollback", "Search all scrollback…", "Search";
    // Typed keys go to every terminal of the tab; GUI memory only.
    BroadcastInput => "broadcast_input", "Broadcast input to all panes in this tab", "Radio";
    // System-wide too (global-hotkey); unbound by default.
    QuickTerminal => "quick_terminal", "Quick terminal (system-wide)", "Terminal";
    // `.terminator/layout.toml`: new terminals only, after the trust answer.
    OpenLaunchLayout => "open_launch_layout", "Open launch layout", "PanelsTopLeft";
    SaveLaunchLayout => "save_launch_layout", "Save layout as launch file", "Save";
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chord {
    pub modifiers: Modifiers,
    pub key: Key,
}

pub fn parse(value: &str) -> Option<Chord> {
    let parts = value
        .split('+')
        .map(|part| part.trim().to_lowercase())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let key_token = parts.last()?;
    let key = parse_key(key_token)?;
    let mut modifiers = Modifiers::NONE;
    let modifier_parts = parts.get(..parts.len().saturating_sub(1))?;
    for part in modifier_parts {
        match part.as_str() {
            "command" | "cmd" | "super" | "meta" => {
                if cfg!(target_os = "macos") {
                    modifiers.mac_cmd = true;
                    modifiers.command = true;
                } else {
                    // Stored `command` means Ctrl+Shift on Linux, matching existing help text.
                    modifiers.ctrl = true;
                    modifiers.command = true;
                    if *part == "command" {
                        modifiers.shift = true;
                    }
                }
            }
            "ctrl" | "control" => {
                modifiers.ctrl = true;
                if !cfg!(target_os = "macos") {
                    modifiers.command = true;
                }
            }
            "alt" | "option" => modifiers.alt = true,
            "shift" => modifiers.shift = true,
            _ => return None,
        }
    }
    Some(Chord { modifiers, key })
}

fn parse_key(token: &str) -> Option<Key> {
    Key::from_name(token)
        .or_else(|| {
            Key::ALL
                .iter()
                .copied()
                .find(|key| key.name().eq_ignore_ascii_case(token))
        })
        .or(match token {
            "," | "comma" => Some(Key::Comma),
            "." | "period" | "dot" => Some(Key::Period),
            ";" | "semicolon" => Some(Key::Semicolon),
            "'" | "quote" => Some(Key::Quote),
            "/" | "slash" => Some(Key::Slash),
            "\\" | "backslash" => Some(Key::Backslash),
            "[" => Some(Key::OpenBracket),
            "]" => Some(Key::CloseBracket),
            "-" | "minus" => Some(Key::Minus),
            "=" | "equals" => Some(Key::Equals),
            "`" | "backtick" => Some(Key::Backtick),
            "space" => Some(Key::Space),
            "enter" | "return" => Some(Key::Enter),
            "esc" | "escape" => Some(Key::Escape),
            "tab" => Some(Key::Tab),
            "backspace" => Some(Key::Backspace),
            _ => None,
        })
}

pub fn display(value: &str) -> String {
    let Some(chord) = parse(value) else {
        return value.to_string();
    };
    let mut parts = Vec::new();
    if cfg!(target_os = "macos") {
        if chord.modifiers.mac_cmd || chord.modifiers.command && !chord.modifiers.ctrl {
            parts.push("⌘");
        }
        if chord.modifiers.ctrl && !chord.modifiers.mac_cmd {
            parts.push("⌃");
        }
        if chord.modifiers.alt {
            parts.push("⌥");
        }
        if chord.modifiers.shift {
            parts.push("⇧");
        }
        format!("{}{}", parts.join(""), key_label(chord.key))
    } else {
        if chord.modifiers.ctrl || chord.modifiers.command {
            parts.push("Ctrl");
        }
        if chord.modifiers.alt {
            parts.push("Alt");
        }
        if chord.modifiers.shift {
            parts.push("Shift");
        }
        parts.push(key_label(chord.key));
        parts.join("+")
    }
}

fn key_label(key: Key) -> &'static str {
    match key {
        Key::Comma => ",",
        Key::Period => ".",
        Key::Semicolon => ";",
        Key::Quote => "'",
        Key::Slash => "/",
        Key::Backslash => "\\",
        Key::OpenBracket => "[",
        Key::CloseBracket => "]",
        Key::Minus => "-",
        Key::Equals => "=",
        Key::Backtick => "`",
        Key::Space => "Space",
        Key::Enter => "Enter",
        Key::Escape => "Esc",
        Key::Tab => "Tab",
        Key::Backspace => "Backspace",
        Key::ArrowUp => "↑",
        Key::ArrowDown => "↓",
        Key::ArrowLeft => "←",
        Key::ArrowRight => "→",
        other => other.symbol_or_name(),
    }
}

pub fn from_input(modifiers: Modifiers, key: Key) -> Option<String> {
    if matches!(
        key,
        Key::Escape
            | Key::Tab
            | Key::Enter
            | Key::Backspace
            | Key::Delete
            | Key::ArrowUp
            | Key::ArrowDown
            | Key::ArrowLeft
            | Key::ArrowRight
    ) && !modifiers.any()
    {
        return None;
    }
    let mut parts = Vec::new();
    if cfg!(target_os = "macos") {
        if modifiers.mac_cmd || modifiers.command {
            parts.push("command");
        }
        if modifiers.ctrl && !modifiers.mac_cmd {
            parts.push("ctrl");
        }
    } else if modifiers.ctrl && modifiers.shift && !modifiers.alt {
        parts.push("command");
    } else if modifiers.ctrl || modifiers.command {
        parts.push("ctrl");
    }
    if modifiers.alt {
        parts.push("alt");
    }
    if modifiers.shift
        && !(cfg!(not(target_os = "macos")) && parts.first().is_some_and(|p| *p == "command"))
    {
        parts.push("shift");
    }
    if parts.is_empty() {
        return None;
    }
    let key = meta::key_token(key);
    parts.push(&key);
    Some(parts.join("+"))
}

pub fn consume(ctx: &egui::Context, value: &str) -> bool {
    let Some(chord) = parse(value) else {
        return false;
    };
    if ctx.input(|input| meta::modifier_variant(input, chord)) {
        return false;
    }
    if meta::types_text(chord) {
        return ctx.input_mut(|input| meta::consume_with_text(input, chord));
    }
    ctx.input_mut(|input| {
        input.consume_key(chord.modifiers, chord.key)
            || meta::shifted_key(chord).is_some_and(|key| input.consume_key(chord.modifiers, key))
    })
}

/// True when a focused native editor owns this chord as an IDE caret
/// motion or line edit (Cmd+Shift+Left selects to line start there, it
/// does not split the pane), or project navigation. The editor wins,
/// as in `IntelliJ`. `jetbrains` adds the keymap's editor chords
/// (Cmd+B declaration, Cmd+L go to line, F2 next problem).
pub fn editor_owns(value: &str, jetbrains: bool) -> bool {
    parse(value).is_some_and(|chord| {
        let mac = cfg!(target_os = "macos");
        if jetbrains
            && terminator_native_edit::view::jetbrains_shortcut(
                chord.key,
                chord.modifiers,
                true,
                mac,
            )
            .is_some()
        {
            return true;
        }
        terminator_native_edit::view::ide_edit_key(terminator_native_edit::view::IdeEditKeyInput {
            key: chord.key,
            modifiers: chord.modifiers,
            mac: cfg!(target_os = "macos"),
            page_rows: 1,
        })
        .is_some()
            || terminator_native_edit::view::ide_shortcut(chord.key, chord.modifiers).is_some()
            // `true`: Cmd+Alt+B is Go to Implementation in the editor,
            // not New browser tab.
            || terminator_native_edit::view::ide_navigation_shortcut(
                chord.key,
                chord.modifiers,
                true,
            )
            .is_some()
    })
}

/// Double Shift (`JetBrains` Search Everywhere): two Shift taps with no
/// other key between them, inside [`DoubleShift::WINDOW`].
#[derive(Debug, Default)]
pub struct DoubleShift {
    down: bool,
    dirty: bool,
    last_tap: Option<std::time::Instant>,
}

impl DoubleShift {
    pub const WINDOW: std::time::Duration = std::time::Duration::from_millis(400);

    /// Feed one frame: `shift` alone is held, `other` input arrived.
    /// Returns true on the second tap.
    pub fn update(&mut self, shift: bool, other: bool, now: std::time::Instant) -> bool {
        if other {
            self.dirty = true;
            self.last_tap = None;
        }
        if shift && !self.down {
            self.down = true;
            self.dirty = other;
            return false;
        }
        if shift || !self.down {
            return false;
        }
        self.down = false;
        if self.dirty {
            return false;
        }
        let second = self
            .last_tap
            .is_some_and(|last| now.saturating_duration_since(last) <= Self::WINDOW);
        self.last_tap = if second { None } else { Some(now) };
        second
    }
}

pub fn binding(map: &BTreeMap<String, String>, action: &str) -> String {
    // Built once: shortcuts look up unbound actions here every frame.
    static DEFAULTS: std::sync::LazyLock<BTreeMap<String, String>> =
        std::sync::LazyLock::new(|| terminator_core::Settings::default().keybindings);
    map.get(action)
        .or_else(|| DEFAULTS.get(action))
        .cloned()
        .unwrap_or_default()
}

pub fn pretty(map: &BTreeMap<String, String>, action: &str) -> String {
    let value = binding(map, action);
    if value.is_empty() {
        String::new()
    } else {
        display(&value)
    }
}

pub fn conflicts(map: &BTreeMap<String, String>) -> Vec<(String, String)> {
    let mut by_chord: HashMap<String, Vec<String>> = HashMap::new();
    for (action, value) in map {
        if value.trim().is_empty() {
            continue;
        }
        let Some(chord) = parse(value) else {
            continue;
        };
        by_chord
            .entry(format!("{chord:?}"))
            .or_default()
            .push(action.clone());
    }
    let mut out = Vec::new();
    for mut actions in by_chord.into_values() {
        // Focus picks one action of a shared pair; they never compete.
        shared::drop_partners(&mut actions);
        if actions.len() > 1 {
            for pair in actions.windows(2) {
                let (Some(left), Some(right)) = (pair.first(), pair.get(1)) else {
                    continue;
                };
                out.push((left.clone(), right.clone()));
            }
        }
    }
    out
}

pub fn invalid(map: &BTreeMap<String, String>) -> Option<String> {
    for (action, value) in map {
        if value.trim().is_empty() {
            continue;
        }
        if parse(value).is_none() {
            return Some(format!("Unknown shortcut for {action}: {value}"));
        }
    }
    let clashes = conflicts(map);
    if let Some((left, right)) = clashes.first() {
        return Some(format!(
            "Shortcuts for {left} and {right} use the same keys"
        ));
    }
    None
}
