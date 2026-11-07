//! Pane zoom: one main-dock terminal or native editor shown large over the
//! workspace until dismissed. GUI-only; the layout and PTY stay the same,
//! and the terminal resizes to the zoomed rect while it is shown.

use crate::*;

/// Pane requests the viewer makes while a dock is checked out for paint.
#[derive(Default)]
pub(crate) struct PaneOps {
    /// The zoomed view. While set, its dock pane paints a placeholder and
    /// the overlay paints the view, so it is never painted twice a frame.
    pub(crate) zoomed: Option<Zoomed>,
    /// Zoom asked for by a header button or menu: the pane (`None`: the
    /// leaf's active one) and its leaf. Applied before the next dock paint.
    pub(crate) zoom_request: Option<(Option<Tab>, Option<egui_dock::NodePath>)>,
    /// A floating window's pane (or, with `None`, the whole window) going
    /// back to the main window: the same path as the OS close button.
    pub(crate) float_return: Option<(egui::ViewportId, Option<Tab>)>,
}

/// The zoomed view: where it lives in the main dock, and what it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Zoomed {
    pub(crate) source: PaneSource,
    pub(crate) tab: Tab,
}

impl App {
    /// Panes that can zoom: terminals and native editors.
    pub(crate) const fn zoomable(tab: &Tab) -> bool {
        matches!(tab, Tab::Terminal(_) | Tab::NativeEditor { .. })
    }

    /// The pane the zoom key zooms: the focused terminal in the selected
    /// project's active tab, else that tab's focused editor.
    pub(crate) fn zoom_target(&self) -> Option<Zoomed> {
        let project = self.selected.as_ref()?;
        let dock = self.layouts.get(project)?;
        let terminal = self
            .active_session
            .as_ref()
            .map(|sid| Tab::Terminal(sid.clone()))
            .and_then(|tab| dock.find_tab(&tab).map(|path| (path, tab)));
        let fallback = self.non_terminal_selected || self.active_session.is_none();
        let (path, tab) = terminal.or_else(|| {
            let node = dock.main_surface().focused_leaf().filter(|_| fallback)?;
            let path = egui_dock::NodePath {
                surface: egui_dock::SurfaceIndex::main(),
                node,
            };
            let leaf = dock.leaf(path).ok()?;
            Some((
                (path, leaf.active).into(),
                leaf.tabs.get(leaf.active.0)?.clone(),
            ))
        })?;
        Self::zoomable(&tab).then(|| Zoomed {
            source: PaneSource {
                project: project.clone(),
                group: dock.active.clone(),
                path,
            },
            tab,
        })
    }

    /// The zoom key: restore a zoomed pane, else zoom the focused one.
    pub(crate) fn toggle_zoom(&mut self) {
        if self.pane_ops.zoomed.is_some() {
            self.unzoom();
        } else if let Some(zoomed) = self.zoom_target() {
            self.open_zoom(zoomed);
        }
    }

    /// A header button or menu asks to zoom `tab` (`None`: the active one
    /// of leaf `node`), or to restore it when it is the zoomed pane.
    pub(crate) fn queue_zoom(&mut self, tab: Option<Tab>, node: Option<egui_dock::NodePath>) {
        let zoomed = self.pane_ops.zoomed.as_ref().is_some_and(|z| match &tab {
            Some(tab) => z.tab == *tab,
            None => node == Some(z.source.path.node_path()),
        });
        if zoomed {
            self.unzoom();
        } else {
            self.pane_ops.zoom_request = Some((tab, node));
        }
    }

    fn open_zoom(&mut self, zoomed: Zoomed) {
        self.focus_zoomed(&zoomed);
        self.pane_ops.zoomed = Some(zoomed);
    }

    /// Return the zoomed pane to its place; the keyboard stays with it.
    pub(crate) fn unzoom(&mut self) {
        if let Some(zoomed) = self.pane_ops.zoomed.take() {
            self.focus_zoomed(&zoomed);
        }
    }

