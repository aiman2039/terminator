use eframe::egui::{self};
use std::collections::HashSet;
use terminator_core::*;

use super::super::*;
impl App {
    pub(crate) fn create(&mut self, split: Option<&str>) {
        if self
            .selected
            .as_ref()
            .is_some_and(|project| self.missing_projects.contains(project))
        {
            return;
        }
        self.hide_center_overlay();
        if split.is_some() && self.strip_focused() {
            self.create_strip_split(split);
            return;
        }
        if split.is_none() {
            self.create_workspace_tab(None);
            return;
        }
        if let Some(project) = &self.selected {
            let _ = self.jobs.send(Job::rpc(
                Request::Create {
                    project: project.clone(),
                    cwd: self.cwd(),
                    file: None,
                    line: None,
                    column: None,
                    editor: false,
                },
                self.editor_target(project, None, split),
            ));
        }
    }
    pub(crate) fn create_workspace_tab(&mut self, index: Option<usize>) {
        self.hide_center_overlay();
        let Some(project) = self.selected.clone() else {
            return;
        };
        if self.selected_project().is_none() || self.missing_projects.contains(&project) {
            return;
        }
        let tab_id = id();
        if let Some(index) = index {
            self.workspace_insert.insert(tab_id.clone(), index);
        }
        let _ = self.jobs.send(Job::rpc(
            Request::Create {
                project,
                cwd: None,
                file: None,
                line: None,
                column: None,
                editor: false,
            },
            After::Workspace(tab_id, Vec::new()),
        ));
    }
    pub(crate) fn begin_workspace_close_tabs(&mut self, project: &str, ids: Vec<String>) {
        self.rename_session = None;
        let mut ids = ids.into_iter();
        let Some(first) = ids.next() else {
            return;
        };
        self.close_workspace_queue = ids.collect();
        self.close_workspace = Some((project.to_owned(), first));
    }
    pub(crate) fn abort_workspace_close(&mut self) {
        self.close_workspace = None;
        self.close_workspace_queue.clear();
    }
    pub(crate) fn close_workspace_tab_now(&mut self, project: &str, tab_id: &str) {
        if let Some(workspace) = self.layouts.get_mut(project) {
            let previous = workspace.active.clone();
            workspace.close(tab_id);
            if self.selected.as_deref() == Some(project) && workspace.active != previous {
                self.active_session = match workspace.active_pane() {
                    Some(Tab::Terminal(sid)) => Some(sid.clone()),
                    _ => None,
                };
                self.non_terminal_selected = self.active_session.is_none();
            }
        }
        self.prune_native_docs();
        self.prune_diff_docs();
        self.advance_workspace_close(project);
    }
    /// Drop cached split-pane scroll for diffs with no live tab. Data-level tab
    /// removal bypasses per-document cleanup, so call it after tabs close.
    pub(crate) fn prune_diff_docs(&mut self) {
        let mut live: HashSet<String> = self
            .layouts
            .values()
            .flat_map(|workspace| &workspace.tabs)
            .flat_map(|tab| tab.layout.iter_all_tabs())
            .filter(|(_, tab)| matches!(tab, Tab::Diff { .. }))
            .map(|(_, tab)| tab.key())
            .collect();
        live.extend(
            self.floating
                .iter()
                .filter_map(|window| window.dock.as_ref())
                .flat_map(|dock| dock.iter_all_tabs())
                .filter(|(_, tab)| matches!(tab, Tab::Diff { .. }))
                .map(|(_, tab)| tab.key()),
        );
        self.diff_split_scroll.retain(|key, _| live.contains(key));
    }
    pub(crate) fn advance_workspace_close(&mut self, project: &str) {
        let existing: HashSet<String> = self
            .layouts
            .get(project)
            .map(|workspace| workspace.tabs.iter().map(|tab| tab.id.clone()).collect())
            .unwrap_or_default();
        self.close_workspace_queue
            .retain(|id| existing.contains(id));
        let next =
            (!self.close_workspace_queue.is_empty()).then(|| self.close_workspace_queue.remove(0));
        self.close_workspace = next.map(|id| (project.to_owned(), id));
    }
    pub(crate) fn refresh_pane_maps(&mut self, project: &str, dock: &Workspace) {
        let tabs = dock.iter_all_tabs().count();
        let focus = dock.main_surface().focused_leaf();
        let fresh = self.pane_index.as_ref().is_none_or(|cached| {
            cached.project != project
                || cached.group != dock.active
                || cached.tabs != tabs
                || cached.focus != focus
        });
        if fresh {
            self.pane_index = Some(PaneIndex {
                project: project.to_owned(),
                group: dock.active.clone(),
                tabs,
                focus,
            });
            self.pane_by_tab = dock
                .iter_all_tabs()
                .map(|(path, tab)| (tab.key(), path.node_path()))
                .collect();
            self.pane_tabs = self
                .pane_by_tab
                .values()
                .map(|path| {
                    (
                        *path,
                        dock.leaf(*path)
                            .map(|leaf| leaf.tabs.clone())
                            .unwrap_or_default(),
                    )
                })
                .collect();
        }
    }
    pub(crate) fn editor_target(
        &self,
        project: &str,
        origin: Option<&Tab>,
        split: Option<&str>,
    ) -> After {
        let mut anchors = Vec::new();

        if let Some(dock) = self.layouts.get(project) {
            let path = origin
                .and_then(|tab| dock.find_tab(tab).map(egui_dock::TabPath::node_path))
                .or_else(|| {
                    dock.main_surface()
                        .focused_leaf()
                        .map(|node| egui_dock::NodePath {
                            surface: egui_dock::SurfaceIndex::main(),
                            node,
                        })
                });
            if let Some(path) = path
                && let Ok(leaf) = dock.leaf(path)
            {
                if let Some(tab) = leaf.tabs.get(leaf.active.0) {
                    anchors.push(tab.clone());
                }
                anchors.extend(
                    leaf.tabs
                        .iter()
                        .filter(|tab| !anchors.contains(tab))
                        .cloned()
                        .collect::<Vec<_>>(),
                );
            }
        } else if self.selected.as_deref() == Some(project)
            && let Some(origin) = origin
            && let Some(path) = self.pane_by_tab.get(&origin.key())
        {
            anchors.push(origin.clone());
            anchors.extend(
                self.pane_tabs
                    .get(path)
                    .into_iter()
                    .flatten()
                    .filter(|tab| *tab != origin)
                    .cloned(),
            );
        }
        if let Some(split) = split {
            After::CreateAt(anchors, Some(split.into()))
        } else {
            After::Workspace(id(), anchors)
        }
    }
    pub(crate) fn begin_rename(&mut self, sid: &str, surface: RenameSurface) {
        if let Some(session) = self.state.sessions.iter().find(|s| s.id == sid) {
            self.rename_session = Some((sid.into(), session.label.clone()));
            self.rename_focus = true;
            self.rename_surface = surface;
            self.rename_seen = false;
            self.rename_painted = false;
            self.rename_missed = false;
            self.rename_field_id = None;
        }
    }
    /// Terminals, shortcuts, and browser panes stay live unless the rename
    /// field is actually going to paint. The field strips keys only once it
    /// paints, and the IDE strip paints first, so a visible field still has
    /// to block input up front.
    pub(crate) fn rename_blocks_input(&self) -> bool {
        self.rename_session.is_some() && self.rename_field_can_show() && !self.rename_missed
    }
    pub(crate) fn rename_field_can_show(&self) -> bool {
        let Some((sid, _)) = &self.rename_session else {
            return false;
        };
        let sid = sid.clone();
        match self.rename_surface {
            // The workspace tab strip stays up over center panes. A tab that
            // scrolls out of view is caught by `rename_missed`.
            RenameSurface::Workspace => true,
            RenameSurface::Pane => self.pane_rename_can_show(&sid),
            RenameSurface::Sidebar => self.sidebar_rename_can_show(&sid),
        }
    }
    pub(crate) fn pane_rename_can_show(&self, sid: &str) -> bool {
        let Some(project_id) = self
            .state
            .sessions
            .iter()
            .find(|session| session.id == sid)
            .map(|session| session.project_id.clone())
        else {
            return false;
        };
        if self.is_strip_session(&project_id, sid) {
            return self.preferences.ide_mode && !self.preferences.ide_terminal_collapsed;
        }
        !self.center_covers_workspace()
    }
    pub(crate) fn center_covers_workspace(&self) -> bool {
        self.settings_open
            || self.player_open
            || self.palette_open
            || self.worktree_open
            || self.worktree_draft.is_some()
            || self.search_open
    }
    pub(crate) fn sidebar_rename_can_show(&self, sid: &str) -> bool {
        let Some((project_id, label, live, kind)) = self
            .state
            .sessions
            .iter()
            .find(|session| session.id == sid)
            .map(|session| {
                (
                    session.project_id.clone(),
                    session.label.clone(),
                    session.lifecycle.live(),
                    session.kind.clone(),
                )
            })
        else {
            return false;
        };
        if live && kind != SessionKind::Editor {
            return self.live_session_row_can_show(&project_id);
        }
        self.history_session_row_can_show(sid, &project_id, &label, live)
    }
    pub(crate) fn live_session_row_can_show(&self, project_id: &str) -> bool {
        self.preferences.left_visible
            && !self.preferences.left_agents
            && self.preferences.expanded.get(project_id) != Some(&false)
            && !self.preferences.hidden_projects.contains(project_id)
    }
    pub(crate) fn history_session_row_can_show(
        &self,
        session_id: &str,
        project_id: &str,
        label: &str,
        live: bool,
    ) -> bool {
        if !self.preferences.visible || self.preferences.tool != SidebarTool::History {
            return false;
        }
        if live || !self.state.session_has_resume(session_id) {
            return false;
        }
        if self.preferences.history_expanded.get(project_id) == Some(&false) {
            return false;
        }
        let query = self.preferences.history_filter.trim().to_lowercase();
        if query.is_empty() {
            return true;
        }
        let project_name = self
            .state
            .projects
            .iter()
            .find(|project| project.id == project_id)
            .map(|project| project.name.to_lowercase())
            .unwrap_or_default();
        project_name.contains(&query) || label.to_lowercase().contains(&query)
    }
    /// A hidden field must not keep egui focus, or the terminal will not take
    /// keys. The draft stays; the next paint focuses the field instead of
    /// treating the gap as a blur commit.
    pub(crate) fn suspend_hidden_rename(&mut self, ctx: &egui::Context) {
        if self.rename_session.is_none() {
            self.rename_seen = false;
            self.rename_painted = false;
            self.rename_missed = false;
            self.rename_field_id = None;
            return;
        }
        if self.rename_field_can_show() {
            return;
        }
        self.rename_missed = false;
        if let Some(id) = self.rename_field_id.take() {
            ctx.memory_mut(|memory| memory.surrender_focus(id));
        }
        self.rename_focus = true;
    }
    pub(crate) fn note_rename_frame(&mut self, ctx: &egui::Context) {
        if self.rename_session.is_none() {
            return;
        }
        if !self.rename_seen {
            self.rename_seen = true;
            self.rename_painted = false;
            return;
        }
        if self.rename_painted {
            self.rename_missed = false;
        } else if self.rename_field_can_show() {
            self.rename_missed = true;
            if let Some(id) = self.rename_field_id.take() {
                ctx.memory_mut(|memory| memory.surrender_focus(id));
            }
            self.rename_focus = true;
        }
        self.rename_painted = false;
    }
    pub(crate) fn finish_rename(&mut self, save: bool) {
        if let Some((sid, title)) = self.rename_session.take()
            && save
            && !title.trim().is_empty()
            && title.trim().len() <= 256
        {
            self.send(Request::Rename {
                session: sid,
                label: title.trim().into(),
            });
        }
    }
    pub(crate) fn renaming(&self, sid: &str, surface: RenameSurface) -> bool {
        self.rename_surface == surface
            && self
                .rename_session
                .as_ref()
                .is_some_and(|(target, _)| target == sid)
    }
}
