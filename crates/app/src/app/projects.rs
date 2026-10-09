use egui_dock::NodeIndex;
use std::{collections::HashSet, path::PathBuf};
use terminator_core::*;

use super::super::*;
impl App {
    pub(crate) fn select_project(&mut self, project: String) {
        self.apply_project_selection(project, false);
    }
    pub(crate) fn reveal_project(&mut self, project: String) {
        self.apply_project_selection(project, true);
    }
    pub(crate) fn apply_project_selection(&mut self, project: String, force_activity: bool) {
        if !self.project_available(&project) {
            return;
        }
        let restored = self.preferences.hidden_projects.remove(&project);
        if restored || force_activity {
            self.touch_project_activity(&project);
        }
        if self.selected.as_ref() != Some(&project) {
            self.finish_rename(true);
        }
        self.selection_generation = self.selection_generation.wrapping_add(1);
        self.selected = Some(project.clone());
        self.active_session = self
            .layouts
            .get_mut(&project)
            .and_then(|d| d.main_surface_mut().find_active_focused())
            .and_then(|(_, tab)| match tab {
                Tab::Terminal(id) => Some(id.clone()),
                _ => None,
            });
        self.send(Request::SelectProject { project });
    }
    pub(crate) fn touch_project_activity(&mut self, project: &str) {
        self.preferences
            .project_activity
            .insert(project.to_string(), now());
    }
    pub(crate) fn hide_project(&mut self, project: &str) {
        if !self.state.projects.iter().any(|p| p.id == project) {
            return;
        }
        self.preferences.hidden_projects.insert(project.into());
        // Also invalidate an outstanding folder-picker result, including when
        // a background project was removed through its context menu.
        self.selection_generation = self.selection_generation.wrapping_add(1);
        if self.selected.as_deref() == Some(project) {
            self.finish_rename(true);
            self.selected = None;
            self.active_session = None;
            if let Some(next) = self
                .visible_projects()
                .into_iter()
                .next()
                .map(|p| p.id.clone())
            {
                self.select_project(next);
            }
        }
        self.info = Some("Project removed from the sidebar. Add the folder again to restore it; its files and sessions are kept.".into());
    }
    /// Split marker that stacks the new tab into the focused leaf instead
    /// of opening a side split or a new top-level tab. It travels the
    /// existing `Option<&str>` split plumbing untouched.
    pub(crate) const SPLIT_HERE: &str = "here";

