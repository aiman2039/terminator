//! Inline rename fields: a session title, a sidebar tab group, or a project
//! name. Only one is active at a time: starting one commits the others.
//! Every field that did not paint is caught at frame end, so a hidden field
//! never keeps terminal input off.

use eframe::egui;
use terminator_core::*;

use super::super::*;

/// Sidebar rename of a workspace tab group. GUI-local; the name lives on
/// the workspace tab.
pub(crate) struct GroupRename {
    pub(crate) project: String,
    pub(crate) tab: String,
    pub(crate) text: String,
    /// The next paint focuses the field.
    pub(crate) focus: bool,
    /// The field painted during this frame. Starts true, because the menu
    /// that starts a rename paints after the field slot.
    pub(crate) painted: bool,
}

impl App {
    pub(crate) fn begin_rename(&mut self, sid: &str, surface: RenameSurface) {
        let Some(label) = self
            .state
            .sessions
            .iter()
            .find(|s| s.id == sid)
            .map(|s| s.label.clone())
        else {
            return;
        };
        self.finish_group_rename(true);
        self.finish_project_rename(true);
        self.clear_session_rename();
        self.rename_session = Some((sid.into(), label));
        self.rename_focus = true;
        self.rename_surface = surface;
    }

    /// Starts naming the tab group `tab` of `project` with `text`.
    pub(crate) fn begin_group_rename(&mut self, project: &str, tab: &str, text: String) {
        self.finish_rename(true);
        self.finish_project_rename(true);
        self.rename_group = Some(GroupRename {
            project: project.to_owned(),
            tab: tab.to_owned(),
            text,
            focus: true,
            painted: true,
        });
    }

    /// Ends the group rename. `save` stores a non-empty name, or clears the
    /// name when the text is empty, if the tab still exists.
    pub(crate) fn finish_group_rename(&mut self, save: bool) {
        let Some(rename) = self.rename_group.take() else {
            return;
        };
        let exists = self
            .layouts
            .get(&rename.project)
            .is_some_and(|w| w.tabs.iter().any(|g| g.id == rename.tab));
        if !save || !exists {
            return;
        }
        let name = rename.text.trim();
        self.set_tab_group_name(
            &rename.project,
            &rename.tab,
            (!name.is_empty()).then(|| name.chars().take(64).collect()),
        );
        self.save_layouts();
    }

    /// Terminals, shortcuts, and browser panes stay live unless the rename
    /// field is actually going to paint. The field strips keys only once it
    /// paints, and the IDE strip paints first, so a visible field still has
    /// to block input up front.
    pub(crate) fn rename_blocks_input(&self) -> bool {
        self.project_names.draft.is_some()
            || self.rename_group.is_some()
            || (self.rename_session.is_some()
                && self.rename_field_can_show()
                && !self.rename_missed)
    }

    pub(crate) fn rename_field_can_show(&self) -> bool {
        let Some((sid, _)) = &self.rename_session else {
            return false;
        };
        let sid = sid.clone();
        match self.rename_surface {
            // The workspace tab strip stays up over center panes. A tab that
            // scrolls out of view is caught by `rename_missed`.
            RenameSurface::Workspace => true,
            RenameSurface::Pane => self.pane_rename_can_show(&sid),
            RenameSurface::Sidebar => self.sidebar_rename_can_show(&sid),
        }
    }

    /// Ends the session rename without saving and resets its field state.
    /// Returns the draft that was open.
    pub(crate) fn clear_session_rename(&mut self) -> Option<(String, String)> {
        self.rename_focus = false;
        self.rename_seen = false;
        self.rename_painted = false;
        self.rename_missed = false;
        self.rename_field_id = None;
        self.rename_session.take()
    }

    /// A hidden field must not keep egui focus, or the terminal will not take
    /// keys. The draft stays; the next paint focuses the field instead of
    /// treating the gap as a blur commit.
    pub(crate) fn suspend_hidden_rename(&mut self, ctx: &egui::Context) {
        if self.rename_session.is_none() {
            self.clear_session_rename();
            return;
        }
        if self.rename_field_can_show() {
            return;
        }
        self.rename_missed = false;
        if let Some(id) = self.rename_field_id.take() {
            ctx.memory_mut(|memory| memory.surrender_focus(id));
        }
        self.rename_focus = true;
    }

    pub(crate) fn note_rename_frame(&mut self, ctx: &egui::Context) {
        // A project or group rename field that did not paint (sidebar
        // hidden, scrolled away, or its tab closed) commits, so it never
        // blocks input.
        match self.project_names.draft.as_mut() {
            Some(draft) if draft.painted => draft.painted = false,
            Some(_) => self.finish_project_rename(true),
            None => {}
        }
        match self.rename_group.as_mut() {
            Some(rename) if rename.painted => rename.painted = false,
            Some(_) => self.finish_group_rename(true),
            None => {}
        }
        if self.rename_session.is_none() {
            return;
        }
        if !self.rename_seen {
            self.rename_seen = true;
            self.rename_painted = false;
            return;
        }
        if self.rename_painted {
            self.rename_missed = false;
        } else if self.rename_field_can_show() {
            self.rename_missed = true;
            if let Some(id) = self.rename_field_id.take() {
                ctx.memory_mut(|memory| memory.surrender_focus(id));
            }
            self.rename_focus = true;
        }
        self.rename_painted = false;
    }

    pub(crate) fn finish_rename(&mut self, save: bool) {
        if let Some((sid, title)) = self.clear_session_rename()
            && save
            && !title.trim().is_empty()
            && title.trim().len() <= 256
        {
            self.send(Request::Rename {
                session: sid,
                label: title.trim().into(),
            });
        }
    }

    pub(crate) fn renaming(&self, sid: &str, surface: RenameSurface) -> bool {
        self.rename_surface == surface
            && self
                .rename_session
                .as_ref()
                .is_some_and(|(target, _)| target == sid)
    }
}
