use std::collections::HashSet;
use terminator_core::*;

use super::super::*;

impl Tab {
    /// Tab title for revision diffs, file history and merges.
    pub(crate) fn git_title(&self) -> String {
        let name = |path: &std::path::Path| {
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        };
        let short = |revision: &str| -> String {
            if revision.is_empty() {
                "local".into()
            } else if revision.trim_end_matches('^').len() >= 12 {
                let suffix = if revision.ends_with('^') { "^" } else { "" };
                format!("{}{suffix}", revision.chars().take(7).collect::<String>())
            } else {
                revision.to_owned()
            }
        };
        match self {
            Self::RevisionDiff {
                path, left, right, ..
            } => format!("{} ({} ↔ {})", name(path), short(left), short(right)),
            Self::FileHistory { path, .. } => format!("History {}", name(path)),
            Self::Merge { path, .. } => format!("Merge {}", name(path)),
            Self::ChangesDiff { staged: true, .. } => "Staged changes".into(),
            Self::ChangesDiff { .. } => "All changes".into(),
            _ => String::new(),
        }
    }
}

pub(crate) struct HistorySessionRowCanShowInput<'a> {
    pub(crate) session_id: &'a str,
    pub(crate) project_id: &'a str,
    pub(crate) label: &'a str,
    pub(crate) live: bool,
}

impl App {
    pub(crate) fn create(&mut self, split: Option<&str>) {
        if self.selected_remote_project().is_some() && !self.remote_supported() {
            self.error = Some("Restart the session service to create SSH terminals".into());
            return;
        }
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
        self.create_workspace_tab_in(index, None);
    }
    /// A new terminal tab whose shell starts in `cwd` (the project default
    /// when `None`).
    pub(crate) fn create_workspace_tab_in(&mut self, index: Option<usize>, cwd: Option<PathBuf>) {
        self.create_workspace_tab_with(index, cwd, None);
    }
    /// A new terminal tab that types `post` once its shell shows a prompt.
    pub(crate) fn create_workspace_tab_with(
        &mut self,
        index: Option<usize>,
        cwd: Option<PathBuf>,
        post: Option<PostCreate>,
    ) {
        self.hide_center_overlay();
        let Some(project) = self.selected.clone() else {
            return;
        };
        if self.selected_project().is_none() || self.missing_projects.contains(&project) {
            return;
        }
        self.create_terminal_tab((project, cwd), (index, post));
    }
    pub(crate) fn begin_workspace_close_tabs(&mut self, project: &str, ids: Vec<String>) {
        self.clear_session_rename();
        let mut ids = ids.into_iter();
        let Some(first) = ids.next() else {
            return;
        };
        self.close_workspace_queue = ids.collect();
        self.close_workspace = Some((project.to_owned(), first));
    }
    /// Stop the current workspace close and drop its native prompts, so a
    /// late answer cannot close the tab after all.
    pub(crate) fn abort_workspace_close(&mut self) {
        if let Some((project, tab)) = self.close_workspace.take() {
            let owner = crate::native_editor::CloseOwner::Workspace(project, tab);
            self.native_close_prompts.retain(|r| r.owner != owner);
            self.native_close_after_save.retain(|r| r.owner != owner);
        }
        self.close_workspace_queue.clear();
    }
    pub(crate) fn close_workspace_tab_now(&mut self, project: &str, tab_id: &str) {
        let keys: Vec<_> = self
            .layouts
            .get(project)
            .and_then(|workspace| workspace.tabs.iter().find(|tab| tab.id == tab_id))
            .map(|tab| {
                tab.layout
                    .iter_all_tabs()
                    .map(|(_, tab)| tab.key())
                    .collect()
            })
            .unwrap_or_default();
        if keys.iter().any(|key| self.block_unsaved_diff(Some(key))) {
            self.abort_workspace_close();
            return;
        }
        // Asynchronous editor and idle-close checks finish here after the
        // GUI stayed usable, so a native buffer may have become dirty since
        // `poll_workspace_close` checked it.
        if let Some(path) = self.dirty_native_in_tab(project, tab_id).into_iter().next() {
            // Another close requested meanwhile keeps its place; the
            // prompt still names this tab, so its answer finishes it.
            self.prompt_workspace_close(project, tab_id, path);
            return;
        }
        if let Some(workspace) = self.layouts.get_mut(project) {
            let previous = workspace.active.clone();
            workspace.close(tab_id);
            if self.selected.as_deref() == Some(project) && workspace.active != previous {
                if let Some(Tab::Terminal(sid)) = workspace.active_pane().cloned() {
                    self.focus_terminal(sid);
                } else {
                    self.focus_non_terminal();
                }
            }
        }
        self.forget_tab_expanded(project, tab_id);
        self.prune_native_docs();
        self.prune_diff_docs();
        // An idle close finishing late must not drop a newer close request.
        if self
            .close_workspace
            .as_ref()
            .is_none_or(|(p, t)| p == project && t == tab_id)
        {
            self.advance_workspace_close(project);
        }
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
        self.diff_reviews
            .retain(|key, review| live.contains(key) || review.busy || review.unsaved_edits());
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
            self.pane_tabs = dock
                .iter_leaves()
                .map(|(path, leaf)| (path, leaf.tabs.clone()))
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
            let tab = self.layouts.get(project).map(|dock| dock.active.clone());
            After::CreateAt(tab, anchors, Some(split.into()))
        } else {
            After::Workspace(id(), anchors)
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
        self.history_session_row_can_show(HistorySessionRowCanShowInput {
            session_id: sid,
            project_id: &project_id,
            label: &label,
            live,
        })
    }
    pub(crate) fn live_session_row_can_show(&self, project_id: &str) -> bool {
        self.preferences.left_visible
            && !self.preferences.left_agents
            && self.preferences.expanded.get(project_id) != Some(&false)
            && !self.preferences.hidden_projects.contains(project_id)
    }

    pub(crate) fn history_session_row_can_show(
        &self,
        input: HistorySessionRowCanShowInput<'_>,
    ) -> bool {
        let HistorySessionRowCanShowInput {
            session_id,
            project_id,
            label,
            live,
        } = input;
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
}
