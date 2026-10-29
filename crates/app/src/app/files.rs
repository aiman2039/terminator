use crate::native_editor::OpenNativeInput;
use crate::remote_ui::OpenRemoteFileInput;
use anyhow::Result;
use std::{collections::HashSet, path::PathBuf};
use terminator_core::*;

use super::super::*;

pub(crate) struct OpenFileInput<'a> {
    pub(crate) path: PathBuf,
    pub(crate) line: Option<u32>,
    pub(crate) split: Option<&'a str>,
    pub(crate) external: bool,
}

pub(crate) struct OpenFileModeInput<'a> {
    pub(crate) path: PathBuf,
    pub(crate) line: Option<u32>,
    pub(crate) split: Option<&'a str>,
    pub(crate) external: bool,
    pub(crate) text: bool,
}

fn same_file_view(left: &Tab, right: &Tab) -> bool {
    match (left, right) {
        (Tab::Image { path: left }, Tab::Image { path: right })
        | (Tab::NativeEditor { path: left }, Tab::NativeEditor { path: right }) => {
            left.components().eq(right.components())
        }
        (Tab::Browser { target: left, .. }, Tab::Browser { target: right, .. }) => left == right,
        (
            Tab::Diff {
                cwd: left_root,
                path: left,
                staged: left_staged,
                ..
            },
            Tab::Diff {
                cwd: right_root,
                path: right,
                staged: right_staged,
                ..
            },
        ) => {
            let left = if left.is_absolute() {
                left.clone()
            } else {
                left_root.join(left)
            };
            let right = if right.is_absolute() {
                right.clone()
            } else {
                right_root.join(right)
            };
            left_staged == right_staged
                && left_root.components().eq(right_root.components())
                && left.components().eq(right.components())
        }
        _ => left == right,
    }
}

impl App {
    /// Select the existing pane and its owning window without moving it or
    /// creating a second top-level tab. Explicit split requests bypass this.
    pub(crate) fn focus_file_view(&mut self, project: &str, requested: &Tab) -> bool {
        let docked = self.layouts.get(project).and_then(|workspace| {
            workspace
                .iter_all_tabs()
                .map(|(_, tab)| tab)
                .chain(
                    workspace
                        .tabs
                        .iter()
                        .flat_map(|group| group.layout.iter_all_tabs())
                        .map(|(_, tab)| tab),
                )
                .find(|tab| same_file_view(tab, requested))
                .cloned()
        });
        if let Some(tab) = docked {
            if let Some(workspace) = self.layouts.get_mut(project) {
                if workspace.find_tab(&tab).is_none() {
                    workspace.activate_containing(&tab);
                }
                if let Some(path) = workspace.find_tab(&tab) {
                    let _ = workspace.set_active_tab(path);
                    workspace.set_focused_node_and_surface(path.node_path());
                }
            }
            if self.selected.as_deref() == Some(project) {
                self.hide_center_overlay();
                self.active_session = None;
                self.non_terminal_selected = true;
                self.focus_tab = Some(tab);
            }
            return true;
        }
        let floated = self
            .floating
            .iter()
            .filter(|window| window.home.0 == project)
            .find_map(|window| {
                let dock = window.dock.as_ref()?;
                dock.iter_all_tabs()
                    .map(|(_, tab)| tab)
                    .find(|tab| same_file_view(tab, requested))
                    .map(|tab| (window.viewport, tab.clone()))
            });
        if let Some((viewport, tab)) = floated {
            if self.selected.as_deref() == Some(project) {
                self.hide_center_overlay();
                self.active_session = None;
                self.non_terminal_selected = true;
                self.focus_float_tab = Some((viewport, tab));
                let _ = self.update_tx.send(Update::FocusViewport(viewport));
            }
            return true;
        }
        false
    }

