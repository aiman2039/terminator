//! View-addressed moves preserve same-file editor copies.
use super::model::Workspace;
use crate::{PaneSource, Tab};
use egui_dock::{Node, NodePath, TabInsert};

pub(crate) struct MoveViewToLeafInput<'a> {
    pub(crate) source: &'a PaneSource,
    pub(crate) pane: &'a Tab,
    pub(crate) group: &'a str,
    pub(crate) dest: NodePath,
    pub(crate) insertion: TabInsert,
}

pub(crate) struct MoveViewSameGroupInput<'a> {
    pub(crate) src: usize,
    pub(crate) source: &'a PaneSource,
    pub(crate) dest: NodePath,
    pub(crate) insertion: TabInsert,
}
pub(crate) struct MoveViewCrossGroupInput<'a> {
    pub(crate) source: &'a PaneSource,
    pub(crate) pane: &'a Tab,
    pub(crate) dest: NodePath,
    pub(crate) insertion: TabInsert,
    pub(crate) src: usize,
    pub(crate) dst: usize,
}

impl Workspace {
    pub(crate) fn view_exists(&self, source: &PaneSource, pane: &Tab) -> bool {
        self.tabs
            .iter()
            .find(|group| group.id == source.group)
            .and_then(|group| group.layout.leaf(source.path.node_path()).ok())
            .and_then(|leaf| leaf.tabs.get(source.path.tab.0))
            == Some(pane)
    }

    pub(crate) fn take_view(&mut self, source: &PaneSource, pane: &Tab) -> Option<String> {
        if !self.view_exists(source, pane) {
            return None;
        }
        let group = self
            .tabs
            .iter_mut()
            .find(|group| group.id == source.group)?;
        group.layout.remove_tab(source.path)?;
        group.primary = group
            .layout
            .iter_all_tabs()
            .next()
            .map(|(_, pane)| pane.clone());
        self.drop_empty_tabs();
        Some(source.group.clone())
    }

    pub(crate) fn move_view_to_group(
        &mut self,
        source: &PaneSource,
        pane: &Tab,
        group: &str,
    ) -> bool {
        if group == source.group {
            if !self.view_exists(source, pane) {
                return false;
            }
            let Some(target) = self.tabs.iter_mut().find(|tab| tab.id == group) else {
                return false;
            };
            let _ = target.layout.set_active_tab(source.path);
            target
                .layout
                .set_focused_node_and_surface(source.path.node_path());
            group.clone_into(&mut self.active);
            return true;
        }
        let Some(target) = self.tabs.iter().find(|tab| tab.id == group) else {
            return false;
        };
        let dest = target
            .layout
            .main_surface()
            .focused_leaf()
            .map(|node| NodePath {
                surface: egui_dock::SurfaceIndex::main(),
                node,
            })
            .filter(|path| target.layout.leaf(*path).is_ok())
            .or_else(|| target.layout.iter_leaves().next().map(|(path, _)| path));
        dest.is_some_and(|dest| {
            self.move_view_to_leaf(crate::workspace::MoveViewToLeafInput { source, pane, group, dest, insertion: TabInsert::Append })
        })
    }

    pub(crate) fn move_view_to_leaf(&mut self, input: MoveViewToLeafInput<'_>) -> bool {
        let MoveViewToLeafInput { source, pane, group, dest, insertion } = input;

        if !self.view_exists(source, pane) {
            return false;
        }
        let Some(dst) = self
            .tabs
            .iter()
            .position(|tab| tab.id == group && tab.layout.leaf(dest).is_ok())
        else {
            return false;
        };
        let Some(src) = self.tabs.iter().position(|tab| tab.id == source.group) else {
            return false;
        };
        if src == dst {
            self.move_view_same_group(MoveViewSameGroupInput { src, source, dest, insertion });
        } else {
            self.move_view_cross_group(MoveViewCrossGroupInput { source, pane, dest, insertion, src, dst });
        }
        self.version = self.version.max(pane.layout_version());
        group.clone_into(&mut self.active);
        self.drop_empty_tabs();
        true
    }

    fn move_view_same_group(&mut self, input: MoveViewSameGroupInput<'_>) {
        let MoveViewSameGroupInput { src, source, dest, mut insertion } = input;
            let Some(tab) = self.tabs.get_mut(src) else {
                return;
            };
            if source.path.node_path() == dest
                && let TabInsert::Insert(index) = &mut insertion
                && source.path.tab.0 < index.0
            {
                index.0 = index.0.saturating_sub(1);
            }
            tab.layout.set_focused_node_and_surface(dest);
            tab.layout.move_tab(source.path, (dest, insertion));
    }

    fn move_view_cross_group(&mut self, input: MoveViewCrossGroupInput<'_>) {
        let MoveViewCrossGroupInput { source, pane, dest, insertion, src, dst } = input;
            // Insert before pruning the source group so destination indices
            // stay valid. All target/source addresses were checked above.
            if let Some(tab) = self.tabs.get_mut(src) {
                tab.layout.remove_tab(source.path);
                tab.primary = tab
                    .layout
                    .iter_all_tabs()
                    .next()
                    .map(|(_, pane)| pane.clone());
            }
            if let Some(tab) = self.tabs.get_mut(dst) {
                if let TabInsert::Split(split) = insertion {
                    let placed = tab.layout.split(dest, split, 0.5, Node::leaf(pane.clone()));
                    tab.layout.set_focused_node_and_surface(NodePath {
                        surface: dest.surface,
                        node: placed[1],
                    });
                } else {
                    if let Ok(leaf) = tab.layout.leaf_mut(dest) {
                        match insertion {
                            TabInsert::Insert(index) => {
                                leaf.insert_tab(index.0.min(leaf.tabs.len()), pane.clone())
                            }
                            _ => leaf.append_tab(pane.clone()),
                        }
                    }
                    tab.layout.set_focused_node_and_surface(dest);
                }
            }
    }

    pub(crate) fn move_view_to_new_group(
        &mut self,
        source: &PaneSource,
        pane: &Tab,
        index: usize,
    ) -> Option<String> {
        if !self.view_exists(source, pane) {
            return None;
        }
        let src = self.tabs.iter().position(|tab| tab.id == source.group)?;
        self.take_view(source, pane)?;
        let index = if !self.tabs.iter().any(|tab| tab.id == source.group) && src < index {
            index.saturating_sub(1)
        } else {
            index
        };
        let id = terminator_core::id();
        self.add_at(index.min(self.tabs.len()), id.clone(), pane.clone());
        Some(id)
    }
}
