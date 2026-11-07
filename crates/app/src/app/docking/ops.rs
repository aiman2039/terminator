//! Dock requests made while a dock is checked out for paint. Each request
//! is one queued [`DockOp`] that carries everything it needs (its dock, its
//! pane, and the exact view source), so two requests in one frame never
//! overwrite each other and no request reads state another one left behind.

use crate::*;

/// The dock a tab request addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DockTarget {
    /// The selected project's main workspace dock.
    Main,
    /// The selected project's IDE terminal strip.
    Strip,
    /// One floating window's dock.
    Float(egui::ViewportId),
}

/// One deferred dock operation, applied in queue order once its dock is
/// checked back in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DockOp {
    /// Open a new terminal beside the active pane of leaf `path`.
    AddTab {
        target: DockTarget,
        path: egui_dock::NodePath,
        split: Option<String>,
    },
    /// Activate and focus `tab`.
    FocusTab { target: DockTarget, tab: Tab },
    /// Move a main-dock pane into a fresh top-level tab. `source` names
    /// the exact native editor view; other panes move by identity.
    Detach {
        pane: Tab,
        source: Option<PaneSource>,
    },
    /// Move a main-dock pane into the top-level tab `dest`.
    DockBack {
        pane: Tab,
        dest: String,
        source: Option<PaneSource>,
    },
    /// Float a main-dock pane into its own OS window.
    Float {
        pane: Tab,
        source: Option<PaneSource>,
    },
    /// A floating window's pane (`None`: the whole window) goes back to the
    /// main window: the same path as the OS close button.
    FloatReturn {
        viewport: egui::ViewportId,
        pane: Option<Tab>,
    },
    /// Move a strip shell into the main pane once the strip is checked in.
    StripToMain(String),
    /// Move a main-pane shell into the strip once the workspace is checked in.
    MainToStrip(String),
}

impl DockTarget {
    /// The dock a viewer or menu renders: the strip, a floating window, or
    /// the main dock.
    pub(crate) const fn of(strip: bool, window: Option<egui::ViewportId>) -> Self {
        match (strip, window) {
            (true, _) => Self::Strip,
            (false, Some(viewport)) => Self::Float(viewport),
            (false, None) => Self::Main,
        }
    }
}

impl DockOp {
    /// The floating window this op belongs to, if any.
    const fn viewport(&self) -> Option<egui::ViewportId> {
        match self {
            Self::AddTab {
                target: DockTarget::Float(viewport),
                ..
            }
            | Self::FocusTab {
                target: DockTarget::Float(viewport),
                ..
            }
            | Self::FloatReturn { viewport, .. } => Some(*viewport),
            _ => None,
        }
    }

    /// Ops applied to the main dock right after it is checked back in.
    pub(crate) const fn main_dock(&self) -> bool {
        matches!(
            self,
            Self::AddTab {
                target: DockTarget::Main,
                ..
            } | Self::FocusTab {
                target: DockTarget::Main,
                ..
            } | Self::Detach { .. }
                | Self::DockBack { .. }
                | Self::Float { .. }
        )
    }

    /// Ops applied to the strip dock right after it renders.
    pub(crate) const fn strip_dock(&self) -> bool {
        matches!(
            self,
            Self::AddTab {
                target: DockTarget::Strip,
                ..
            } | Self::FocusTab {
                target: DockTarget::Strip,
                ..
            }
        )
    }
}

/// Queued dock ops, oldest first, each with the project selected when it
/// was queued. Floating-window ops belong to their window instead.
#[derive(Default)]
pub(crate) struct DockOps {
    queue: Vec<(Option<String>, DockOp)>,
}

impl DockOps {
    pub(crate) fn push(&mut self, project: Option<String>, op: DockOp) {
        self.queue.push((project, op));
    }

    /// Removes and returns, in queue order, the ops `wanted` selects. It
    /// sees each op's owning project.
    pub(crate) fn take(&mut self, wanted: impl Fn(Option<&str>, &DockOp) -> bool) -> Vec<DockOp> {
        let (taken, kept) = std::mem::take(&mut self.queue)
            .into_iter()
            .partition(|(project, op)| wanted(project.as_deref(), op));
        self.queue = kept;
        taken.into_iter().map(|(_, op)| op).collect()
    }

    /// Ops of the main dock or strip of `project`, in queue order.
    pub(crate) fn take_project(
        &mut self,
        project: &str,
        wanted: impl Fn(&DockOp) -> bool,
    ) -> Vec<DockOp> {
        self.take(|owner, op| owner == Some(project) && wanted(op))
    }

