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
        | (Tab::Document { path: left, .. }, Tab::Document { path: right, .. })
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
                self.focus_non_terminal();
                self.queue_main_focus(tab);
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
                self.focus_non_terminal();
                self.queue_focus(DockTarget::Float(viewport), tab);
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
                self.error.set("Remote file paths must be UTF-8");
            }
            return;
        }
        if !external && !text && self.open_default_viewer(&path, split) {
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
    /// Opens `path` in its GUI-only default viewer (image, table, browser or
    /// player). False when it has none and opens as text.
    fn open_default_viewer(&mut self, path: &std::path::Path, split: Option<&str>) -> bool {
        let document = crate::document_view::DocumentKind::for_path(path);
        if !(image_preview::supported(path)
            || document.is_some()
            || crate::browser::supported_file(path)
            || player::supported(path))
        {
            return false;
        }
        let Some(project) = self.selected.clone() else {
            return true;
        };
        let path = path.to_path_buf();
        if image_preview::supported(&path) {
            self.open_image(&project, path, split);
        } else if document.is_some() {
            self.open_document(&project, path, split);
        } else if crate::browser::supported_file(&path) {
            self.open_html(&project, path, split);
        } else {
            self.open_audio(&project, path, split);
        }
        true
    }

    /// Opens `path` in its `Tab::Document` viewer; no-op without one.
    pub(crate) fn open_document(&mut self, project: &str, path: PathBuf, split: Option<&str>) {
        let Some(kind) = crate::document_view::DocumentKind::for_path(&path) else {
            return;
        };
        self.hide_center_overlay();
        let origin = self
            .active_session
            .as_ref()
            .map(|sid| Tab::Terminal(sid.clone()));
        let after = self.editor_target(project, origin.as_ref(), split);
        let path = std::path::absolute(&path).unwrap_or(path);
        let _ = self.update_tx.send(Update::OpenDocument(
            project.into(),
            Tab::Document { path, kind },
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
        self.center_covers_workspace()
            || self.command_dialog_open()
            || self.picker.busy()
            || self.close_session.is_some()
            || self.close_workspace.is_some()
            || self.notice_detail_modal_open()
            || self.rename_blocks_input()
    }

    pub(crate) fn sync_browsers(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        // Floating windows and dialogs (middle layer) and menus, context
        // menus, and combo boxes (foreground layer), and tooltips would
        // otherwise be drawn over by the native view.
        let covers: Vec<egui::Rect> = ctx.memory(|memory| {
            memory
                .areas()
                .visible_layer_ids()
                .into_iter()
                .filter(|layer| crate::browser::covers_webview(layer.order))
                .filter_map(|layer| memory.area_rect(layer.id))
                .collect()
        });
        self.browser_host
            .set_devtools(self.preferences.browser.devtools);
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
            if let Some(project) = project
                && let Err(error) = self.open_browser_url(&project, &url, None)
            {
                self.error.set(format!("Could not open {url}: {error:#}"));
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
        let reuse = match &after {
            After::Workspace(..) => true,
            After::CreateAt(_, _, split) => split.as_deref() == Some(Self::SPLIT_NAV),
            _ => false,
        };
        if reuse && self.focus_file_view(&project, &tab) {
            return;
        }
        self.place_gui_tab(project, tab, after);
    }

    pub(crate) fn place_gui_tab(&mut self, project: String, tab: Tab, after: After) {
        if let After::CreateAt(origin, anchors, direction) = after {
            let previous = self.layouts.get(&project).map(|d| d.active.clone());
            self.focus_created_target(&project, origin.as_deref(), &anchors);
            let same = previous.as_ref() == self.layouts.get(&project).map(|d| &d.active);
            self.insert(&project, tab, direction.as_deref());
            if !same
                && let Some(previous) = previous
                && let Some(dock) = self.layouts.get_mut(&project)
            {
                dock.active = previous;
            }
            if same && self.selected.as_deref() == Some(&project) {
                self.focus_non_terminal();
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
                self.focus_non_terminal();
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
        let mut open = std::collections::HashMap::new();
        for (project, workspace) in &self.layouts {
            for group in &workspace.tabs {
                for (_, tab) in group.layout.iter_all_tabs() {
                    if let Tab::Browser { target, .. } = tab {
                        browsers.insert(tab.key());
                        open.insert(tab.key(), (project.clone(), target.clone()));
                    }
                }
            }
        }
        self.browser_extras.closed.track(open);
        self.browser_extras
            .find
            .retain(|key, _| browsers.contains(key));
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
}
