//! `JetBrains` tool window keys: Cmd+1 Explorer, Cmd+6 Problems, Cmd+7
//! Structure, and Cmd+9 Git (Alt+digit on Linux), plus Alt+F1 Select In.
//! Alt+F12 is in `terminal_key`.

use crate::shortcuts::{self, Side, ToolClaim};
use crate::*;

/// What a tool window key does now (the `JetBrains` rule).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolStep {
    /// Hidden: show it on this side and focus it.
    Show(Side),
    /// Shown but not focused: focus it.
    Focus(Side),
    /// Shown and focused: hide it.
    Hide(Side),
}

/// Where one tool window shows, and which sidebar has keyboard focus.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ToolPlace {
    pub(crate) left: bool,
    pub(crate) right: bool,
    pub(crate) focused: Option<Side>,
    /// Where the tool opens when it is hidden.
    pub(crate) usual: Side,
}

impl ToolPlace {
    pub(crate) const fn shows(self, side: Side) -> bool {
        match side {
            Side::Left => self.left,
            Side::Right => self.right,
        }
    }

    /// The side where the tool shows now (its usual side first), else its
    /// usual side.
    pub(crate) fn side(self) -> Side {
        if !self.shows(self.usual) && self.shows(self.usual.other()) {
            self.usual.other()
        } else {
            self.usual
        }
    }

    pub(crate) fn step(self) -> ToolStep {
        match self.focused.filter(|side| self.shows(*side)) {
            Some(side) => ToolStep::Hide(side),
            None if self.shows(self.side()) => ToolStep::Focus(self.side()),
            None => ToolStep::Show(self.usual),
        }
    }
}

/// Each tool window key and the tool it toggles.
const TOOL_KEYS: [(Action, SidebarTool); 4] = [
    (Action::ToolExplorer, SidebarTool::Explorer),
    (Action::ToolProblems, SidebarTool::Problems),
    (Action::ToolStructure, SidebarTool::Structure),
    (Action::ToolGit, SidebarTool::Git),
];

/// The key action that toggles `tool`, if it has one.
pub(crate) fn tool_action(tool: SidebarTool) -> Option<Action> {
    TOOL_KEYS
        .iter()
        .find(|(_, candidate)| *candidate == tool)
        .map(|(action, _)| *action)
}

/// Drop keyboard focus, so the active terminal takes it when it paints.
pub(crate) fn release_focus(ctx: &egui::Context) {
    ctx.memory_mut(|memory| {
        if let Some(id) = memory.focused() {
            memory.surrender_focus(id);
        }
    });
}

impl App {
    /// A tool window key, from its Settings binding or the palette.
    pub(crate) fn run_tool_window_key(&mut self, ctx: &egui::Context, action: Action) {
        if action == Action::ToolTerminal {
            self.toggle_terminal_window(ctx);
        } else if action == Action::SelectIn {
            self.select_in_explorer(ctx);
        } else if let Some(&(_, tool)) = TOOL_KEYS.iter().find(|(key, _)| *key == action) {
            self.toggle_tool_window(ctx, tool);
        }
    }

    /// The tool shows on a side when that sidebar is open on it. Its usual
    /// side is the left one when a left stripe put it there.
    pub(crate) fn tool_place(&self, ctx: &egui::Context, tool: SidebarTool) -> ToolPlace {
        let prefs = &self.preferences;
        ToolPlace {
            left: prefs.left_visible && prefs.left_tool == Some(tool),
            right: prefs.visible && !prefs.right_projects && prefs.tool == tool,
            focused: shortcuts::focused_sidebar(ctx),
            usual: if prefs.left_tool == Some(tool) {
                Side::Left
            } else {
                Side::Right
            },
        }
    }

    pub(crate) fn toggle_tool_window(&mut self, ctx: &egui::Context, tool: SidebarTool) {
        match self.tool_place(ctx, tool).step() {
            ToolStep::Show(side) => {
                self.show_tool_window(side, tool);
                shortcuts::request_tool_focus(ctx, side, ToolClaim::default());
            }
            ToolStep::Focus(side) => shortcuts::request_tool_focus(ctx, side, ToolClaim::default()),
            ToolStep::Hide(side) => {
                match side {
                    Side::Left => self.preferences.left_visible = false,
                    Side::Right => self.preferences.visible = false,
                }
                self.return_focus_to_work(ctx);
            }
        }
    }

    fn show_tool_window(&mut self, side: Side, tool: SidebarTool) {
        match side {
            Side::Left => {
                self.preferences.left_tool = Some(tool);
                self.preferences.left_visible = true;
            }
            Side::Right => {
                self.preferences.right_projects = false;
                self.preferences.tool = tool;
                self.preferences.visible = true;
            }
        }
    }

    /// Alt+F1 (Select In): show and focus the Explorer, then select the
    /// current file there, as its locate button does.
    pub(crate) fn select_in_explorer(&mut self, ctx: &egui::Context) {
        let side = self.tool_place(ctx, SidebarTool::Explorer).side();
        self.show_tool_window(side, SidebarTool::Explorer);
        if self.explorer_reveal_candidate().is_none() {
            self.info = Some(self.explorer_reveal_note().into());
        }
        shortcuts::request_tool_focus(ctx, side, ToolClaim { reveal: true });
    }

    /// Give keyboard focus back to the last editor or terminal. A terminal
    /// takes free focus by itself; a native editor needs a request.
    pub(crate) fn return_focus_to_work(&mut self, ctx: &egui::Context) {
        release_focus(ctx);
        if self.active_session.is_some() {
            return;
        }
        let Some(project) = self.selected.clone() else {
            return;
        };
        let Some(dock) = self.layouts.remove(&project) else {
            return;
        };
        if let Some(pane) = dock.active_pane().cloned() {
            self.focus_moved_view(&pane, &project, &dock);
        }
        self.layouts.insert(project, dock);
    }

    /// Esc dropped the focus a tool window held: hand it back. Runs before
    /// any pane paints and uses the Esc up, so the terminal or editor that
    /// takes focus now does not read it (Esc stops an agent). An open
    /// Explorer name prompt, inline rename (History), popup, or modal
    /// handles its own Esc.
    pub(crate) fn escape_tool_window(&mut self, ctx: &egui::Context) {
        let overlay = egui::Popup::is_any_open(ctx)
            || ctx.memory(|memory| memory.top_modal_layer().is_some());
        let prompt = self.name_prompt.is_some() || self.rename_blocks_input();
        if !prompt && !overlay && shortcuts::escaped_tool_window(ctx) {
            Self::drop_key_events(ctx, egui::Key::Escape);
            self.return_focus_to_work(ctx);
        }
    }

    /// The key that toggles `tool`, as menus show it ("⌘1"), or "".
    pub(crate) fn tool_keys(&self, tool: SidebarTool) -> String {
        tool_action(tool).map_or_else(String::new, |action| self.shortcut_label(action))
    }

    /// A tool tooltip with its key: "Explorer (⌘1)".
    pub(crate) fn tool_tip(&self, label: &str, tool: SidebarTool) -> String {
        appearance::shortcut_tip(label, &self.tool_keys(tool))
    }
}
