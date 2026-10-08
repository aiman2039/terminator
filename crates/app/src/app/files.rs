use anyhow::Result;
use std::{collections::HashSet, path::PathBuf};
use terminator_core::*;

use super::super::*;
impl App {
    pub(crate) fn open_file(
        &mut self,
        path: PathBuf,
        line: Option<u32>,
        split: Option<&str>,
        external: bool,
    ) {
        self.open_file_mode(path, line, split, external, false);
    }
    pub(crate) fn open_file_mode(
        &mut self,
        path: PathBuf,
        line: Option<u32>,
        split: Option<&str>,
        external: bool,
        text: bool,
    ) {
        if !external && !text && image_preview::supported(&path) {
            if let Some(project) = self.selected.clone() {
                self.open_image(&project, path, split);
            }
            return;
        }
        if !external && !text && crate::browser::supported_file(&path) {
            if let Some(project) = self.selected.clone() {
                self.open_html(&project, path, split);
            }
            return;
        }
        if !external && !text && player::supported(&path) {
            if let Some(project) = self.selected.clone() {
                self.open_audio(&project, path, split);
            }
            return;
        }
        if external || self.state.settings.editor_mode == EditorMode::External {
            let _ = self.jobs.send(Job::External(path));
            return;
        }
        if self.state.settings.editor_mode == EditorMode::Native
            && let Some(project) = self.selected.clone()
        {
            self.open_native(&project, path, line, split);
            return;
        }
        self.hide_center_overlay();
        if let Some(project) = &self.selected {
            let _ = self.jobs.send(Job::rpc(
                Request::Create {
                    project: project.clone(),
                    cwd: self.cwd(),
                    file: Some(path),
                    line,
                    column: None,
                    editor: true,
                },
                self.editor_target(project, None, split),
            ));
        }
    }
    pub(crate) fn open_image(&mut self, project: &str, path: PathBuf, split: Option<&str>) {
        self.hide_center_overlay();
        let origin = self
            .active_session
            .as_ref()
            .map(|sid| Tab::Terminal(sid.clone()));
        let after = self.editor_target(project, origin.as_ref(), split);
        let _ = self.update_tx.send(Update::OpenImage(
            project.into(),
            std::path::absolute(&path).unwrap_or(path),
            after,
        ));
    }
    pub(crate) fn open_html(&mut self, project: &str, path: PathBuf, split: Option<&str>) {
        self.hide_center_overlay();
        let origin = self
            .active_session
            .as_ref()
            .map(|sid| Tab::Terminal(sid.clone()));
        let after = self.editor_target(project, origin.as_ref(), split);
        let Tab::Browser { target, .. } =
            Tab::browser_file(std::path::absolute(&path).unwrap_or(path))
        else {
            return;
        };
        let _ = self
            .update_tx
            .send(Update::OpenBrowser(project.into(), target, after));
    }
    pub(crate) fn browser_covered(&self) -> bool {
        self.settings_open
            || self.player_open
            || self.command_dialog_open()
            || self.picker_active
            || self.close_session.is_some()
            || self.close_workspace.is_some()
            || self.notice_detail_modal_open()
            || self.open_path
            || self.add_project
            || self.rename_blocks_input()
    }

    pub(crate) fn sync_browsers(&mut self, frame: &eframe::Frame) {
        self.browser_host.sync(browser_host::SyncInput {
            frame,
            visible: &self.visible_browsers,
            occluded: self.browser_covered(),
            data_dir: &self.paths.data,
        });
        for (key, url) in self.browser_host.take_opens() {
            let project = self
                .layouts
                .iter()
                .find(|(_, workspace)| {
                    workspace.tabs.iter().any(|group| {
                        group
                            .layout
                            .iter_all_tabs()
                            .any(|(_, tab)| tab.key() == key)
                    })
                })
                .map(|(project, _)| project.clone());
            if let Some(project) = project {
                let _ = self.open_browser_url(&project, &url, None);
            }
        }
    }

