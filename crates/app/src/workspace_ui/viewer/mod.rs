//! The dock tab viewer: tab chrome, pane menus, and pane bodies.
use super::super::*;

mod caption;
mod chrome;
mod dead_session;
mod pane_menu;
mod tab_viewer;
mod terminal_pane;

pub(crate) struct Viewer<'a> {
    pub(crate) app: &'a mut App,
    /// True when rendering the IDE strip dock instead of the main dock.
    /// Creation queues, pane maps, and focus targets switch docks; the
    /// strip always shows native tab bars, while the main dock shows
    /// them only on stacked leaves (nested tabs) and otherwise titles
    /// single terminals with captions under the workspace strip. Native
    /// editors keep their dock tab so each split has a drag handle.
    pub(crate) strip: bool,
    /// Docked project under render, if any: native close requests
    /// remember their issuing view. Floating windows pass none (the
    /// viewport identifies them instead).
    pub(crate) project: Option<String>,
    /// Top-level tab under render, if any: same-file twin tabs need it
    /// to tell the issuing copy apart.
    pub(crate) tab: Option<String>,
    /// Floating window under render, if any: pane actions (splits, focus,
    /// creation) route to this window's dock instead of the main dock.
    pub(crate) window: Option<egui::ViewportId>,
    /// Leaf currently rendering, reported by the dock before each tab
    /// body: native close requests remember their issuing pane. Starts
    /// empty for direct (non-dock) renders such as floating panes.
    pub(crate) render_path: Option<egui_dock::NodePath>,
    pub(crate) render_tab: Option<egui_dock::TabPath>,
}
impl Viewer<'_> {
    /// Queue tab creation in the dock under render: strip, floating
    /// window, or main dock. Floating splits land in the issuing window.
    fn queue_add_tab(&mut self, pane: egui_dock::NodePath, split: Option<String>) {
        if self.strip {
            self.app.add_strip_tab = Some((pane, split));
        } else if let Some(viewport) = self.window {
            self.app.add_float_tab = Some((viewport, pane, split));
        } else {
            self.app.add_tab = Some((pane, split));
        }
    }
    /// Focus a tab in the dock under render.
    fn queue_focus_tab(&mut self, tab: Tab) {
        if self.strip {
            self.app.focus_strip_tab = Some(tab);
        } else if let Some(viewport) = self.window {
            self.app.focus_float_tab = Some((viewport, tab));
        } else {
            self.app.focus_tab = Some(tab);
        }
    }
    /// Whether the terminal under render accepts input: floating windows
    /// use their own gate so center-only overlays never suspend them.
    pub(crate) fn terminal_input_for(&self, sid: &str) -> bool {
        if self.strip {
            self.app.strip_terminal_input_enabled(sid)
        } else if self.window.is_some() {
            self.app.float_terminal_input_enabled(sid)
        } else {
            self.app.terminal_input_enabled(sid)
        }
    }

    pub(crate) fn terminal_copy_for(&self, sid: &str) -> bool {
        if self.strip {
            self.app.strip_terminal_copy_enabled(sid)
        } else if self.window.is_some() {
            self.app.float_terminal_copy_enabled(sid)
        } else {
            self.app.terminal_copy_enabled(sid)
        }
    }
    /// Leaf path for a tab key in the dock under render, if any.
    pub(super) fn pane_for(&self, key: &str) -> Option<egui_dock::NodePath> {
        if self.strip {
            self.app.strip_pane_by_tab.get(key).copied()
        } else if self.window.is_some() {
            self.app.float_pane_by_tab.get(key).copied()
        } else {
            self.app.pane_by_tab.get(key).copied()
        }
    }
}