    /// Ops of one floating window, in queue order.
    pub(crate) fn take_window(&mut self, viewport: egui::ViewportId) -> Vec<DockOp> {
        self.take(|_, op| op.viewport() == Some(viewport))
    }

    /// Drops ops nobody can apply any more: a floating window's ops once it
    /// closed, and every other op once its project is no longer selected.
    pub(crate) fn prune(
        &mut self,
        selected: Option<&str>,
        live: impl Fn(egui::ViewportId) -> bool,
    ) {
        self.queue.retain(|(project, op)| match op.viewport() {
            Some(viewport) => live(viewport),
            None => project.is_some() && project.as_deref() == selected,
        });
    }

    #[cfg(test)]
    pub(crate) fn iter(&self) -> impl Iterator<Item = &DockOp> {
        self.queue.iter().map(|(_, op)| op)
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
}

impl App {
    /// Queue `op` for the selected project's docks (or its own window).
    pub(crate) fn queue_dock_op(&mut self, op: DockOp) {
        self.dock_ops.push(self.selected.clone(), op);
    }

    /// Focus `tab` in the selected project's main dock once it is checked in.
    pub(crate) fn queue_main_focus(&mut self, tab: Tab) {
        self.queue_focus(DockTarget::Main, tab);
    }

    pub(crate) fn queue_focus(&mut self, target: DockTarget, tab: Tab) {
        self.queue_dock_op(DockOp::FocusTab { target, tab });
    }

    pub(crate) fn queue_add(
        &mut self,
        target: DockTarget,
        path: egui_dock::NodePath,
        split: Option<String>,
    ) {
        self.queue_dock_op(DockOp::AddTab {
            target,
            path,
            split,
        });
    }

    /// Drop queued ops whose dock is gone; runs once per frame after every
    /// dock painted.
    pub(crate) fn prune_dock_ops(&mut self) {
        let windows = self
            .floating
            .iter()
            .map(|window| window.viewport)
            .collect::<Vec<_>>();
        self.dock_ops.prune(self.selected.as_deref(), |viewport| {
            windows.contains(&viewport)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn focus(target: DockTarget, sid: &str) -> DockOp {
        DockOp::FocusTab {
            target,
            tab: Tab::Terminal(sid.into()),
        }
    }

    #[test]
    fn two_requests_in_one_frame_are_both_kept_in_order() {
        let mut ops = DockOps::default();
        ops.push(Some("p".into()), focus(DockTarget::Main, "a"));
        ops.push(
            Some("p".into()),
            DockOp::Detach {
                pane: Tab::Terminal("b".into()),
                source: None,
            },
        );
        ops.push(Some("p".into()), focus(DockTarget::Strip, "c"));
        let main = ops.take_project("p", DockOp::main_dock);
        assert_eq!(
            main,
            vec![
                focus(DockTarget::Main, "a"),
                DockOp::Detach {
                    pane: Tab::Terminal("b".into()),
                    source: None,
                },
            ]
        );
        assert_eq!(
            ops.iter().cloned().collect::<Vec<_>>(),
            vec![focus(DockTarget::Strip, "c")]
        );
    }

    #[test]
    fn another_project_does_not_take_queued_ops() {
        let mut ops = DockOps::default();
        ops.push(Some("a".into()), focus(DockTarget::Main, "one"));
        assert!(ops.take_project("b", DockOp::main_dock).is_empty());
        assert_eq!(ops.take_project("a", DockOp::main_dock).len(), 1);
    }

    #[test]
    fn prune_drops_ops_of_closed_windows_and_unselected_projects() {
        let live = egui::ViewportId::from_hash_of("live");
        let closed = egui::ViewportId::from_hash_of("closed");
        let mut ops = DockOps::default();
        ops.push(Some("a".into()), focus(DockTarget::Main, "kept"));
        ops.push(Some("b".into()), focus(DockTarget::Main, "other project"));
        ops.push(None, focus(DockTarget::Main, "no project"));
        ops.push(Some("b".into()), focus(DockTarget::Float(live), "window"));
        ops.push(
            None,
            DockOp::FloatReturn {
                viewport: closed,
                pane: None,
            },
        );
        ops.prune(Some("a"), |viewport| viewport == live);
        assert_eq!(
            ops.iter().cloned().collect::<Vec<_>>(),
            vec![
                focus(DockTarget::Main, "kept"),
                focus(DockTarget::Float(live), "window"),
            ]
        );
        ops.prune(None, |_| false);
        assert!(ops.is_empty());
    }
}
