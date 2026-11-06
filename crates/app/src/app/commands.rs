use eframe::egui::{self};
use egui_dock::NodeIndex;
use terminator_core::*;

use super::super::*;

/// Actions that need a focused session. Shortcuts, the palette and menus
/// share this list through [`App::shortcut_applies`].
pub(crate) const SESSION_ACTIONS: &[Action] = &[
    Action::SelectAll,
    Action::FindInTerminal,
    Action::SearchScrollback,
    Action::CopyWorkingDirectory,
    Action::RenameTerminal,
    Action::CloseSession,
    Action::ClearScrollback,
];

/// Directory that "Copy working directory" copies: the remote project root
/// for SSH sessions, the local working directory otherwise.
pub(crate) fn session_directory(session: &Session) -> String {
    session.remote.as_ref().map_or_else(
        || session.cwd.display().to_string(),
        |remote| remote.root.clone(),
    )
}

/// Context label for [`Action::CopyWorkingDirectory`].
pub(crate) const fn copy_directory_label(session: &Session) -> &'static str {
    if session.remote.is_some() {
        "Copy project directory"
    } else {
        Action::CopyWorkingDirectory.label()
    }
}

impl App {
    pub(crate) fn shortcut_allowed(&self, action: Action) -> bool {
        if self.command_dialog_open()
            || self.shortcut_capture.is_some()
            || self.rename_blocks_input()
            || self.picker_active
        {
            return false;
        }
        let switcher = matches!(action, Action::SwitcherNext | Action::SwitcherBack);
        if self.switcher.is_some() {
            return switcher;
        }
        if self.settings_open || self.player_open {
            return switcher
                || matches!(
                    action,
                    Action::OpenSettings
                        | Action::OpenPalette
                        | Action::RecentFiles
                        | Action::ToggleIdeMode
                );
        }
        true
    }

    /// One availability rule for every entry point: shortcuts, the
    /// palette and context menus. Cheap; no I/O.
    pub(crate) fn shortcut_applies(&self, action: Action) -> bool {
        match action {
            Action::FileStructure => self.active_editor_path().is_some(),
            Action::DiffNextChange
            | Action::DiffPrevChange
            | Action::DiffNextFile
            | Action::DiffPrevFile => self.active_diff_tab(),
            Action::EditorSave | Action::CompareDisk | Action::MoveToMain | Action::MoveToStrip => {
                self.active_session_record()
                    .is_some_and(|session| self.session_action_applies(action, session))
            }
            action if SESSION_ACTIONS.contains(&action) => self.active_session.is_some(),
            action if super::actions::GIT_ACTIONS.contains(&action) => {
                self.git_action_applies(action)
            }
            _ => true,
        }
    }

    /// Availability of a session-scoped action for `session`, which may be
    /// the focused session or the target of a context menu.
    pub(crate) fn session_action_applies(&self, action: Action, session: &Session) -> bool {
        let shell = session.kind == SessionKind::Shell;
        match action {
            Action::EditorSave | Action::CompareDisk => {
                session.kind == SessionKind::Editor && !session.review
            }
            Action::MoveToMain => shell && self.is_strip_session(&session.project_id, &session.id),
            // Floating windows are outside `layouts`, so they never match.
            Action::MoveToStrip => {
                shell
                    && self.preferences.ide_mode
                    && !self.is_strip_session(&session.project_id, &session.id)
                    && self
                        .layouts
                        .get(&session.project_id)
                        .is_some_and(|workspace| {
                            workspace.contains(&Tab::Terminal(session.id.clone()))
                        })
            }
            _ => true,
        }
    }

    fn active_session_record(&self) -> Option<&Session> {
        let sid = self.active_session.as_ref()?;
        self.state
            .sessions
            .iter()
            .find(|session| &session.id == sid)
    }

    /// The selected project's active pane is a native working-tree diff.
    fn active_diff_tab(&self) -> bool {
        self.selected
            .as_ref()
            .and_then(|project| self.layouts.get(project)?.active_pane())
            .is_some_and(|pane| matches!(pane, Tab::Diff { .. }))
    }

    /// Live or ended shell whose tab is in the strip and which currently has
    /// keyboard focus. Main-pane shells stay put.
    pub(crate) fn focused_strip_shell(&self) -> Option<String> {
        self.active_session_record()
            .filter(|session| self.session_action_applies(Action::MoveToMain, session))
            .map(|session| session.id.clone())
    }

    /// Focused shell in the main pane (not the IDE strip or a floating
    /// window) that can move to the strip.
    pub(crate) fn focused_main_shell(&self) -> Option<String> {
        self.active_session_record()
            .filter(|session| self.session_action_applies(Action::MoveToStrip, session))
            .map(|session| session.id.clone())
    }

