use super::*;

impl App {
    pub(crate) fn open_new_browser_tab(&mut self, split: Option<&str>) {
        let Some(project) = self.selected.clone() else {
            return;
        };
        let _ = self.open_browser_url(&project, "https://example.com", split);
    }

    pub(crate) fn open_new_vnc_tab(&mut self, split: Option<&str>) {
        self.hide_center_overlay();
        let Some(project) = self.selected.clone() else {
            return;
        };
        let origin = self
            .active_session
            .as_ref()
            .map(|sid| Tab::Terminal(sid.clone()));
        let after = self.editor_target(&project, origin.as_ref(), split);
        let tab = Tab::Vnc {
            id: id(),
            session: String::new(),
            profile_id: None,
            host: String::new(),
            port: DEFAULT_CONNECTION_PORT,
            label: "VNC Viewer".into(),
        };
        self.place_gui_tab(project, tab, after);
    }

    pub(crate) fn connect_vnc_profile(&mut self, profile: &VncProfile, split: Option<&str>) {
        self.hide_center_overlay();
        let Some(project) = self.selected.clone() else {
            return;
        };
        let origin = self
            .active_session
            .as_ref()
            .map(|sid| Tab::Terminal(sid.clone()));
        let after = self.editor_target(&project, origin.as_ref(), split);
        if let Some(error) =
            self.vnc_options_unsupported(&profile.options, self.vnc_features_supported())
        {
            self.error = Some(error.into());
            return;
        }
        if profile.password.is_empty() && !profile.credential_saved {
            let tab_id = id();
            let view = self.vnc_views.entry(format!("draft:{tab_id}")).or_default();
            view.draft_host.clone_from(&profile.host);
            view.draft_port = profile.port.to_string();
            view.draft_display = display_for_port(&view.draft_port);
            view.draft_username.clone_from(&profile.username);
            view.draft_name = profile.display_name();
            view.draft_options = profile.options.clone();
            view.draft_shared = profile.shared;
            view.draft_view_only = profile.view_only;
            view.save_profile = false;
            let tab = Tab::Vnc {
                id: tab_id,
                session: String::new(),
                profile_id: Some(profile.id.clone()),
                host: profile.host.clone(),
                port: profile.port,
                label: profile.display_name(),
            };
            self.place_gui_tab(project, tab, after);
            return;
        }
        let _ = self.jobs.send(Job::rpc(
            Request::VncConnect {
                options: profile.options.clone(),
                project,
                profile_id: Some(profile.id.clone()),
                host: profile.host.clone(),
                port: profile.port,
                username: profile.username.clone(),
                password: profile.password.clone(),
                shared: profile.shared,
                view_only: profile.view_only,
                label: Some(profile.display_name()),
            },
            after,
        ));
    }

    /// Drop view state for sessions and drafts that no open tab uses. Runs
    /// after the frame is laid out, so a tab that closed this frame loses
    /// its texture instead of keeping it alive forever.
    pub(crate) fn prune_vnc_views(&mut self) {
        let mut live: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut collect = |tab: &Tab| {
            if let Tab::Vnc { id, session, .. } = tab {
                if session.is_empty() {
                    live.insert(format!("draft:{id}"));
                } else {
                    live.insert(session.clone());
                }
            }
        };
        for workspace in self.layouts.values() {
            for tab in &workspace.tabs {
                for (_, tab) in tab.layout.iter_all_tabs() {
                    collect(tab);
                }
            }
        }
        for window in &self.floating {
            let Some(dock) = window.dock.as_ref() else {
                continue;
            };
            for (_, tab) in dock.iter_all_tabs() {
                collect(tab);
            }
        }
        live.insert(page::PAGE_KEY.to_owned());
        // Views painted this frame are kept even if their tab is mid-move:
        // a transient dock check-out must not drop a live texture.
        live.extend(self.visible_vnc.iter().cloned());
        self.vnc_views.retain(|key, _| live.contains(key));
    }

    /// True when any open tab (docked or floating) still shows `session`.
    pub(crate) fn vnc_session_has_tab(&self, session: &str) -> bool {
        let docked = self.layouts.values().any(|workspace| {
            workspace.tabs.iter().any(|tab| {
                tab.layout
                    .iter_all_tabs()
                    .any(|(_, tab)| matches!(tab, Tab::Vnc { session: sid, .. } if sid == session))
            })
        });
        if docked {
            return true;
        }
        self.floating.iter().any(|window| {
            window.dock.as_ref().is_some_and(|dock| {
                dock.iter_all_tabs()
                    .any(|(_, tab)| matches!(tab, Tab::Vnc { session: sid, .. } if sid == session))
            })
        })
    }

    pub(crate) fn focus_or_open_vnc_session(&mut self, session: &VncSession) {
        let project = if !session.project_id.is_empty() {
            session.project_id.clone()
        } else if let Some(selected) = &self.selected {
            selected.clone()
        } else {
            return;
        };
        if let Some(workspace) = self.layouts.get_mut(&project) {
            let existing = workspace
                .tabs
                .iter()
                .flat_map(|group| group.layout.iter_all_tabs())
                .map(|(_, tab)| tab.clone())
                .find(|tab| matches!(tab, Tab::Vnc { session: sid, .. } if sid == &session.id));
            if let Some(tab) = existing {
                workspace.activate_containing(&tab);
                if let Some(path) = workspace.find_tab(&tab) {
                    let _ = workspace.set_active_tab(path);
                    workspace.set_focused_node_and_surface(path.node_path());
                }
                self.select_project(project);
                self.active_session = None;
                self.non_terminal_selected = true;
                self.focus_tab = Some(tab);
                return;
            }
        }
        let tab = Tab::Vnc {
            id: id(),
            session: session.id.clone(),
            profile_id: session.profile_id.clone(),
            host: session.host.clone(),
            port: session.port,
            label: session.label.clone(),
        };
        self.select_project(project.clone());
        self.place_gui_tab(project, tab, After::Workspace(id(), Vec::new()));
    }
}
