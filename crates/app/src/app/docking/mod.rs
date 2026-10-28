//! Dock bookkeeping that runs after a workspace or strip dock is checked
//! back in: queued tab creation and focus, pane moves, and focus paint.

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use crate::*;

mod floating;
mod pane_drop;

impl App {
    pub(crate) fn focus_moved_view(&mut self, pane: &Tab, project: &str, dock: &Workspace) {
        self.follow_focus_tab(Some(pane.clone()));
        if matches!(pane, Tab::NativeEditor { .. })
            && let Some(node) = dock.main_surface().focused_leaf()
        {
            let path = egui_dock::NodePath {
                surface: egui_dock::SurfaceIndex::main(),
                node,
            };
            if let Ok(leaf) = dock.leaf(path) {
                self.native_focus_pending = Some(PaneSource {
                    project: project.to_owned(),
                    group: dock.active.clone(),
                    path: (path, leaf.active).into(),
                });
            }
        }
    }
    /// Strip-dock pane lookup, rebuilt every strip render (small dock; no cache).
    pub(crate) fn refresh_strip_pane_maps(&mut self, dock: &egui_dock::DockState<Tab>) {
        self.strip_pane_by_tab = dock
            .iter_all_tabs()
            .map(|(path, tab)| (tab.key(), path.node_path()))
            .collect();
        self.strip_pane_tabs = dock
            .iter_leaves()
            .map(|(path, leaf)| (path, leaf.tabs.clone()))
            .collect();
    }

    pub(crate) fn apply_focus_strip_tab(&mut self, dock: &mut egui_dock::DockState<Tab>) {
        if let Some(tab) = self.focus_strip_tab.take()
            && let Some(path) = dock.find_tab(&tab)
        {
            let _ = dock.set_active_tab(path);
            dock.set_focused_node_and_surface(path.node_path());
        }
    }

    pub(crate) fn apply_add_strip_tab(
        &mut self,
        project: &str,
        dock: &mut egui_dock::DockState<Tab>,
    ) {
        let Some((path, split)) = self.add_strip_tab.take() else {
            return;
        };
        let cwd = dock
            .leaf(path)
            .ok()
            .and_then(|leaf| leaf.tabs.get(leaf.active.0))
            .and_then(|tab| match tab {
                Tab::Terminal(id) => self
                    .state
                    .sessions
                    .iter()
                    .find(|s| &s.id == id)
                    .map(|s| s.cwd.clone()),
                _ => None,
            })
            .or_else(|| self.selected_project().map(|p| p.path.clone()));
        let _ = self.jobs.send(Job::rpc(
            Request::Create {
                project: project.to_owned(),
                cwd,
                file: None,
                line: None,
                column: None,
                editor: false,
            },
            // Anchor even plain "new tab" on the clicked pane; otherwise the
            // tab lands in whichever strip leaf happened to be focused.
            After::StripAt(
                dock.leaf(path)
                    .map(|leaf| leaf.tabs.clone())
                    .unwrap_or_default(),
                split,
            ),
        ));
    }

    /// Clear a finished or cancelled pane drag, including its ghost snapshot.
    pub(crate) fn end_pane_drag(&mut self) {
        self.drag.pane = None;
        self.drag.pane_source = None;
        self.drag.pane_from_strip = false;
        self.drag.pane_snapshot.clear();
        self.drag.strip_tab_hover = false;
        self.drag.strip_new_tab_hover = false;
    }

    pub(crate) fn apply_focus_tab(&mut self, dock: &mut Workspace) {
        if let Some(tab) = self.focus_tab.take()
            && let Some(path) = dock.find_tab(&tab)
        {
            let _ = dock.set_active_tab(path);
            dock.set_focused_node_and_surface(path.node_path());
        }
    }

    /// Working directory of the active pane in leaf `path`, for a new
    /// terminal opened beside it.
    fn leaf_cwd(
        &self,
        dock: &egui_dock::DockState<Tab>,
        path: egui_dock::NodePath,
    ) -> Option<PathBuf> {
        dock.leaf(path)
            .ok()
            .and_then(|leaf| leaf.tabs.get(leaf.active.0))
            .and_then(|tab| match tab {
                Tab::Terminal(id) => self
                    .state
                    .sessions
                    .iter()
                    .find(|s| &s.id == id)
                    .map(|s| s.cwd.clone()),
                Tab::Diff { cwd, .. } => Some(cwd.clone()),
                Tab::Image { path } | Tab::NativeEditor { path } => {
                    path.parent().map(PathBuf::from)
                }
                Tab::Browser { target, .. } => {
                    target.file().and_then(Path::parent).map(PathBuf::from)
                }
                Tab::Player | Tab::Vnc { .. } => None,
                Tab::CommitLog { cwd }
                | Tab::Blame { cwd, .. }
                | Tab::RevisionDiff { cwd, .. }
                | Tab::FileHistory { cwd, .. } => Some(cwd.clone()),
            })
    }

