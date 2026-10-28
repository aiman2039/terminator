//! Removing single copies of a native file view from docks and floating
//! windows.
use super::*;

impl App {
    /// Remove one `target` copy from one top-level tab: the copy in
    /// `node` (the issuing pane recorded when the command started), then
    /// the focused leaf, then the first match. With `strict` (a delayed
    /// `:wq` naming its pane), only the recorded pane goes: a stale node
    /// (issuing view already closed) is a no-op, never another copy.
    /// Fixes `primary` afterwards. Returns whether a copy was removed.
    pub(super) fn remove_copy_from_tab(
        tab_state: &mut WorkspaceTab,
        target: &Tab,
        node: Option<egui_dock::NodePath>,
        strict: bool,
    ) -> bool {
        let same_leaf = |at: &egui_dock::TabPath, leaf: egui_dock::NodePath| {
            at.surface == leaf.surface && at.node == leaf.node
        };
        let pick = {
            let layout = &tab_state.layout;
            layout
                .iter_all_tabs()
                .filter(|(_, tab)| *tab == target)
                .map(|(at, _)| at)
                .find(|at| node.is_some_and(|stored| same_leaf(at, stored)))
                .or_else(|| {
                    (!strict)
                        .then(|| {
                            layout
                                .iter_all_tabs()
                                .filter(|(_, tab)| *tab == target)
                                .map(|(at, _)| at)
                                .find(|at| {
                                    layout
                                        .focused_leaf()
                                        .is_some_and(|focus| same_leaf(at, focus))
                                })
                        })
                        .flatten()
                })
                .or_else(|| (!strict).then(|| layout.find_tab(target)).flatten())
        };
        let Some(found) = pick else {
            return false;
        };
        tab_state.layout.remove_tab(found);
        if tab_state.primary.as_ref() == Some(target) {
            tab_state.primary = tab_state
                .layout
                .iter_all_tabs()
                .next()
                .map(|(_, tab)| tab.clone());
        }
        true
    }

    /// Remove one docked copy of `path` from `workspace`: the focused
    /// copy first, then the first match. Returns whether a copy was
    /// there.
    pub(super) fn remove_one_docked_copy(workspace: &mut Workspace, path: &Path) -> bool {
        let target = Tab::NativeEditor {
            path: path.to_owned(),
        };
        let mut removed = false;
        for tab in &mut workspace.tabs {
            if Self::remove_copy_from_tab(tab, &target, None, false) {
                removed = true;
                break;
            }
        }
        // Restores the non-empty invariant (`:qa` can empty a
        // workspace); a bare retain panics the next dock access.
        workspace.drop_empty_tabs();
        removed
    }

    /// Remove one `target` copy from a floating window's dock: the copy
    /// in `node` (the issuing pane recorded when the command started),
    /// then the window's focused copy, then the first match. `strict`
    /// mirrors [`Self::remove_copy_from_tab`]: a delayed `:wq` whose
    /// issuing view is gone closes nothing. Returns whether a copy was
    /// removed.
    pub(super) fn remove_copy_from_float_dock(
        dock: &mut egui_dock::DockState<Tab>,
        target: &Tab,
        node: Option<egui_dock::NodePath>,
        strict: bool,
    ) -> bool {
        let same_leaf = |at: &egui_dock::TabPath, leaf: egui_dock::NodePath| {
            at.surface == leaf.surface && at.node == leaf.node
        };
        let pick = dock
            .iter_all_tabs()
            .filter(|(_, tab)| *tab == target)
            .map(|(at, _)| at)
            .find(|at| node.is_some_and(|stored| same_leaf(at, stored)))
            .or_else(|| {
                (!strict)
                    .then(|| {
                        dock.iter_all_tabs()
                            .filter(|(_, tab)| *tab == target)
                            .map(|(at, _)| at)
                            .find(|at| {
                                dock.main_surface().focused_leaf().is_some_and(|focus| {
                                    same_leaf(
                                        at,
                                        egui_dock::NodePath {
                                            surface: egui_dock::SurfaceIndex::main(),
                                            node: focus,
                                        },
                                    )
                                })
                            })
                    })
                    .flatten()
            })
            .or_else(|| (!strict).then(|| dock.find_tab(target)).flatten());
        let Some(found) = pick else {
            return false;
        };
        dock.remove_tab(found);
        true
    }

    /// Drop floating windows left with no tabs (a plain close emptied
    /// them, or a force close took their last copy).
    pub(super) fn drop_empty_floats(&mut self) {
        self.floating.retain(|window| {
            window
                .dock
                .as_ref()
                .is_none_or(|dock| dock.iter_all_tabs().next().is_some())
        });
    }
}
