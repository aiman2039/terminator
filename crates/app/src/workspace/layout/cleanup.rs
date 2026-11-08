use super::*;

impl Workspace {
    pub(crate) fn drop_empty_tabs(&mut self) {
        let previous = self.active_index();
        self.tabs
            .retain(|tab| tab.layout.iter_all_tabs().next().is_some());
        self.normalize(previous);
    }

    pub(crate) fn remove_terminal_ids(&mut self, ids: &HashSet<String>) {
        if ids.is_empty() {
            return;
        }
        let hit = self.tabs.iter().any(|tab| {
            tab.layout
                .iter_all_tabs()
                .any(|(_, pane)| matches!(pane, Tab::Terminal(sid) if ids.contains(sid)))
        });
        if !hit {
            return;
        }
        let previous = self.active_index();
        for tab in &mut self.tabs {
            let panes: Vec<Tab> = tab
                .layout
                .iter_all_tabs()
                .filter(|(_, pane)| matches!(pane, Tab::Terminal(sid) if ids.contains(sid)))
                .map(|(_, pane)| pane.clone())
                .collect();
            for pane in panes {
                while let Some(path) = tab.layout.find_tab(&pane) {
                    tab.layout.remove_tab(path);
                }
                if tab.primary.as_ref() == Some(&pane) {
                    tab.primary = tab
                        .layout
                        .iter_all_tabs()
                        .next()
                        .map(|(_, pane)| pane.clone());
                }
            }
        }
        self.tabs
            .retain(|tab| tab.layout.iter_all_tabs().next().is_some());
        self.normalize(previous);
    }

    pub(crate) fn adopt_dock(&mut self, mut layout: DockState<Tab>, activate: bool) -> bool {
        let primary = layout
            .main_surface_mut()
            .find_active_focused()
            .map(|(_, tab)| tab.clone())
            .or_else(|| layout.iter_all_tabs().next().map(|(_, tab)| tab.clone()));
        let Some(primary) = primary else {
            return false;
        };
        self.version = layout
            .iter_all_tabs()
            .map(|(_, tab)| tab.layout_version())
            .fold(self.version, u32::max);
        let active_index = self.active_index();
        let reuse = self
            .tabs
            .get(active_index)
            .is_some_and(|tab| tab.layout.iter_all_tabs().next().is_none());
        if reuse {
            let Some(tab) = self.tabs.get_mut(active_index) else {
                return false;
            };
            tab.layout = layout;
            tab.primary = Some(primary);
            return true;
        }
        let id = terminator_core::id();
        self.tabs.push(WorkspaceTab {
            id: id.clone(),
            name: None,
            pinned: false,
            primary: Some(primary),
            layout,
        });
        if activate {
            self.active = id;
            self.record_active();
        }
        true
    }

    pub fn strip_player(&mut self) {
        if !self.tabs.iter().any(|tab| {
            tab.primary == Some(Tab::Player) || tab.layout.find_tab(&Tab::Player).is_some()
        }) {
            return;
        }
        let previous = self.active_index();
        for tab in &mut self.tabs {
            while let Some(path) = tab.layout.find_tab(&Tab::Player) {
                tab.layout.remove_tab(path);
            }
            if tab.primary == Some(Tab::Player) {
                tab.primary = tab
                    .layout
                    .iter_all_tabs()
                    .next()
                    .map(|(_, pane)| pane.clone());
            }
        }
        self.tabs
            .retain(|tab| tab.layout.iter_all_tabs().next().is_some());
        self.normalize(previous);
    }

    pub fn remove_session(&mut self, sid: &str) {
        let target = Tab::Terminal(sid.into());
        if !self
            .tabs
            .iter()
            .any(|tab| tab.layout.find_tab(&target).is_some())
        {
            return;
        }
        let previous = self.active_index();
        for tab in &mut self.tabs {
            if let Some(path) = tab.layout.find_tab(&Tab::Terminal(sid.into())) {
                tab.layout.remove_tab(path);
                if tab.primary == Some(Tab::Terminal(sid.into())) {
                    tab.primary = tab
                        .layout
                        .iter_all_tabs()
                        .next()
                        .map(|(_, tab)| tab.clone());
                }
            }
        }
        self.tabs
            .retain(|tab| tab.layout.iter_all_tabs().next().is_some());
        self.normalize(previous);
    }

    /// Removes every pane whose [`Tab::key`] is `key`, for example a remote
    /// window that closed on its host.
    pub(crate) fn remove_key(&mut self, key: &str) {
        if !self
            .tabs
            .iter()
            .any(|tab| tab.layout.find_tab_from(|pane| pane.key() == key).is_some())
        {
            return;
        }
        let previous = self.active_index();
        for tab in &mut self.tabs {
            while let Some(path) = tab.layout.find_tab_from(|pane| pane.key() == key) {
                tab.layout.remove_tab(path);
            }
            if tab.primary.as_ref().is_some_and(|pane| pane.key() == key) {
                tab.primary = tab
                    .layout
                    .iter_all_tabs()
                    .next()
                    .map(|(_, pane)| pane.clone());
            }
        }
        self.tabs
            .retain(|tab| tab.layout.iter_all_tabs().next().is_some());
        self.normalize(previous);
    }

    /// Sets the saved title of the remote window tabs with `key`. Returns
    /// whether one changed.
    pub(crate) fn retitle_remote_app(&mut self, key: &str, title: &str) -> bool {
        let mut changed = false;
        for tab in &mut self.tabs {
            let panes = tab
                .layout
                .iter_all_tabs_mut()
                .map(|(_, pane)| pane)
                .chain(tab.primary.as_mut());
            for pane in panes {
                if pane.key() == key
                    && let Tab::RemoteApp { title: saved, .. } = pane
                    && saved != title
                {
                    title.clone_into(saved);
                    changed = true;
                }
            }
        }
        changed
    }

    pub(super) fn normalize(&mut self, previous: usize) {
        if self.tabs.is_empty() {
            // An emptied workspace is rebuilt from scratch. `Self::empty()` mints
            // a fresh top-level id, so callers that normalize every frame (or on
            // every unrelated mutation) would otherwise see a different layout
            // each time and re-persist it. Keep the previous identity instead.
            let id = std::mem::take(&mut self.active);
            let version = self.version;
            *self = Self::empty();
            self.version = version;
            if !id.is_empty() {
                if let Some(tab) = self.tabs.first_mut() {
                    tab.id.clone_from(&id);
                }
                self.active = id;
            }
        } else if !self.tabs.iter().any(|tab| tab.id == self.active) {
            let origin = self
                .return_tabs
                .get(&self.active)
                .and_then(|origin| self.tabs.iter().position(|tab| &tab.id == origin));
            let fallback = origin.unwrap_or_else(|| {
                previous
                    .saturating_sub(1)
                    .min(self.tabs.len().saturating_sub(1))
            });
            if let Some(tab) = self.tabs.get(fallback) {
                self.active.clone_from(&tab.id);
            }
        }
        self.return_tabs
            .retain(|id, _| self.tabs.iter().any(|tab| &tab.id == id));
        self.recent_tabs
            .retain(|id| self.tabs.iter().any(|tab| &tab.id == id));
        self.record_active();
    }
}