    pub(crate) fn apply_add_tab(&mut self, project: &str, dock: &mut Workspace) {
        let Some((path, split)) = self.add_tab.take() else {
            return;
        };
        let cwd = self
            .leaf_cwd(dock, path)
            .or_else(|| self.selected_project().map(|p| p.path.clone()));
        let _ = self.jobs.send(Job::rpc(
            Request::Create {
                project: project.to_owned(),
                cwd,
                file: None,
                line: None,
                column: None,
                editor: false,
            },
            // The leaf `+` always opens stacked inside that split; the
            // workspace-strip `+` (a new top-level tab) arrives through
            // `create_workspace_tab`, not here.
            After::CreateAt(
                dock.leaf(path)
                    .map(|leaf| leaf.tabs.clone())
                    .unwrap_or_default(),
                split,
            ),
        ));
    }

    /// Create the queued floating tab: same leaf-anchored request as the
    /// main dock, but completing into the issuing window via `After::Float`
    /// so a focus or project change mid-flight cannot redirect it.
    pub(crate) fn apply_add_float_tab(
        &mut self,
        viewport: egui::ViewportId,
        project: &str,
        dock: &mut egui_dock::DockState<Tab>,
    ) {
        let Some((queued, path, split)) = self.add_float_tab.take() else {
            return;
        };
        if queued != viewport {
            self.add_float_tab = Some((queued, path, split));
            return;
        }
        let cwd = self
            .leaf_cwd(dock, path)
            .or_else(|| {
                self.state
                    .projects
                    .iter()
                    .find(|p| p.id == project)
                    .map(|p| p.path.clone())
            })
            .or_else(|| self.selected_project().map(|p| p.path.clone()));
        let _ = self.jobs.send(Job::rpc(
            Request::Create {
                project: project.to_owned(),
                cwd,
                file: None,
                line: None,
                column: None,
                editor: false,
            },
            After::Float(
                viewport,
                dock.leaf(path)
                    .map(|leaf| leaf.tabs.clone())
                    .unwrap_or_default(),
                split,
            ),
        ));
    }

    /// Focus the queued tab in its floating window. Runs after that
    /// window's dock is checked back in.
    pub(crate) fn apply_focus_float_tab(
        &mut self,
        viewport: egui::ViewportId,
        dock: &mut egui_dock::DockState<Tab>,
    ) {
        let Some((queued, tab)) = self.focus_float_tab.take() else {
            return;
        };
        if queued != viewport {
            self.focus_float_tab = Some((queued, tab));
            return;
        }
        if let Some(path) = dock.find_tab(&tab) {
            let _ = dock.set_active_tab(path);
            dock.set_focused_node_and_surface(path.node_path());
        }
    }

    /// Detach the queued pane into a fresh top-level tab at the end of the
    /// strip. Runs after the workspace dock is checked back in; unknown or
    /// already-moved panes are a silent no-op via the move primitive.
    pub(crate) fn apply_detach_pane(&mut self, dock: &mut Workspace) {
        let Some(pane) = self.detach_pane.take() else {
            return;
        };
        let moved = if let Some(source) = self.pane_action_source.take() {
            self.selected.as_ref() == Some(&source.project)
                && dock
                    .move_view_to_new_group(&source, &pane, dock.tabs.len())
                    .is_some()
        } else {
            dock.move_pane_to_new_group_at(&pane, dock.tabs.len())
                .is_some()
        };
        if moved {
            if let Some(project) = self.selected.clone() {
                self.focus_moved_view(&pane, &project, dock);
            }
            if let Tab::Terminal(sid) = &pane {
                self.active_session = Some(sid.clone());
                self.focus_tab = Some(pane);
            }
            self.pane_index = None;
            self.pending_layout_save = true;
        }
    }

    /// Dock the queued pane back into its chosen sibling tab. Same
    /// checked-back-in contract as [`Self::apply_detach_pane`].
    pub(crate) fn apply_dock_back(&mut self, dock: &mut Workspace) {
        let Some((pane, dest)) = self.dock_back_pane.take() else {
            return;
        };
        let moved = if let Some(source) = self.pane_action_source.take() {
            self.selected.as_ref() == Some(&source.project)
                && dock.move_view_to_group(&source, &pane, &dest)
        } else {
            dock.move_pane_to_group(&pane, &dest)
        };
        if moved {
            if let Some(project) = self.selected.clone() {
                self.focus_moved_view(&pane, &project, dock);
            }
            if let Tab::Terminal(sid) = &pane {
                self.active_session = Some(sid.clone());
                self.focus_tab = Some(pane);
            }
            self.pane_index = None;
            self.pending_layout_save = true;
        }
    }

    pub(crate) fn paint_session_focus(&mut self, ui: &mut egui::Ui, dock: &Workspace) {
        if self.highlight_session != self.active_session {
            self.highlight_session = self.active_session.clone();
            self.highlight_since = Instant::now();
        }
        if let Some(sid) = &self.active_session
            && let Some(path) = dock.find_tab(&Tab::Terminal(sid.clone()))
            && let Ok(leaf) = dock.leaf(path.node_path())
        {
            ui.painter().rect_stroke(
                leaf.rect.shrink(1.0),
                2,
                appearance::focus_stroke(
                    appearance::color(&self.theme.accent),
                    self.highlight_since.elapsed(),
                ),
                egui::StrokeKind::Inside,
            );
        }
        if self.highlight_since.elapsed() < Duration::from_millis(1200) {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
    }
}