    pub(crate) fn active_editor_id(&self) -> Option<String> {
        let sid = self.active_session.as_ref()?;
        self.state.sessions.iter().find_map(|session| {
            (session.id == *sid && session.kind == SessionKind::Editor && !session.review)
                .then(|| session.id.clone())
        })
    }

    /// Type-ahead applies to shell sessions only; editors draw their own
    /// cursor and buffer.
    pub(crate) fn type_ahead_mode(&self, sid: &str) -> egui_term::TypeAheadMode {
        let shell = self
            .state
            .sessions
            .iter()
            .any(|session| session.id == sid && session.kind == SessionKind::Shell);
        if shell {
            self.preferences.type_ahead.terminal_mode()
        } else {
            egui_term::TypeAheadMode::Off
        }
    }

    pub(crate) fn shortcut_label(&self, action: Action) -> String {
        shortcuts::pretty(&self.state.settings.keybindings, action.id())
    }

    pub(crate) fn run_shortcut(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::OpenFile => self.open_path = true,
            Action::NewTerminal => self.create(None),
            Action::SplitUp => self.create(Some("up")),
            Action::SplitDown => self.create(Some("down")),
            Action::SplitLeft => self.create(Some("left")),
            Action::SplitRight => self.create(Some("right")),
            Action::OpenSettings => self.open_settings(),
            Action::OpenPalette => self.open_command_palette(),
            Action::GotoSymbol => self.open_goto_symbol(),
            Action::FileStructure => self.open_file_structure(),
            Action::NavigateBack => self.navigate_history(false),
            Action::NavigateForward => self.navigate_history(true),
            Action::RecentFiles => {
                self.open_command_palette();
                self.palette.filters = crate::palette::FILTER_FILES;
            }
            Action::FindInFiles => self.find_in_files_action(ctx),
            Action::ReplaceInFiles => self.replace_in_files_action(ctx),
            Action::FindInTerminal => self.find_in_active_terminal(ctx),
            Action::NextPane => self.focus_next_pane(),
            Action::SelectAll => self.select_all_active(),
            Action::SearchScrollback => self.search_active_scrollback(ctx),
            Action::CopyWorkingDirectory => self.copy_active_working_directory(ctx),
            Action::RenameTerminal => self.rename_active_terminal(),
            Action::CloseSession => self.close_active_session(),
            Action::ClearScrollback => self.clear_active_scrollback(),
            Action::EditorSave => self.save_active_editor(),
            Action::CompareDisk => self.compare_active_editor(),
            Action::ToggleLeftSidebar => self.toggle_left_sidebar(),
            Action::ToggleRightSidebar => self.toggle_right_sidebar(),
            Action::ToggleIdeMode => self.toggle_ide_mode(),
            Action::MoveToMain => {
                if let Some(sid) = self.focused_strip_shell() {
                    self.move_strip_session_to_main(&sid);
                }
            }
            Action::MoveToStrip => {
                if let Some(sid) = self.focused_main_shell() {
                    self.move_main_session_to_strip(&sid);
                }
            }
            Action::NextAttention => self.next_attention(),
            Action::NewBrowser => self.open_new_browser_tab(None),
            Action::NewVnc => self.open_new_vnc_tab(None),
            Action::SwitcherNext => self.step_switcher(false),
            Action::SwitcherBack => self.step_switcher(true),
            Action::ToolExplorer
            | Action::ToolProblems
            | Action::ToolStructure
            | Action::ToolGit
            | Action::ToolTerminal
            | Action::SelectIn => self.run_tool_window_key(ctx, action),
            Action::DiffNextChange
            | Action::DiffPrevChange
            | Action::DiffNextFile
            | Action::DiffPrevFile => {
                self.diff_nav = Some(action);
                ctx.request_repaint();
            }
            Action::GitCommit
            | Action::GitCommitPush
            | Action::GitRefresh
            | Action::GitShowLog
            | Action::GitCompareWith
            | Action::GitCompareRevision
            | Action::GitShowHistory
            | Action::GitAnnotate
            | Action::GitStageAll => self.run_git_action(action),
        }
    }

    /// Go to Symbol: Search Everywhere with only the Symbols chip.
    pub(crate) fn open_goto_symbol(&mut self) {
        self.open_command_palette();
        self.palette.filters = crate::palette::FILTER_SYMBOLS;
    }

    /// File Structure popup: the active editor's outline, filterable.
    pub(crate) fn open_file_structure(&mut self) {
        let Some(path) = self.active_editor_path() else {
            return;
        };
        self.open_command_palette();
        self.palette.filters = crate::palette::FILTER_SYMBOLS;
        self.palette.file_scope = Some(path);
    }

    pub(crate) fn open_command_palette(&mut self) {
        // Settings and the Player draw above the palette in the center pane.
        if self.settings_open || self.player_open {
            self.hide_center_overlay();
        }
        self.palette.open = true;
        self.palette.focus_pending = true;
        self.palette.file_scope = None;
        self.palette.query.clear();
        self.palette.index = 0;
        self.palette.filters = 0;
        self.palette.cache = None;
        self.palette.query_changed_at = Instant::now();
        self.lsp.clear_workspace_symbols();
        // Go-to-file stays fresh: a background rescan starts here when
        // roots changed or the index aged out.
        self.refresh_file_index();
    }

    pub(crate) fn find_in_active_terminal(&mut self, ctx: &egui::Context) {
        let Some(sid) = self.active_session.clone() else {
            return;
        };
        self.terminal_find.entry(sid.clone()).or_default();
        ctx.memory_mut(|memory| {
            memory.request_focus(egui::Id::unique(("terminal-find", sid)));
        });
    }

    pub(crate) fn focus_next_pane(&mut self) {
        let Some(dock) = self
            .selected
            .as_ref()
            .and_then(|id| self.layouts.get_mut(id))
        else {
            return;
        };
        let nodes = dock
            .main_surface()
            .iter()
            .enumerate()
            .filter(|(_, node)| node.is_leaf())
            .map(|(index, _)| NodeIndex(index))
            .collect::<Vec<_>>();
        if nodes.is_empty() {
            return;
        }
        let current = dock.main_surface().focused_leaf();
        let index = nodes
            .iter()
            .position(|node| Some(*node) == current)
            .map(|index| {
                index
                    .saturating_add(1)
                    .checked_rem(nodes.len())
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        if let Some(node) = nodes.get(index) {
            dock.main_surface_mut().set_focused_node(*node);
        }
    }

    pub(crate) fn select_all_active(&mut self) {
        let Some(sid) = self.active_session.clone() else {
            return;
        };
        if let Some(backend) = self.backends.get_mut(&sid) {
            backend.select_all();
        }
    }

    /// A focused text field or native editor owns select-all: egui selects
    /// field text natively, so the global terminal select-all must not
    /// consume the chord first. This covers every `TextEdit` in the app
    /// (find bars, searches, palette, dialogs, settings) plus native
    /// editor panes, which use stable focus ids instead of `TextEdit`.
    pub(crate) fn text_input_focused(&self, ctx: &egui::Context) -> bool {
        if ctx.text_edit_focused() {
            return true;
        }
        terminator_native_edit::view::source_has_focus(ctx)
    }

    pub(crate) fn open_scrollback_search(&mut self, ctx: &egui::Context, sid: &str) {
        self.search_session = Some(sid.to_string());
        self.search_open = true;
        self.scrollback_hits = crate::app::ScrollbackHits::default();
        self.texts.remove(&format!("history:{sid}"));
        ctx.memory_mut(|memory| {
            memory.request_focus(egui::Id::unique("scrollback-search"));
        });
    }

    pub(crate) fn search_active_scrollback(&mut self, ctx: &egui::Context) {
        let Some(sid) = self.active_session.clone() else {
            return;
        };
        self.open_scrollback_search(ctx, &sid);
    }

    /// Copy `text` and say so in the status bar (`what` names it).
    pub(crate) fn copy_with_note(&mut self, ctx: &egui::Context, text: String, what: &str) {
        ctx.copy_text(text);
        self.info = Some(format!("{what} copied"));
    }

    pub(crate) fn copy_active_working_directory(&mut self, ctx: &egui::Context) {
        let directory = self
            .active_session_record()
            .map(session_directory)
            .or_else(|| self.cwd().map(|cwd| cwd.display().to_string()));
        let Some(directory) = directory else {
            return;
        };
        self.copy_with_note(ctx, directory, "Working directory");
    }

    pub(crate) fn rename_active_terminal(&mut self) {
        let Some(sid) = self.active_session.clone() else {
            return;
        };
        self.begin_rename(&sid, RenameSurface::Pane);
    }

    pub(crate) fn close_active_session(&mut self) {
        if let Some(sid) = self.active_session.clone() {
            self.close_session = Some(sid);
        }
    }

    pub(crate) fn clear_active_scrollback(&mut self) {
        let Some(sid) = self.active_session.clone() else {
            return;
        };
        self.send(Request::ClearHistory {
            session: Some(sid.clone()),
        });
        self.texts.remove(&format!("history:{sid}"));
    }

    pub(crate) fn save_active_editor(&mut self) {
        let Some(sid) = self.active_editor_id() else {
            return;
        };
        self.send(Request::EditorSave { session: sid });
    }

    pub(crate) fn compare_active_editor(&mut self) {
        let Some(sid) = self.active_editor_id() else {
            return;
        };
        self.send(Request::EditorCompare { session: sid });
    }
}
