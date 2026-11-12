//! Dock bookkeeping that runs after a workspace or strip dock is checked
//! back in: queued tab creation and focus, pane moves, and focus paint.

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use crate::*;

mod floating;
mod ops;
mod pane_drop;
mod session_drop;
mod zoom;

pub(crate) use ops::{DockOp, DockOps, DockTarget};
pub(crate) use zoom::{PaneOps, Zoomed};

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

    /// Apply the strip ops queued while `dock` rendered, in order.
    pub(crate) fn apply_strip_dock_ops(
        &mut self,
        project: &str,
        dock: &mut egui_dock::DockState<Tab>,
    ) {
        for op in self.dock_ops.take_project(project, DockOp::strip_dock) {
            match op {
                DockOp::AddTab { path, split, .. } => {
                    self.apply_add_strip_tab(project, dock, (path, split));
                }
                DockOp::FocusTab { tab, .. } => focus_dock_tab(dock, &tab),
                _ => {}
            }
        }
    }

    fn apply_add_strip_tab(
        &mut self,
        project: &str,
        dock: &mut egui_dock::DockState<Tab>,
        (path, split): (egui_dock::NodePath, Option<String>),
    ) {
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

    /// End a pane drag: the one exit for every drop, cancel, and release.
    /// A drop that `landed` in the main dock keeps the previewed tab and
    /// saves the layout; any other end switches back to the tab the drag
    /// started on. `dock` is the workspace while it is checked out for
    /// paint; with `None` the preview reverts in `layouts`.
    pub(crate) fn end_pane_drag(&mut self, dock: Option<&mut Workspace>, landed: bool) {
        let origin = self.drag.preview_origin.take();
        if landed {
            self.pending_layout_save = true;
        } else if let Some((project, origin)) = origin {
            let dock = match dock {
                Some(dock) => Some(dock),
                None => self.layouts.get_mut(&project),
            };
            // Previewing never moves panes, so the origin normally exists.
            if let Some(dock) = dock
                && dock.tabs.iter().any(|tab| tab.id == origin)
            {
                dock.active = origin;
            }
        }
        self.drag.pane = None;
        self.drag.pane_source = None;
        self.drag.pane_from_strip = false;
        self.drag.pane_snapshot.clear();
        self.drag.strip_tab_hover = false;
        self.drag.strip_new_tab_hover = false;
    }

    /// Apply the main-dock ops of `project`, in queue order, now that its
    /// dock is checked back in. An op may queue a focus; it applies here too.
    pub(crate) fn apply_main_dock_ops(&mut self, project: &str, dock: &mut Workspace) {
        // A pane move queues at most one focus, so two rounds settle it.
        for _ in 0..2 {
            let ops = self.dock_ops.take_project(project, DockOp::main_dock);
            if ops.is_empty() {
                return;
            }
            for op in ops {
                match op {
                    DockOp::AddTab { path, split, .. } => {
                        self.apply_add_tab(project, dock, (path, split));
                    }
                    DockOp::FocusTab { tab, .. } => focus_dock_tab(dock, &tab),
                    DockOp::Detach { pane, source } => self.apply_detach_pane(dock, pane, source),
                    DockOp::DockBack { pane, dest, source } => {
                        self.apply_dock_back(DockBackInput {
                            dock,
                            pane,
                            dest,
                            source,
                        });
                    }
                    DockOp::Float { pane, source } => {
                        self.apply_float_pane(project, dock, (pane, source));
                    }
                    _ => {}
                }
            }
        }
    }

    /// Apply the ops queued by one floating window, in order.
    pub(crate) fn apply_float_dock_ops(
        &mut self,
        viewport: egui::ViewportId,
        project: &str,
        dock: &mut egui_dock::DockState<Tab>,
    ) -> Vec<DockOp> {
        let mut returns = Vec::new();
        for op in self.dock_ops.take_window(viewport) {
            match op {
                DockOp::AddTab { path, split, .. } => {
                    self.apply_add_float_tab(AddFloatTabInput {
                        viewport,
                        project,
                        dock,
                        path,
                        split,
                    });
                }
                DockOp::FocusTab { tab, .. } => focus_dock_tab(dock, &tab),
                other => returns.push(other),
            }
        }
        returns
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
                Tab::Image { path } | Tab::NativeEditor { path } | Tab::Document { path, .. } => {
                    path.parent().map(PathBuf::from)
                }
                Tab::Browser { target, .. } => {
                    target.file().and_then(Path::parent).map(PathBuf::from)
                }
                Tab::Player | Tab::Vnc { .. } | Tab::RemoteApp { .. } => None,
                Tab::CommitLog { cwd }
                | Tab::Blame { cwd, .. }
                | Tab::RevisionDiff { cwd, .. }
                | Tab::FileHistory { cwd, .. }
                | Tab::Merge { cwd, .. }
                | Tab::ChangesDiff { cwd, .. } => Some(cwd.clone()),
            })
    }

    fn apply_add_tab(
        &mut self,
        project: &str,
        dock: &mut Workspace,
        (path, split): (egui_dock::NodePath, Option<String>),
    ) {
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
                Some(dock.active.clone()),
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
    fn apply_add_float_tab(&mut self, input: AddFloatTabInput<'_>) {
        let AddFloatTabInput {
            viewport,
            project,
            dock,
            path,
            split,
        } = input;
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

    /// Detach the queued pane into a fresh top-level tab at the end of the
    /// strip. Runs after the workspace dock is checked back in; unknown or
    /// already-moved panes are a silent no-op via the move primitive.
    fn apply_detach_pane(&mut self, dock: &mut Workspace, pane: Tab, source: Option<PaneSource>) {
        let moved = if let Some(source) = source {
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
                self.focus_terminal(sid.clone());
                self.queue_dock_op(DockOp::FocusTab {
                    target: DockTarget::Main,
                    tab: pane,
                });
            }
            self.pane_index = None;
            self.pending_layout_save = true;
        }
    }

    /// Dock the queued pane back into its chosen sibling tab. Same
    /// checked-back-in contract as [`Self::apply_detach_pane`].
    fn apply_dock_back(&mut self, input: DockBackInput<'_>) {
        let DockBackInput {
            dock,
            pane,
            dest,
            source,
        } = input;
        let moved = if let Some(source) = source {
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
                self.focus_terminal(sid.clone());
                self.queue_dock_op(DockOp::FocusTab {
                    target: DockTarget::Main,
                    tab: pane,
                });
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
                appearance::RADIUS_XSMALL,
                appearance::focus_stroke(
                    appearance::color(&self.theme.accent),
                    self.highlight_since.elapsed(),
                ),
                egui::StrokeKind::Inside,
            );
        }
        if let Some(pane) = self.focused_main_pane(dock.main_surface()) {
            let rule = appearance::paint_focus_rule(
                ui.painter(),
                pane,
                appearance::color(&self.theme.accent),
            );
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "pane-focus-rule", rule);
            let _ = rule;
        }
        if self.highlight_since.elapsed() < Duration::from_millis(1200) {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
    }

    /// The rect of the pane that has the keys, when it is in `surface` and
    /// `surface` has more than one pane. A terminal is found by its id, so
    /// a strip or floating terminal marks nothing here; with no terminal
    /// focused, the surface's focused leaf counts when its active tab is
    /// not a terminal (editor, browser, diff).
    pub(crate) fn focused_main_pane(&self, surface: &egui_dock::Tree<Tab>) -> Option<egui::Rect> {
        let panes = surface
            .iter()
            .filter(|node| node.tabs().is_some_and(|tabs| !tabs.is_empty()))
            .count();
        if panes < 2 {
            return None;
        }
        let node = match &self.active_session {
            Some(sid) => surface.find_tab(&Tab::Terminal(sid.clone()))?.0,
            None if self.non_terminal_selected => surface.focused_leaf()?,
            None => return None,
        };
        let leaf = surface.leaf(node).ok()?;
        let terminal = matches!(leaf.tabs.get(leaf.active.0), Some(Tab::Terminal(_)));
        (self.active_session.is_some() || !terminal).then_some(leaf.rect)
    }
}

struct AddFloatTabInput<'a> {
    viewport: egui::ViewportId,
    project: &'a str,
    dock: &'a mut egui_dock::DockState<Tab>,
    path: egui_dock::NodePath,
    split: Option<String>,
}

struct DockBackInput<'a> {
    dock: &'a mut Workspace,
    pane: Tab,
    dest: String,
    source: Option<PaneSource>,
}

/// Activate and focus `tab` in `dock`, if it is there.
pub(crate) fn focus_dock_tab(dock: &mut egui_dock::DockState<Tab>, tab: &Tab) {
    if let Some(path) = dock.find_tab(tab) {
        let _ = dock.set_active_tab(path);
        dock.set_focused_node_and_surface(path.node_path());
    }
}