    pub(crate) fn open_browser_url(
        &mut self,
        project: &str,
        url: &str,
        split: Option<&str>,
    ) -> Result<()> {
        self.hide_center_overlay();
        let origin = self
            .active_session
            .as_ref()
            .map(|sid| Tab::Terminal(sid.clone()));
        let after = self.editor_target(project, origin.as_ref(), split);
        self.update_tx
            .send(Update::OpenBrowser(
                project.into(),
                BrowserTarget::from_http_url(url)?,
                after,
            ))
            .map_err(|_| anyhow::anyhow!("Browser open queue closed"))?;
        Ok(())
    }
    pub(crate) fn place_gui_tab(&mut self, project: String, tab: Tab, after: After) {
        if let After::CreateAt(anchors, direction) = after {
            let previous = self.layouts.get(&project).map(|d| d.active.clone());
            if let Some(dock) = self.layouts.get_mut(&project) {
                if let Some(anchor) = anchors.iter().find(|t| dock.contains(t)) {
                    dock.activate_containing(anchor);
                }
                if let Some(path) = anchors.iter().find_map(|t| dock.find_tab(t)) {
                    dock.set_focused_node_and_surface(path.node_path());
                }
            }
            let same = previous.as_ref() == self.layouts.get(&project).map(|d| &d.active);
            self.insert(&project, tab, direction.as_deref());
            if !same
                && let Some(previous) = previous
                && let Some(dock) = self.layouts.get_mut(&project)
            {
                dock.active = previous;
            }
            if same && self.selected.as_deref() == Some(&project) {
                self.active_session = None;
            }
        } else {
            self.layouts
                .entry(project.clone())
                .or_insert_with(Workspace::empty)
                .add(id(), tab);
            if self.selected.as_deref() == Some(&project) {
                self.active_session = None;
            }
        }
    }
    pub(crate) fn apply_browser_submit(&mut self) {
        if let Some((key, target)) = self.browser_submit.take() {
            if self.browser_host.navigate(&key, &target) {
                return;
            }
            // No mounted view (e.g. unsupported platform): persist the requested target.
            self.apply_browser_navigation(&key, target);
        }
    }

    pub(crate) fn apply_browser_navigation(&mut self, key: &str, target: BrowserTarget) {
        for workspace in self.layouts.values_mut() {
            for group in &mut workspace.tabs {
                for tab in group
                    .primary
                    .iter_mut()
                    .chain(group.layout.iter_all_tabs_mut().map(|(_, tab)| tab))
                {
                    if tab.key() == key
                        && let Tab::Browser {
                            target: current, ..
                        } = tab
                    {
                        *current = target.clone();
                    }
                }
            }
        }
        self.browser_urls.insert(
            key.into(),
            crate::browser::href(&target).unwrap_or_default(),
        );
        self.browser_host.committed(key, target);
    }

    pub(crate) fn reconcile_gui_resources(&mut self) {
        let mut browsers = HashSet::new();
        for workspace in self.layouts.values() {
            for group in &workspace.tabs {
                for (_, tab) in group.layout.iter_all_tabs() {
                    if let Tab::Browser { .. } = tab {
                        browsers.insert(tab.key());
                    }
                }
            }
        }
        self.browser_host.retain(&browsers);
        self.browser_urls.retain(|key, _| browsers.contains(key));
        self.visible_browsers
            .retain(|pane| browsers.contains(&pane.key));
        // Safe to run per frame: `strip_player` returns early when a workspace
        // holds no Player pane, so it never rebuilds or re-identifies one.
        for workspace in self.layouts.values_mut() {
            workspace.strip_player();
        }
    }
    pub(crate) fn hide_center_overlay(&mut self) {
        if self.settings_open && self.settings_dirty() {
            self.settings_pending = Some(settings_ui::SettingsPending::Close);
        }
        self.settings_open = false;
        self.player_open = false;
        self.close_scrollback_search();
        self.worktree_open = false;
        self.shortcut_capture = None;
    }

    /// Drop the scrollback-search pane. `search_session` only names the query;
    /// leaving it set used to keep every main terminal disabled after the pane
    /// was gone.
    pub(crate) fn close_scrollback_search(&mut self) {
        self.search_open = false;
        self.search_session = None;
    }
}
