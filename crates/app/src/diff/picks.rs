//! Line picks inside one hunk for line-level stage, unstage, and discard:
//! single toggles, press-and-drag ranges, and keyboard extension.
use super::actions::LineSelection;
use super::review::{Hunk, State};

/// A press-and-drag pick: the hunk and side it started on, the first
/// line, and whether the drag picks (`true`) or unpicks lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PickDrag {
    pub hunk: usize,
    pub old: bool,
    pub from: usize,
    pub picking: bool,
}

impl State {
    /// Take the picked lines when they belong to `hunk`.
    pub fn picked_lines(&mut self, hunk: usize) -> Option<LineSelection> {
        match self.picked.take() {
            Some((picked_hunk, lines)) if picked_hunk == hunk && !lines.is_empty() => Some(lines),
            other => {
                self.picked = other;
                None
            }
        }
    }

    /// Number of lines picked in `hunk`.
    pub fn picked_in(&self, hunk: usize) -> usize {
        self.picked
            .as_ref()
            .filter(|(picked_hunk, _)| *picked_hunk == hunk)
            .map_or(0, |(_, lines)| lines.len())
    }

    /// Drop every pick and any drag in progress.
    pub fn clear_picks(&mut self) {
        self.picked = None;
        self.pick_anchor = None;
        self.drag = None;
    }

    /// The picks of `hunk`; picks in another hunk start over.
    fn picks_for(&mut self, hunk: usize) -> &mut LineSelection {
        let (picked_hunk, lines) = self
            .picked
            .get_or_insert_with(|| (hunk, LineSelection::default()));
        if *picked_hunk != hunk {
            *picked_hunk = hunk;
            *lines = LineSelection::default();
        }
        lines
    }

    /// Pick (or unpick) one changed line (`old` side or new side).
    pub fn set_line(&mut self, hunk: usize, (old, line): (bool, usize), picked: bool) {
        let lines = self.picks_for(hunk);
        let set = if old { &mut lines.old } else { &mut lines.new };
        if picked {
            set.insert(line);
        } else {
            set.remove(&line);
        }
        self.pick_anchor = Some((old, line));
    }

    /// Toggle one changed line (`old` side or new side) of `hunk`;
    /// returns whether the line is now picked.
    pub fn toggle_line(&mut self, hunk: usize, old: bool, line: usize) -> bool {
        let picked = self
            .picked
            .as_ref()
            .filter(|(picked_hunk, _)| *picked_hunk == hunk)
            .is_some_and(|(_, lines)| (if old { &lines.old } else { &lines.new }).contains(&line));
        self.set_line(hunk, (old, line), !picked);
        !picked
    }

    /// Start a drag pick on a line: it toggles, and the drag applies the
    /// same state to every line it passes.
    pub fn start_drag(&mut self, hunk: usize, old: bool, line: usize) {
        let picking = self.toggle_line(hunk, old, line);
        self.drag = Some(PickDrag {
            hunk,
            old,
            from: line,
            picking,
        });
    }

    /// Extend the drag to `line` on its side: every line between the
    /// start and `line` takes the drag's state.
    pub fn drag_to(&mut self, line: usize) {
        let Some(drag) = self.drag else {
            return;
        };
        let (low, high) = (drag.from.min(line), drag.from.max(line));
        for each in low..=high {
            self.set_line(drag.hunk, (drag.old, each), drag.picking);
        }
    }

    /// Shift+Down / Shift+Up: pick the next (or previous) changed line of
    /// `hunk` after the anchor, removed lines first, then added lines.
    pub fn extend_pick(&mut self, index: usize, hunk: &Hunk, forward: bool) {
        let order: Vec<(bool, usize)> = hunk
            .old
            .clone()
            .map(|line| (true, line))
            .chain(hunk.new.clone().map(|line| (false, line)))
            .collect();
        let anchor = self
            .pick_anchor
            .filter(|_| self.picked.as_ref().is_some_and(|(h, _)| *h == index))
            .and_then(|anchor| order.iter().position(|line| *line == anchor));
        let next = match anchor {
            None if forward => Some(0),
            None => order.len().checked_sub(1),
            Some(at) if forward => Some(at.saturating_add(1).min(order.len().saturating_sub(1))),
            Some(at) => Some(at.saturating_sub(1)),
        };
        if let Some(&line) = next.and_then(|at| order.get(at)) {
            self.set_line(index, line, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hunk() -> Hunk {
        Hunk {
            old: 2..4,
            new: 2..5,
            unified_row: 0,
            split_row: 0,
        }
    }

    #[test]
    fn dragging_over_lines_picks_the_whole_range() {
        let mut state = State::default();
        state.start_drag(0, false, 2);
        state.drag_to(4);
        assert_eq!(state.picked_in(0), 3);
        // Dragging back narrows nothing it already picked, and a drag
        // that starts on a picked line unpicks.
        state.drag = None;
        state.start_drag(0, false, 3);
        state.drag_to(4);
        assert_eq!(state.picked_in(0), 1);
    }

    #[test]
    fn shift_arrows_extend_picks_through_removed_then_added_lines() {
        let mut state = State::default();
        let hunk = hunk();
        for _ in 0..3 {
            state.extend_pick(0, &hunk, true);
        }
        let (_, lines) = state.picked.clone().expect("picks");
        assert_eq!(lines.old.iter().copied().collect::<Vec<_>>(), [2, 3]);
        assert_eq!(lines.new.iter().copied().collect::<Vec<_>>(), [2]);
        state.extend_pick(0, &hunk, false);
        assert_eq!(state.pick_anchor, Some((true, 3)));
    }

    #[test]
    fn picks_in_another_hunk_start_over_and_clear_resets_everything() {
        let mut state = State::default();
        state.toggle_line(0, true, 2);
        state.toggle_line(1, false, 9);
        assert_eq!(state.picked_in(0), 0);
        assert_eq!(state.picked_in(1), 1);
        state.clear_picks();
        assert!(state.picked.is_none() && state.pick_anchor.is_none());
    }
}
