//! Session row selection in the project sidebar. Cmd/Ctrl toggles a row
//! and Shift selects the painted range from the anchor, like the Explorer
//! (`explorer_click`), but keyed by session id. Bulk actions (move to a
//! tab group, tile, close) act on the selection.

mod actions;
#[cfg(test)]
mod tests;

use eframe::egui;
use std::collections::VecDeque;

#[derive(Default)]
pub(crate) struct SessionSelection {
    ids: Vec<String>,
    anchor: Option<String>,
    /// Session rows in paint order during the last complete frame.
    rows: Vec<String>,
    /// Session rows painted so far in this frame.
    frame_rows: Vec<String>,
    /// Sessions a bulk close still has to ask about, one at a time,
    /// through the normal close flow.
    pub(crate) close_queue: VecDeque<String>,
}

impl SessionSelection {
    /// Start a new paint pass of the session rows.
    pub(crate) fn begin_frame(&mut self) {
        self.rows = std::mem::take(&mut self.frame_rows);
    }

    /// Record that a session row is painted (or skipped as clipped) in
    /// this frame, in order.
    pub(crate) fn painted(&mut self, sid: &str) {
        self.frame_rows.push(sid.to_owned());
    }

    /// Selection bookkeeping for a row click. `current` is the session
    /// the sidebar shows as selected before any multi-selection. Returns
    /// true for a plain click, so the caller opens the session.
    pub(crate) fn click(
        &mut self,
        sid: &str,
        current: Option<&str>,
        modifiers: egui::Modifiers,
    ) -> bool {
        if modifiers.command {
            if self.ids.is_empty()
                && let Some(current) = current.filter(|current| *current != sid)
            {
                self.ids.push(current.to_owned());
            }
            if let Some(at) = self.ids.iter().position(|id| id == sid) {
                self.ids.remove(at);
            } else {
                self.ids.push(sid.to_owned());
            }
            self.anchor = Some(sid.to_owned());
            return false;
        }
        if modifiers.shift
            && let Some(anchor) = self.anchor.clone().or(current.map(str::to_owned))
        {
            self.select_range(&anchor, sid);
            return false;
        }
        self.ids = vec![sid.to_owned()];
        self.anchor = Some(sid.to_owned());
        true
    }

    fn select_range(&mut self, anchor: &str, to: &str) {
        let index = |sid: &str| self.rows.iter().position(|row| row == sid);
        let (Some(a), Some(b)) = (index(anchor), index(to)) else {
            self.ids = vec![to.to_owned()];
            self.anchor = Some(to.to_owned());
            return;
        };
        let (lo, hi) = (a.min(b), a.max(b));
        let mut ids: Vec<String> = Vec::new();
        for sid in self.rows.get(lo..=hi).unwrap_or_default() {
            // A session painted twice (a tab row and its member) counts once.
            if !ids.contains(sid) {
                ids.push(sid.clone());
            }
        }
        self.ids = ids;
    }

    /// The row paints as part of a multi-selection.
    pub(crate) fn selected(&self, sid: &str) -> bool {
        self.ids.len() > 1 && self.ids.iter().any(|id| id == sid)
    }

    /// The selection a menu on `sid` acts on: every selected session when
    /// `sid` is one of several, else none (the row acts alone).
    pub(crate) fn bulk(&self, sid: &str) -> Option<Vec<String>> {
        self.selected(sid).then(|| self.ids.clone())
    }

    /// Drop sessions that are gone or ended.
    pub(crate) fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        self.ids.retain(|id| keep(id));
        if self.anchor.as_deref().is_some_and(|anchor| !keep(anchor)) {
            self.anchor = None;
        }
    }

    pub(crate) fn clear(&mut self) {
        self.ids.clear();
        self.anchor = None;
    }

    /// The next session of a bulk close, once the previous one is done.
    pub(crate) fn next_close(&mut self) -> Option<String> {
        self.close_queue.pop_front()
    }
}