    /// Give the keyboard to the zoomed view (zoom in or out).
    fn focus_zoomed(&mut self, zoomed: &Zoomed) {
        self.follow_focus_tab(Some(zoomed.tab.clone()));
        if matches!(zoomed.tab, Tab::NativeEditor { .. }) {
            self.native_focus_pending = Some(zoomed.source.clone());
        }
    }

    /// Before the dock paints: apply a queued zoom, and drop a zoom whose
    /// pane closed, moved away, ended, or whose tab or project is no
    /// longer the one on screen.
    pub(crate) fn settle_zoom(&mut self, project: &str, dock: &mut Workspace) {
        if let Some((tab, node)) = self.pane_ops.zoom_request.take()
            && let Some((path, tab)) = zoom_path(dock, tab, node)
        {
            let _ = dock.set_active_tab(path);
            dock.set_focused_node_and_surface(path.node_path());
            self.open_zoom(Zoomed {
                source: PaneSource {
                    project: project.to_owned(),
                    group: dock.active.clone(),
                    path,
                },
                tab,
            });
        }
        self.check_zoom(project, dock);
    }

    /// After the dock painted the zoomed pane's placeholder: paint the
    /// zoom overlay over the workspace rect `outer`.
    pub(crate) fn paint_zoom(&mut self, ctx: &egui::Context, outer: egui::Rect, project: &str) {
        let Some(zoomed) = self.pane_ops.zoomed.clone() else {
            return;
        };
        let dismissed = Viewer {
            app: self,
            strip: false,
            project: Some(project.to_owned()),
            tab: Some(zoomed.source.group.clone()),
            window: None,
            render_path: Some(zoomed.source.path.node_path()),
            render_tab: Some(zoomed.source.path),
        }
        .zoom_overlay(ctx, outer, &zoomed);
        if dismissed {
            self.unzoom();
        }
    }

    /// Drop a zoom that no longer matches the dock on screen. A terminal
    /// that moved inside its tab follows its new leaf.
    pub(crate) fn check_zoom(&mut self, project: &str, dock: &Workspace) {
        let Some(zoomed) = self.pane_ops.zoomed.as_mut() else {
            return;
        };
        let source = &mut zoomed.source;
        let here = source.project == project && source.group == dock.active;
        let at_path = dock
            .leaf(source.path.node_path())
            .ok()
            .and_then(|leaf| leaf.tabs.get(source.path.tab.0))
            == Some(&zoomed.tab);
        let moved = match (&zoomed.tab, at_path) {
            (Tab::Terminal(_), false) => dock.find_tab(&zoomed.tab),
            _ => None,
        };
        if let Some(path) = moved {
            source.path = path;
        }
        let live = match &zoomed.tab {
            Tab::Terminal(sid) => self
                .state
                .sessions
                .iter()
                .any(|session| &session.id == sid && session.lifecycle.live()),
            _ => true,
        };
        if !here || !(at_path || moved.is_some()) || !live {
            self.pane_ops.zoomed = None;
        }
    }
}

/// Where `tab` sits: in leaf `node` when it holds it, else anywhere. No
/// `tab` means the active pane of leaf `node`.
fn zoom_path(
    dock: &Workspace,
    tab: Option<Tab>,
    node: Option<egui_dock::NodePath>,
) -> Option<(egui_dock::TabPath, Tab)> {
    let leaf = node.and_then(|node| Some((node, dock.leaf(node).ok()?)));
    let active = || leaf.and_then(|(_, leaf)| leaf.tabs.get(leaf.active.0).cloned());
    let tab = tab.or_else(active)?;
    if !App::zoomable(&tab) {
        return None;
    }
    leaf.and_then(|(node, leaf)| {
        let index = leaf.tabs.iter().position(|t| *t == tab)?;
        Some((node, egui_dock::TabIndex(index)).into())
    })
    .or_else(|| dock.find_tab(&tab))
    .map(|path| (path, tab))
}
