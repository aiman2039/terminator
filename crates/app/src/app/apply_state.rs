use std::collections::HashSet;
use terminator_core::*;

use super::super::*;
impl App {
    pub(crate) fn apply_state(&mut self, mut state: State) {
        if observation_is_stale(&self.state, &state) {
            return;
        }
        // Failed owners can only supply an older on-disk snapshot. Keep their
        // last observed records while publishing the new unavailability status.
        let unavailable: HashSet<_> = state
            .generations
            .iter()
            .filter(|incoming| {
                incoming.error.is_some()
                    && self.state.generations.iter().any(|current| {
                        current.owner.id == incoming.owner.id
                            && current.revision > incoming.revision
                    })
            })
            .map(|health| health.owner.id.clone())
            .collect();
        if !unavailable.is_empty() {
            let sessions: HashSet<_> = state
                .sessions
                .iter()
                .chain(&self.state.sessions)
                .filter(|session| unavailable.contains(&session.generation))
                .map(|session| session.id.clone())
                .collect();
            state
                .sessions
                .retain(|session| !unavailable.contains(&session.generation));
            state.sessions.extend(
                self.state
                    .sessions
                    .iter()
                    .filter(|session| unavailable.contains(&session.generation))
                    .cloned(),
            );
            state
                .agents
                .retain(|agent| !sessions.contains(&agent.session_id));
            state.agents.extend(
                self.state
                    .agents
                    .iter()
                    .filter(|agent| sessions.contains(&agent.session_id))
                    .cloned(),
            );
            state
                .notifications
                .retain(|notice| !sessions.contains(&notice.session_id));
            state.notifications.extend(
                self.state
                    .notifications
                    .iter()
                    .filter(|notice| sessions.contains(&notice.session_id))
                    .cloned(),
            );
            state
                .terminal_notices
                .retain(|notice| !sessions.contains(&notice.session_id));
            state.terminal_notices.extend(
                self.state
                    .terminal_notices
                    .iter()
                    .filter(|notice| sessions.contains(&notice.session_id))
                    .cloned(),
            );
            for incoming in &mut state.generations {
                if unavailable.contains(&incoming.owner.id)
                    && let Some(current) = self
                        .state
                        .generations
                        .iter()
                        .find(|current| current.owner.id == incoming.owner.id)
                {
                    incoming.revision = current.revision;
                    incoming.live_sessions = current.live_sessions;
                    incoming.capabilities.clone_from(&current.capabilities);
                    incoming.helper.clone_from(&current.helper);
                }
            }
            if state.generation == self.state.generation && unavailable.contains(&state.generation)
            {
                state.revision = self.state.revision;
                state.capabilities.clone_from(&self.state.capabilities);
                state.attachment_helper_available = self.state.attachment_helper_available;
                state
                    .attachment_helper_executable
                    .clone_from(&self.state.attachment_helper_executable);
            }
        }
        // An async poll may have begun before a creation/mutation acknowledgment.
        // Never replace a newer owner/catalog observation with that older result.
        // A stale historical owner must not drop an active owner's session exit.
        if self.state_loaded && inventory_is_stale(&self.state, &state) {
            return;
        }
        if self
            .error
            .as_deref()
            .is_some_and(daemon_connection::is_connection_error)
        {
            self.error = None;
        }
        if state.generation != self.state.generation
            || state.attachment_helper_available == Some(true)
        {
            self.installation_error = None;
            if self
                .error
                .as_deref()
                .is_some_and(installation::is_helper_error)
            {
                self.error = None;
            }
        }
        self.connected = true;
        let initial = !self.state_loaded;
        self.state_loaded = true;
        let removed_projects: HashSet<_> = state
            .worktrees
            .iter()
            .filter(|worktree| worktree.removed)
            .map(|worktree| worktree.project_id.as_str())
            .collect();
        self.terminal_context.retain(|project, sid| {
            !removed_projects.contains(project.as_str())
                && state.sessions.iter().any(|session| {
                    &session.id == sid
                        && &session.project_id == project
                        && session.kind == SessionKind::Shell
                        && session.lifecycle.live()
                })
        });
        let ended_sessions: Vec<_> = state
            .sessions
            .iter()
            .filter(|session| {
                session.lifecycle == Lifecycle::Ended
                    && (initial
                        || self
                            .state
                            .sessions
                            .iter()
                            .any(|old| old.id == session.id && old.lifecycle.live()))
            })
            .cloned()
            .collect();
        let available_projects: Vec<_> = state
            .projects
            .iter()
            .filter(|project| !removed_projects.contains(project.id.as_str()))
            .cloned()
            .collect();
        for p in &available_projects {
            if !self.layouts.contains_key(&p.id) {
                let dock = match Workspace::load(p.layout.clone()) {
                    Ok(workspace) => workspace,
                    Err(error) => {
                        self.error = Some(format!(
                            "{}: {error:#}. Layout will not be overwritten.",
                            p.name
                        ));
                        self.layout_readonly.insert(p.id.clone());
                        Workspace::empty()
                    }
                };
                self.layout_saved.insert(p.id.clone(), p.layout.to_string());
                self.layouts.insert(p.id.clone(), dock);
            }
        }
        self.reconcile_project_inventory(&available_projects);
        if self.selected.as_ref().is_some_and(|project| {
            self.missing_projects.contains(project)
                && !state
                    .sessions
                    .iter()
                    .any(|session| &session.project_id == project && session.lifecycle.live())
        }) {
            self.selected = None;
            self.active_session = None;
        }
        for ended in ended_sessions {
            if self
                .rename_session
                .as_ref()
                .is_some_and(|(sid, _)| sid == &ended.id)
            {
                self.rename_session = None;
            }

            #[cfg(feature = "test-support")]
            if std::env::var_os("TERMINATOR_CAPTURE_PATH").is_some() {
                eprintln!("Fixture ended {:?}", ended.kind);
            }
            let was_active = self.active_session.as_ref() == Some(&ended.id);
            let old_group = self
                .layouts
                .get(&ended.project_id)
                .and_then(|workspace| {
                    workspace.tabs.iter().find(|tab| {
                        tab.layout
                            .find_tab(&Tab::Terminal(ended.id.clone()))
                            .is_some()
                    })
                })
                .map(|tab| tab.id.clone());
            let anchors = self.editor_origins.remove(&ended.id).unwrap_or_default();
            self.remove_tab(&ended.id);
            if was_active
                && self.selected.as_ref() == Some(&ended.project_id)
                && let Some(dock) = self.layouts.get_mut(&ended.project_id)
            {
                let live_tab = |tab: &Tab| match tab {
                    Tab::Terminal(sid) => state
                        .sessions
                        .iter()
                        .any(|s| &s.id == sid && s.lifecycle.live()),
                    Tab::Diff { .. }
                    | Tab::Image { .. }
                    | Tab::Browser { .. }
                    | Tab::Player
                    | Tab::NativeEditor { .. }
                    | Tab::CommitLog { .. }
                    | Tab::Blame { .. } => true,
                };
                let survives = old_group
                    .as_ref()
                    .is_some_and(|id| dock.tabs.iter().any(|tab| &tab.id == id));
                if survives {
                    if let Some(group) = old_group {
                        dock.active = group;
                    }
                } else if let Some(tab) = anchors
                    .iter()
                    .find(|tab| live_tab(tab) && dock.contains(tab))
                {
                    dock.activate_containing(tab);
                }
                let target = if survives {
                    anchors
                        .iter()
                        .filter(|tab| live_tab(tab))
                        .find_map(|tab| dock.find_tab(tab))
                } else {
                    None
                }
                .or_else(|| {
                    dock.active_pane()
                        .filter(|tab| live_tab(tab))
                        .and_then(|tab| dock.find_tab(tab))
                })
                .or_else(|| {
                    dock.iter_all_tabs()
                        .find(|(_, tab)| live_tab(tab))
                        .map(|(path, _)| path)
                });
                if let Some(path) = target {
                    let _ = dock.set_active_tab(path);
                    dock.set_focused_node_and_surface(path.node_path());
                    self.active_session = dock
                        .leaf(path.node_path())
                        .ok()
                        .and_then(|leaf| leaf.tabs.get(leaf.active.0))
                        .and_then(|tab| match tab {
                            Tab::Terminal(sid) => Some(sid.clone()),
                            _ => None,
                        });
                }
            }
        }
        if self.selected.is_none() {
            self.selected = state
                .selected_project
                .clone()
                .filter(|id| {
                    !self.preferences.hidden_projects.contains(id)
                        && !removed_projects.contains(id.as_str())
                        && (!self.missing_projects.contains(id)
                            || state
                                .sessions
                                .iter()
                                .any(|s| &s.project_id == id && s.lifecycle.live()))
                        && state.projects.iter().any(|project| &project.id == id)
                })
                .or_else(|| {
                    state
                        .projects
                        .iter()
                        .find(|p| {
                            !self.preferences.hidden_projects.contains(&p.id)
                                && !removed_projects.contains(p.id.as_str())
                                && (!self.missing_projects.contains(&p.id)
                                    || state
                                        .sessions
                                        .iter()
                                        .any(|s| s.project_id == p.id && s.lifecycle.live()))
                        })
                        .map(|p| p.id.clone())
                });
        }
        shortcuts::fill_defaults(&mut state.settings.keybindings);
        self.preferences.markdown_modes.retain(|sid, _| {
            state
                .sessions
                .iter()
                .any(|s| &s.id == sid && markdown::available(s))
        });
        self.sidebar_projects
            .get_mut()
            .retain(|id, _| state.projects.iter().any(|p| &p.id == id));
        self.state = state;
        self.reconcile_presentations();
        self.migrate_attention();

        if self.preferences_writable
            && !self.preferences.typography_migrated
            && !self.migration_requested
        {
            self.migration_requested = true;
            let _ = self.jobs.send(Job::MigrateTypography);
        }
    }
}
