//! Tiling several panes into one new top-level tab, and picking editor-only
//! tabs for a bulk close.
use super::*;

/// Most panes "Tile in new tab" places in one tab.
pub(crate) const TILE_MAX: usize = 4;

/// Where the `index`-th pane of a tile goes: beside which earlier pane,
/// and on which side. Two panes sit side by side, a third goes below the
/// first, and a fourth below the second (a 2×2 grid).
fn tile_slot(index: usize) -> Option<(usize, egui_dock::Split)> {
    match index {
        1 => Some((0, egui_dock::Split::Right)),
        2 => Some((0, egui_dock::Split::Below)),
        3 => Some((1, egui_dock::Split::Below)),
        _ => None,
    }
}

impl Workspace {
    /// Puts 2 to [`TILE_MAX`] panes into one new top-level tab, split right
    /// and down. A pane that is in no tab yet (a background terminal)
    /// joins as well. Returns the new tab's id; `None` leaves the layout
    /// unchanged.
    pub fn tile_panes(&mut self, panes: &[Tab]) -> Option<String> {
        let unique: HashSet<&Tab> = panes.iter().collect();
        if !(2..=TILE_MAX).contains(&panes.len()) || unique.len() != panes.len() {
            return None;
        }
        let first = panes.first()?;
        let group = if self.contains(first) {
            self.move_pane_to_new_group_at(first, self.tabs.len())?
        } else {
            let id = terminator_core::id();
            self.add(id.clone(), first.clone());
            id
        };
        for (index, pane) in panes.iter().enumerate().skip(1) {
            let (beside, split) = tile_slot(index)?;
            let anchor = panes.get(beside)?;
            if !self.contains(pane) {
                // A temporary tab that the split move empties and drops.
                self.add(terminator_core::id(), pane.clone());
            }
            let dest = self
                .tabs
                .iter()
                .find(|tab| tab.id == group)?
                .layout
                .find_tab(anchor)?
                .node_path();
            self.move_pane_to_split(MovePaneToSplitInput {
                pane,
                dest_group: &group,
                dest,
                split,
            });
        }
        group.clone_into(&mut self.active);
        self.record_active();
        Some(group)
    }

    /// Unpinned top-level tabs that hold only editors: native editors and
    /// terminals `is_editor` says run an editor.
    pub fn editor_group_ids(&self, is_editor: impl Fn(&str) -> bool) -> Vec<String> {
        self.tabs
            .iter()
            .filter(|tab| !tab.pinned)
            .filter(|tab| {
                let mut panes = tab.layout.iter_all_tabs().peekable();
                panes.peek().is_some()
                    && panes.all(|(_, pane)| match pane {
                        Tab::NativeEditor { .. } => true,
                        Tab::Terminal(sid) => is_editor(sid),
                        _ => false,
                    })
            })
            .map(|tab| tab.id.clone())
            .collect()
    }
}