    pub(crate) fn reuse_file_session(&mut self, project: &str, path: &std::path::Path) -> bool {
        let existing = self
            .state
            .sessions
            .iter()
            .find(|session| {
                session.project_id == project
                    && session.kind == SessionKind::Editor
                    && !session.review
                    && session.lifecycle.live()
                    && daemon_connection::session_owner_available(&self.state, session)
                    && session
                        .file
                        .as_ref()
                        .is_some_and(|file| file.components().eq(path.components()))
            })
            .map(|session| session.id.clone());
        if let Some(session) = existing {
            // Defer until all rendering docks are checked back in.
            self.hide_center_overlay();
            let _ = self
                .update_tx
                .send(Update::RevealFileSession(session, project.into()));
            true
        } else {
            false
        }
    }

    pub(crate) fn open_file(&mut self, input: OpenFileInput<'_>) {
        let OpenFileInput {
            path,
            line,
            split,
            external,
        } = input;
        self.open_file_mode(OpenFileModeInput {
            path,
            line,
            split,
            external,
            text: false,
        });
    }

    pub(crate) fn open_file_mode(&mut self, input: OpenFileModeInput<'_>) {
        let OpenFileModeInput {
            path,
            line,
            split,
            external,
            text,
        } = input;
        if let Some(remote) = self.selected_remote_project().cloned() {
            if let Some(relative) = self.remote_relative_path(&remote, &path) {
                self.open_remote_file(OpenRemoteFileInput {
                    project: &remote,
                    path: relative,
                    line,
                    split,
                    text: text || external,
                });
            } else {
                self.error = Some("Remote file paths must be UTF-8".into());
            }
            return;
        }
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
            self.open_native(OpenNativeInput {
                project: &project,
                path,
                line,
                split,
            });
            return;
        }
        if split.is_none()
            && let Some(project) = self.selected.clone()
            && self.reuse_file_session(&project, &path)
        {
            return;
        }
        self.hide_center_overlay();
        if let Some(project) = &self.selected {
            let _ = self.jobs.send(Job::rpc(
                Request::Create {
                    project: project.clone(),
                    cwd: None,
                    file: Some(path),
                    line,
                    column: None,
                    editor: true,
                },
                self.editor_target(project, None, split),
            ));
        }
    }
    /// `path` relative to the remote project, whether it points into the
    /// local snapshot cache or at the remote root; `None` when not UTF-8.
    fn remote_relative_path(
        &self,
        remote: &terminator_core::remote::RemoteProject,
        path: &std::path::Path,
    ) -> Option<String> {
        let cache = crate::remote_ui::snapshot_root(&self.paths.data, &remote.id);
        path.strip_prefix(&cache)
            .or_else(|_| path.strip_prefix(remote.identity_path()))
            .or_else(|_| path.strip_prefix(&remote.root))
            .map_or_else(
                |_| path.to_str().map(str::to_owned),
                |p| p.to_str().map(str::to_owned),
            )
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

    pub(crate) fn sync_browsers(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        // Floating windows and dialogs (middle layer) and menus, context
        // menus, and combo boxes (foreground layer) would otherwise be drawn
        // over by the native view.
        let covers: Vec<egui::Rect> = ctx.memory(|memory| {
            memory
                .areas()
                .visible_layer_ids()
                .into_iter()
                .filter(|layer| {
                    matches!(layer.order, egui::Order::Middle | egui::Order::Foreground)
                })
                .filter_map(|layer| memory.area_rect(layer.id))
                .collect()
        });
        self.browser_host.sync(browser_host::SyncInput {
            ctx,
            frame,
            visible: &self.visible_browsers,
            occluded: self.browser_covered(),
            covers: &covers,
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
    pub(crate) fn open_gui_tab(&mut self, project: String, tab: Tab, after: After) {
        if matches!(after, After::Workspace(..)) && self.focus_file_view(&project, &tab) {
            return;
        }
        self.place_gui_tab(project, tab, after);
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
            let anchors = match after {
                After::Workspace(_, anchors) => anchors,
                _ => Vec::new(),
            };
            self.layouts
                .entry(project.clone())
                .or_insert_with(Workspace::empty)
                .add_from(id(), tab, &anchors);
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