    /// Maps a file action to its split routing. `Here` stacks into the
    /// focused leaf; `Split` opens a side split; anything else opens a
    /// new top-level tab.
    pub(crate) fn split_for(action: FileAction) -> Option<&'static str> {
        match action {
            FileAction::Split => Some("right"),
            FileAction::Here => Some(Self::SPLIT_HERE),
            _ => None,
        }
    }

    pub(crate) fn insert(&mut self, project: &str, tab: Tab, split: Option<&str>) {
        let dock = self
            .layouts
            .entry(project.into())
            .or_insert_with(Workspace::empty);
        dock.version = dock.version.max(tab.layout_version());
        if let Some(path) = dock.find_tab(&tab) {
            let _ = dock.set_active_tab(path);
            dock.set_focused_node_and_surface(path.node_path());
            return;
        }
        if let Some(direction) = split {
            if direction == Self::SPLIT_HERE {
                dock.push_to_focused_leaf(tab);
                return;
            }
            let tree = dock.main_surface_mut();
            if !tree.is_empty() {
                let node = tree.focused_leaf().unwrap_or(NodeIndex::root());
                let result = match direction {
                    "left" => tree.split_left(node, 0.5, vec![tab]),
                    "up" => tree.split_above(node, 0.5, vec![tab]),
                    "down" => tree.split_below(node, 0.5, vec![tab]),
                    _ => tree.split_right(node, 0.5, vec![tab]),
                };
                tree.set_focused_node(result[1]);
                return;
            }
        }
        dock.push_to_focused_leaf(tab);
    }
    /// Place a tab into a floating window's dock: stacked for plain
    /// tabs, beside the anchored leaf for splits. Returns false when the
    /// window is gone, so callers can fall back to the project dock.
    pub(crate) fn insert_float(
        &mut self,
        viewport: egui::ViewportId,
        tab: Tab,
        split: Option<&str>,
        anchors: &[Tab],
    ) -> bool {
        let Some(window) = self
            .floating
            .iter_mut()
            .find(|window| window.viewport == viewport)
        else {
            return false;
        };

        let Some(dock) = window.dock.as_mut() else {
            return false;
        };
        if let Some(path) = dock.find_tab(&tab) {
            let _ = dock.set_active_tab(path);
            dock.set_focused_node_and_surface(path.node_path());
            return true;
        }
        // Focus the issuing leaf first so the split lands next to it,
        // mirroring the main-dock completion.
        if let Some(path) = anchors.iter().find_map(|anchor| dock.find_tab(anchor)) {
            dock.set_focused_node_and_surface(path.node_path());
        }
        if let Some(direction) = split {
            let tree = dock.main_surface_mut();
            if !tree.is_empty() {
                let node = tree.focused_leaf().unwrap_or(NodeIndex::root());
                let result = match direction {
                    "left" => tree.split_left(node, 0.5, vec![tab]),
                    "up" => tree.split_above(node, 0.5, vec![tab]),
                    "down" => tree.split_below(node, 0.5, vec![tab]),
                    _ => tree.split_right(node, 0.5, vec![tab]),
                };
                tree.set_focused_node(result[1]);
                return true;
            }
        }
        dock.push_to_focused_leaf(tab);
        true
    }
    pub(crate) fn selected_project(&self) -> Option<&Project> {
        self.state
            .projects
            .iter()
            .find(|p| Some(&p.id) == self.selected.as_ref() && self.project_available(&p.id))
    }
    pub(crate) fn project_available(&self, project: &str) -> bool {
        self.state.projects.iter().any(|p| p.id == project)
            && !self
                .state
                .worktrees
                .iter()
                .any(|w| w.project_id == project && w.removed)
            && (!self.missing_projects.contains(project)
                || self
                    .state
                    .sessions
                    .iter()
                    .any(|s| s.project_id == project && s.lifecycle.live()))
    }

    pub(crate) fn apply_project_directories(&mut self, directories: Vec<(String, PathBuf, bool)>) {
        for (project, path, available) in directories {
            // A delayed read cannot hide a project that was opened at a new path.
            if !self
                .state
                .projects
                .iter()
                .any(|p| p.id == project && p.path == path)
            {
                continue;
            }
            if available {
                self.missing_projects.remove(&project);
            } else {
                self.missing_projects.insert(project.clone());
                self.terminal_context.remove(&project);
            }
        }
        if self
            .selected
            .as_ref()
            .is_some_and(|project| !self.project_available(project))
        {
            self.selection_generation = self.selection_generation.wrapping_add(1);
            self.selected = None;
            self.active_session = None;
            if let Some(next) = self.visible_projects().first().map(|p| p.id.clone()) {
                self.select_project(next);
            }
        }
    }
    pub(crate) fn context_session(&self) -> Option<&Session> {
        self.state
            .sessions
            .iter()
            .find(|s| {
                Some(&s.id) == self.active_session.as_ref()
                    && s.kind == SessionKind::Shell
                    && s.lifecycle.live()
                    && Some(&s.project_id) == self.selected.as_ref()
            })
            .or_else(|| {
                self.selected
                    .as_ref()
                    .and_then(|p| self.terminal_context.get(p))
                    .and_then(|id| {
                        self.state.sessions.iter().find(|s| {
                            &s.id == id
                                && s.lifecycle.live()
                                && s.kind == SessionKind::Shell
                                && Some(&s.project_id) == self.selected.as_ref()
                        })
                    })
            })
            .filter(|s| self.project_available(&s.project_id))
    }
    pub(crate) fn cwd(&self) -> Option<PathBuf> {
        self.context_session()
            .map(|s| s.cwd.clone())
            .or_else(|| self.selected_project().map(|p| p.path.clone()))
    }
    pub(crate) fn sync_resource_sample(&mut self) {
        // Always sample: the status strip shows GUI/daemon/hook totals from
        // every sample, and Info adds the focused session when it is open.
        // The background cadence stays at one sample every few seconds;
        // only which session pid to follow changes.
        let next = Some({
            let session = self
                .context_session()
                .filter(|session| session.lifecycle.live());
            resource_sample::Request {
                pid: session.and_then(|session| session.pid),
                started: session.map(|session| session.created).unwrap_or(0),
            }
        });
        if next != self.resource_request {
            self.resource_request.clone_from(&next);
            let _ = self.resource_tx.send(next);
        }
    }
    pub(crate) fn report_status_error(&mut self, error: String) {
        let cwd = self.cwd();
        if self.error_cwd.as_deref() != cwd.as_deref() {
            self.error_cwd = cwd;
            self.dismissed_error = None;
            self.missing_path_reports = 0;
        }
        if self.dismissed_error.as_deref() == Some(error.as_str()) {
            return;
        }
        if is_missing_path_error(&error) {
            if self.error.as_deref() != Some(error.as_str()) {
                if self.missing_path_reports >= retry_budget::MISSING_PATH_RETRY_LIMIT {
                    return;
                }
                self.missing_path_reports = self.missing_path_reports.saturating_add(1);
            }
        } else if self.error.as_deref() != Some(error.as_str()) {
            self.dismissed_error = None;
            self.missing_path_reports = 0;
        }
        self.error = Some(error);
    }
    pub(crate) fn dismiss_status_error(&mut self) {
        self.dismissed_error = self.error.take();
    }
    pub(crate) fn dialog_directory(&self) -> PathBuf {
        self.cwd()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| "/".into())
    }

    pub(crate) fn go_session(&mut self, sid: &str) {
        self.hide_center_overlay();
        self.finish_rename(true);
        if let Some(s) = self.state.sessions.iter().find(|s| s.id == sid).cloned() {
            if s.kind == SessionKind::Shell && self.is_strip_session(&s.project_id, sid) {
                if self.preferences.ide_mode {
                    self.select_project(s.project_id.clone());
                    self.preferences.ide_terminal_collapsed = false;
                    self.activate_strip_session(&s.project_id, &s.id);
                    return;
                }
                // Regular mode uses the same shell in the main dock.
                if self.move_project_strip_to_main(&s.project_id) {
                    self.save_layouts();
                }
            }
            self.select_project(s.project_id.clone());
            let pane = Tab::Terminal(sid.into());
            // A session already visible in a floating window stays there:
            // adopting it into the main dock as well would duplicate it.
            let floated = self.floating.iter().any(|window| {
                window
                    .dock
                    .as_ref()
                    .is_some_and(|dock| dock.find_tab(&pane).is_some())
            });
            if !floated {
                let workspace = self
                    .layouts
                    .entry(s.project_id.clone())
                    .or_insert_with(Workspace::empty);
                if !workspace.activate_containing(&pane) {
                    workspace.add(id(), pane.clone());
                }
                self.insert(&s.project_id, pane, None);
            }
            self.active_session = Some(s.id.clone());
            self.send(Request::SelectProject {
                project: s.project_id,
            });
            self.send(Request::Focus { session: s.id });
        }
    }
    pub(crate) fn has_project(&self, project: &str) -> bool {
        self.state.projects.iter().any(|p| p.id == project)
    }
    pub(crate) fn can_persist_layout(&self, project: &str) -> bool {
        !self.layout_readonly.contains(project) && self.has_project(project)
    }
    pub(crate) fn persistable_layouts(&self) -> Vec<(String, Workspace)> {
        self.layouts
            .iter()
            .filter(|(project, _)| self.can_persist_layout(project))
            .map(|(project, layout)| (project.clone(), layout.clone()))
            .collect()
    }
    pub(crate) fn reconcile_project_inventory(&mut self, projects: &[Project]) {
        let known: HashSet<&str> = projects.iter().map(|project| project.id.as_str()).collect();
        self.missing_projects
            .retain(|id| known.contains(id.as_str()));
        self.layouts.retain(|id, _| known.contains(id.as_str()));
        self.layout_saved
            .retain(|id, _| known.contains(id.as_str()));
        self.layout_pending
            .retain(|id, _| known.contains(id.as_str()));
        if let Some(signature) = &mut self.layout_signature {
            signature.retain(|(id, _)| known.contains(id.as_str()));
        }
        self.layout_readonly
            .retain(|id| known.contains(id.as_str()));
        if self
            .selected
            .as_ref()
            .is_some_and(|id| !known.contains(id.as_str()))
        {
            self.selected = None;
            self.active_session = None;
        }
    }
    pub(crate) fn layout_save_failed(&mut self, error: String) {
        if error.contains("Unknown project") {
            return;
        }
        if self.exit.active() {
            self.cancel_exit(error);
        } else {
            self.report_status_error(error);
        }
    }
    pub(crate) fn save_layouts(&mut self) {
        let mut signature: Vec<(String, String)> = self
            .layouts
            .iter()
            .filter(|(project, _)| self.can_persist_layout(project))
            .map(|(project, layout)| {
                let text = terminator_core::sanitize_layout(
                    serde_json::to_value(layout).unwrap_or(serde_json::Value::Null),
                )
                .to_string();
                (project.clone(), text)
            })
            .collect();
        signature.sort_by(|a, b| a.0.cmp(&b.0));
        if self.layout_signature.as_ref() == Some(&signature) {
            return;
        }
        self.layout_signature = Some(signature);
        let layouts = self.persistable_layouts();
        self.layout_generation = self.layout_generation.wrapping_add(1);
        let _ = self
            .jobs
            .send(Job::PrepareLayouts(self.layout_generation, layouts));
    }
}
