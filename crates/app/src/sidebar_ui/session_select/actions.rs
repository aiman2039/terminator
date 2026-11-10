//! What the session rows do with one session or with the selection: open
//! beside the focused pane, move into a tab group, tile, and close.
use crate::{App, Tab, appearance, workspace::Workspace};
use eframe::egui;
use terminator_core::{Request, Session};

impl App {
    /// The session can join the main dock of its project: it is live and
    /// not shown by the IDE strip or a floating window (those keep it).
    fn dockable_session(&self, sid: &str) -> Option<Session> {
        let session = self
            .state
            .sessions
            .iter()
            .find(|s| s.id == sid && s.lifecycle.live())?;
        (!self.is_strip_session(&session.project_id, sid) && !self.is_floated_session(sid))
            .then(|| session.clone())
    }

    /// Opens the session beside the focused pane of its project's active
    /// tab: a pane in the dock moves there, a background session attaches
    /// there. A strip or floating session only gets focus.
    pub(crate) fn open_session_in_split(&mut self, sid: &str, split: egui_dock::Split) {
        let Some(session) = self.dockable_session(sid) else {
            self.go_session(sid);
            return;
        };
        let project = session.project_id.clone();
        self.select_project(project.clone());
        self.hide_center_overlay();
        let pane = Tab::Terminal(sid.to_owned());
        let workspace = self
            .layouts
            .entry(project.clone())
            .or_insert_with(Workspace::empty);
        let dest = workspace
            .main_surface()
            .focused_leaf()
            .map(|node| egui_dock::NodePath {
                surface: egui_dock::SurfaceIndex::main(),
                node,
            })
            .or_else(|| workspace.iter_leaves().next().map(|(path, _)| path));
        let group = workspace.active.clone();
        let moved = workspace.contains(&pane)
            && dest.is_some_and(|dest| {
                workspace.move_pane_to_split(crate::workspace::MovePaneToSplitInput {
                    pane: &pane,
                    dest_group: &group,
                    dest,
                    split,
                })
            });
        if !moved && !workspace.contains(&pane) {
            let direction = match split {
                egui_dock::Split::Below => "down",
                egui_dock::Split::Above => "up",
                egui_dock::Split::Left => "left",
                egui_dock::Split::Right => "right",
            };
            self.insert(&project, pane, Some(direction));
        }
        self.after_session_layout_change(sid);
    }

    /// Focus the session that moved and save the layout.
    fn after_session_layout_change(&mut self, sid: &str) {
        self.pane_index = None;
        self.pending_layout_save = true;
        self.focus_terminal(sid.to_owned());
        self.send(Request::Focus {
            session: sid.to_owned(),
        });
    }

    /// Selected sessions of `project` that can be tiled, in selection order.
    fn tileable(&self, sids: &[String]) -> Option<(String, Vec<String>)> {
        let sessions: Vec<Session> = sids
            .iter()
            .map(|sid| self.dockable_session(sid))
            .collect::<Option<_>>()?;
        let project = sessions.first()?.project_id.clone();
        (sessions.iter().all(|s| s.project_id == project)
            && (2..=crate::workspace::TILE_MAX).contains(&sessions.len()))
        .then(|| (project, sids.to_vec()))
    }

    /// Puts the sessions into one new top-level tab, split right and down.
    pub(crate) fn tile_sessions(&mut self, sids: &[String]) -> bool {
        let Some((project, sids)) = self.tileable(sids) else {
            return false;
        };
        let panes: Vec<Tab> = sids.iter().map(|sid| Tab::Terminal(sid.clone())).collect();
        let tiled = self
            .layouts
            .entry(project.clone())
            .or_insert_with(Workspace::empty)
            .tile_panes(&panes)
            .is_some();
        if tiled {
            self.select_project(project);
            self.hide_center_overlay();
            if let Some(first) = sids.first() {
                self.after_session_layout_change(first);
            }
        }
        tiled
    }

    /// Moves each session of `project` into the tab group `tab_id`. A
    /// background session attaches to that tab's focused pane.
    pub(crate) fn move_sessions_to_tab_group(&mut self, project: &str, sids: &[String], tab: &str) {
        for sid in sids {
            let pane = Tab::Terminal(sid.clone());
            let in_dock = self
                .layouts
                .get(project)
                .is_some_and(|workspace| workspace.contains(&pane));
            if in_dock {
                self.move_session_to_tab_group(project, sid, tab);
            } else if self.dockable_session(sid).is_some()
                && let Some(workspace) = self.layouts.get_mut(project)
                && workspace.tabs.iter().any(|group| group.id == tab)
            {
                tab.clone_into(&mut workspace.active);
                workspace.push_to_focused_leaf(pane);
                self.pending_layout_save = true;
                self.pane_index = None;
            }
        }
    }

    /// Asks about each live session in turn through the normal close flow
    /// (keep running in the background, or terminate).
    pub(crate) fn close_sessions(&mut self, sids: &[String]) {
        let live = sids.iter().filter(|sid| {
            self.state
                .sessions
                .iter()
                .any(|s| &s.id == *sid && s.lifecycle.live())
        });
        self.session_selection.close_queue = live.cloned().collect();
        self.session_selection.clear();
        if self.close_session.is_none() {
            self.close_session = self.session_selection.next_close();
        }
    }

    /// Starts the next queued close once the current one is answered.
    pub(crate) fn advance_session_close_queue(&mut self) {
        if self.close_session.is_none() {
            self.close_session = self.session_selection.next_close();
        }
    }

    /// The row menu for a multi-selection.
    pub(crate) fn session_bulk_menu(&mut self, ui: &mut egui::Ui, project: &str, sids: &[String]) {
        ui.weak(format!("{} sessions selected", sids.len()));
        let tileable = self.tileable(sids).is_some();
        let tile = ui
            .add_enabled_ui(tileable, |ui| {
                appearance::menu_item(appearance::MenuItemInput {
                    ui,
                    label: "Tile in new tab",
                    icon: "LayoutDashboard",
                    shortcut: "",
                })
            })
            .inner
            .on_disabled_hover_text("Select 2 to 4 live sessions of one project.");
        if tile.clicked() {
            self.tile_sessions(sids);
            ui.close();
        }
        let groups: Vec<(String, String)> = self
            .layouts
            .get(project)
            .map(|workspace| {
                workspace
                    .tabs
                    .iter()
                    .enumerate()
                    .map(|(index, group)| {
                        let name =
                            crate::workspace::group_display_name(group.name.as_deref(), index);
                        (group.id.clone(), name)
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !groups.is_empty() {
            appearance::menu_button(ui, "Move to tab group", |ui| {
                for (id, name) in &groups {
                    if appearance::menu_item(appearance::MenuItemInput {
                        ui,
                        label: name,
                        icon: "LayoutDashboard",
                        shortcut: "",
                    })
                    .clicked()
                    {
                        self.move_sessions_to_tab_group(project, sids, id);
                        ui.close();
                    }
                }
            });
        }
        ui.separator();
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: &format!("Close {} sessions…", sids.len()),
            icon: "X",
            shortcut: "",
        })
        .clicked()
        {
            self.close_sessions(sids);
            ui.close();
        }
    }
}
