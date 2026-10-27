use eframe::egui::{self};
use egui_dock::NodeIndex;
use std::collections::{HashMap, HashSet};
use terminator_core::*;

use super::super::*;
impl App {
    pub(crate) fn shortcut_allowed(&self, action: Action) -> bool {
        if self.command_dialog_open()
            || self.shortcut_capture.is_some()
            || self.rename_blocks_input()
            || self.picker_active
        {
            return false;
        }
        if self.settings_open || self.player_open {
            return matches!(
                action,
                Action::OpenSettings | Action::OpenPalette | Action::ToggleIdeMode
            );
        }
        true
    }

    pub(crate) fn shortcut_applies(&self, action: Action) -> bool {
        match action {
            Action::EditorSave | Action::CompareDisk => self.active_editor_id().is_some(),
            Action::FileStructure => self.active_editor_path().is_some(),
            Action::MoveToMain => self.focused_strip_shell().is_some(),
            Action::MoveToStrip => self.preferences.ide_mode && self.focused_main_shell().is_some(),
            _ => true,
        }
    }

    /// Live or ended shell whose tab is in the strip and which currently has
    /// keyboard focus. Main-pane shells stay put.
    pub(crate) fn focused_strip_shell(&self) -> Option<String> {
        let sid = self.active_session.as_ref()?;
        let session = self
            .state
            .sessions
            .iter()
            .find(|session| &session.id == sid)?;
        (session.kind == SessionKind::Shell && self.is_strip_session(&session.project_id, sid))
            .then(|| sid.clone())
    }

    /// Shell that is open in the main pane, not the IDE strip.
    pub(crate) fn focused_main_shell(&self) -> Option<String> {
        let sid = self.active_session.as_ref()?;
        let session = self
            .state
            .sessions
            .iter()
            .find(|session| &session.id == sid)?;
        if session.kind != SessionKind::Shell || self.is_strip_session(&session.project_id, sid) {
            return None;
        }
        let pane = Tab::Terminal(sid.clone());
        self.layouts
            .get(&session.project_id)
            .is_some_and(|workspace| workspace.contains(&pane))
            .then(|| sid.clone())
    }

    pub(crate) fn active_editor_id(&self) -> Option<String> {
        let sid = self.active_session.as_ref()?;
        self.state.sessions.iter().find_map(|session| {
            (session.id == *sid && session.kind == SessionKind::Editor && !session.review)
                .then(|| session.id.clone())
        })
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
                if self.preferences.ide_mode
                    && let Some(sid) = self.focused_main_shell()
                {
                    self.move_main_session_to_strip(&sid);
                }
            }
            Action::NextAttention => self.next_attention(),
            Action::NewBrowser => self.open_new_browser_tab(None),
            Action::NewVnc => self.open_new_vnc_tab(None),
        }
    }

    pub(crate) fn present_session(
        &self,
        id: &str,
    ) -> std::sync::Arc<agent_presence::AgentPresentation> {
        self.presentations
            .borrow_mut()
            .get(&self.state, id, now(), self.services.presence_fresh())
    }

    pub(crate) fn reconcile_presentations(&mut self) {
        let fresh = self.services.presence_fresh();
        let moment = now();
        self.presentations
            .get_mut()
            .reconcile(&self.state, moment, fresh);
    }

    pub(crate) fn cached_tab_attention(
        &self,
        session_ids: &[String],
    ) -> agent_presence::AttentionCounts {
        self.presentations.borrow_mut().attention(
            &self.state,
            session_ids,
            now(),
            self.services.presence_fresh(),
        )
    }

    /// Global cycle order for attention navigation: distinct live terminals
    /// with pending permission/input requests first, then failures; oldest
    /// first within each group. Dismissal and snooze are respected.
    pub(crate) fn attention_order(&self) -> Vec<String> {
        let live: HashSet<&str> = self
            .state
            .sessions
            .iter()
            .filter(|s| s.lifecycle.live())
            .map(|s| s.id.as_str())
            .collect();
        let mut earliest_waiting: HashMap<&str, u64> = HashMap::new();
        let mut earliest_failed: HashMap<&str, u64> = HashMap::new();
        for notice in &self.state.notifications {
            if !live.contains(notice.session_id.as_str())
                || !agent_presence::notice_pending(notice, now())
            {
                continue;
            }
            let slot = match notice.state {
                AgentState::WaitingInput | AgentState::WaitingPermission => &mut earliest_waiting,
                AgentState::Failed => &mut earliest_failed,
                _ => continue,
            };
            slot.entry(notice.session_id.as_str())
                .and_modify(|seen| *seen = (*seen).min(notice.created))
                .or_insert(notice.created);
        }
        let mut order: Vec<(&str, u64)> = earliest_waiting.into_iter().collect();
        order.sort_by_key(|(id, created)| (*created, *id));
        let mut failed: Vec<(&str, u64)> = earliest_failed
            .into_iter()
            .filter(|(session, _)| !order.iter().any(|(waiting, _)| waiting == session))
            .collect();
        failed.sort_by_key(|(id, created)| (*created, *id));
        order
            .into_iter()
            .chain(failed)
            .map(|(session, _)| session.to_string())
            .collect()
    }

    /// Reveal the next terminal needing attention, cycling globally through
    /// [`Self::attention_order`] and reusing session navigation.
    pub(crate) fn next_attention(&mut self) {
        let order = self.attention_order();
        if order.is_empty() {
            self.info = Some("No agents need attention".into());
            return;
        }
        let next = self
            .active_session
            .as_ref()
            .and_then(|active| order.iter().position(|id| id == active))
            .map_or(0, |index| {
                index
                    .saturating_add(1)
                    .checked_rem(order.len())
                    .unwrap_or(0)
            });
        let Some(target) = order.get(next).cloned() else {
            return;
        };
        self.go_session(&target);
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
        self.palette.open = true;
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

    pub(crate) fn copy_active_working_directory(&self, ctx: &egui::Context) {
        let cwd = self
            .active_session
            .as_ref()
            .and_then(|sid| {
                self.state
                    .sessions
                    .iter()
                    .find(|session| &session.id == sid)
            })
            .map(|session| session.cwd.clone())
            .or_else(|| self.cwd());
        let Some(cwd) = cwd else {
            return;
        };
        ctx.copy_text(cwd.display().to_string());
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
