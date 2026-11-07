//! Vim window keys from a native editor: Ctrl-W h/j/k/l/w and
//! `:bn` / `:bp` move between panes and the tabs of the focused pane.
use eframe::egui;
use egui_dock::NodeIndex;

use super::super::*;

impl App {
    /// Vim Ctrl-W h/j/k/l: focus the nearest pane on that side of the
    /// focused one (by painted rects); `w` cycles like Next pane. The
    /// keyboard follows into a native editor there.
    pub(crate) fn focus_pane_toward(&mut self, direction: terminator_native_edit::vim::PaneMove) {
        use terminator_native_edit::vim::PaneMove;
        if direction == PaneMove::Next {
            self.focus_next_pane();
        } else {
            let Some(project) = self.selected.clone() else {
                return;
            };
            let Some(dock) = self.layouts.get_mut(&project) else {
                return;
            };
            let tree = dock.main_surface();
            let Some(from) = tree
                .focused_leaf()
                .and_then(|node| tree[node].rect())
                .map(|rect| rect.center())
            else {
                return;
            };
            let toward = |at: egui::Pos2| {
                let delta = egui::vec2(at.x - from.x, at.y - from.y);
                let (along, across) = match direction {
                    PaneMove::Left => (-delta.x, delta.y),
                    PaneMove::Right => (delta.x, delta.y),
                    PaneMove::Up => (-delta.y, delta.x),
                    _ => (delta.y, delta.x),
                };
                (along > 1.0).then_some(along + across.abs() * 2.0)
            };
            let best = tree
                .iter()
                .enumerate()
                .filter(|(_, node)| node.is_leaf())
                .filter_map(|(index, node)| {
                    let score = toward(node.rect()?.center())?;
                    Some((NodeIndex(index), score))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let Some((node, _)) = best else {
                return;
            };
            dock.main_surface_mut().set_focused_node(node);
        }
        self.follow_focused_pane();
    }

    /// Vim `:bn` / `:bp`: the next or previous tab of the focused pane,
    /// wrapping.
    pub(crate) fn cycle_pane_tab(&mut self, forward: bool) {
        let Some(dock) = self
            .selected
            .as_ref()
            .and_then(|id| self.layouts.get_mut(id))
        else {
            return;
        };
        let tree = dock.main_surface_mut();
        let Some(node) = tree.focused_leaf() else {
            return;
        };
        let Some(leaf) = tree[node].get_leaf_mut() else {
            return;
        };
        let count = leaf.tabs.len();
        if count < 2 {
            return;
        }
        let step = if forward { 1 } else { count.saturating_sub(1) };
        leaf.active.0 = leaf
            .active
            .0
            .saturating_add(step)
            .checked_rem(count)
            .unwrap_or(0);
        self.follow_focused_pane();
    }

    /// Move keyboard focus to the active tab of the focused leaf.
    fn follow_focused_pane(&mut self) {
        let Some(project) = self.selected.clone() else {
            return;
        };
        let Some(dock) = self.layouts.remove(&project) else {
            return;
        };
        let tree = dock.main_surface();
        let pane = tree
            .focused_leaf()
            .and_then(|node| tree[node].get_leaf())
            .and_then(|leaf| leaf.tabs.get(leaf.active.0))
            .cloned();
        if let Some(pane) = pane {
            self.focus_moved_view(&pane, &project, &dock);
        }
        self.layouts.insert(project, dock);
    }

    // Focus is `active_session` (the focused terminal) plus
    // `non_terminal_selected` (with no terminal: a real image, browser,
    // diff, player, or editor selection rather than a cleared focus). Write
    // them only through these setters: they keep the pair consistent, so a
    // stale flag never blocks `restore_cleared_focus`.

    /// A terminal has focus.
    pub(crate) fn focus_terminal(&mut self, sid: String) {
        self.active_session = Some(sid);
        self.non_terminal_selected = false;
    }

    /// A non-terminal pane has focus; recovery must not replace it.
    pub(crate) fn focus_non_terminal(&mut self) {
        self.active_session = None;
        self.non_terminal_selected = true;
    }

    /// Nothing has focus; recovery may pick a terminal.
    pub(crate) fn clear_focus(&mut self) {
        self.active_session = None;
        self.non_terminal_selected = false;
    }

    /// Focus follows `tab`: a terminal, another pane, or nothing.
    pub(crate) fn focus_pane(&mut self, tab: Option<&Tab>) {
        match tab {
            Some(Tab::Terminal(sid)) => self.focus_terminal(sid.clone()),
            Some(_) => self.focus_non_terminal(),
            None => self.clear_focus(),
        }
    }
}

#[cfg(test)]
mod focus_tests {
    use super::*;
    use crate::app::tests::support::fixture;

    #[test]
    fn terminal_focus_clears_a_stale_non_terminal_selection() {
        let (mut app, _ctx, _dir) = fixture();
        app.focus_non_terminal();
        app.focus_terminal("one".into());
        assert_eq!(app.active_session.as_deref(), Some("one"));
        assert!(!app.non_terminal_selected);
    }

    #[test]
    fn project_switch_to_an_unfocused_dock_allows_focus_recovery() {
        let (mut app, _ctx, _dir) = fixture();
        app.selected = Some("a".into());
        // An editor had focus in project `a`.
        app.focus_non_terminal();
        app.layouts.insert("b".into(), Workspace::empty());
        app.select_project("b".into());
        assert_eq!(app.selected.as_deref(), Some("b"));
        assert!(app.active_session.is_none());
        assert!(
            !app.non_terminal_selected,
            "a stale non-terminal flag would block restore_cleared_focus"
        );
    }

    /// Focus writes go through the setters above. A direct write can leave
    /// a terminal focused with the non-terminal flag still set.
    #[test]
    fn focus_fields_are_written_only_by_the_setters() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let name = path.to_string_lossy();
                if !name.ends_with(".rs")
                    || name.contains("tests")
                    || name.ends_with("pane_focus.rs")
                    || name.ends_with("lifecycle.rs")
                {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                for (line, code) in text.lines().enumerate() {
                    let code = code.trim_start();
                    if code.contains(".active_session = ")
                        || code.contains(".non_terminal_selected = ")
                    {
                        offenders.push(format!("{name}:{}", line + 1));
                    }
                }
            }
        }
        assert!(offenders.is_empty(), "direct focus writes: {offenders:?}");
    }
}
