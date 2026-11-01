//! Ctrl+Tab recent-tab switcher: hold the modifier and press Tab to step
//! through the selected project's top-level tabs in most-recent order;
//! release to select, Escape to cancel.
#[cfg(test)]
mod tests;

use crate::shortcuts::{self, Action};
use crate::*;
use eframe::egui::Modifiers;

pub(crate) struct SwitcherItem {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) icon: &'static str,
}

/// Open switcher state. It keeps the project that opened it, so a
/// selection never lands in another project.
pub(crate) struct Switcher {
    pub(crate) project: String,
    pub(crate) items: Vec<SwitcherItem>,
    pub(crate) index: usize,
    /// Modifiers that keep the switcher open while held.
    pub(crate) hold: Modifiers,
}

impl Switcher {
    /// Opens on the next (or, backward, the oldest) item. Fewer than two
    /// items leave nothing to switch to.
    pub(crate) fn open(
        project: String,
        items: Vec<SwitcherItem>,
        backward: bool,
        hold: Modifiers,
    ) -> Option<Self> {
        if items.len() < 2 {
            return None;
        }
        let mut switcher = Self {
            project,
            items,
            index: 0,
            hold,
        };
        switcher.step(backward);
        Some(switcher)
    }

    /// Moves one item, wrapping at both ends.
    pub(crate) fn step(&mut self, backward: bool) {
        let len = self.items.len();
        if len == 0 {
            return;
        }
        let next = if backward {
            self.index.checked_sub(1).unwrap_or(len.saturating_sub(1))
        } else {
            self.index.saturating_add(1)
        };
        self.index = next.checked_rem(len).unwrap_or(0);
    }

    pub(crate) fn selected(&self) -> Option<&str> {
        self.items.get(self.index).map(|item| item.id.as_str())
    }

    /// True while any modifier of the opening chord is still down.
    pub(crate) fn held(&self, current: Modifiers) -> bool {
        let hold = self.hold;
        (hold.ctrl && current.ctrl)
            || (hold.alt && current.alt)
            || (hold.mac_cmd && current.mac_cmd)
            || (hold.command && current.command)
    }
}

impl App {
    /// Ctrl+Tab: open the switcher, or step it when already open.
    pub(crate) fn step_switcher(&mut self, backward: bool) {
        if let Some(switcher) = self.switcher.as_mut() {
            switcher.step(backward);
            return;
        }
        let Some(project) = self.selected.clone() else {
            return;
        };
        let Some(workspace) = self.layouts.get(&project) else {
            return;
        };
        let items = workspace
            .mru_ids()
            .into_iter()
            .filter_map(|id| {
                let group = workspace.tabs.iter().find(|tab| tab.id == id)?;
                let face = self.tab_face(Self::group_primary(group));
                let label = group
                    .name
                    .clone()
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or(face.label);
                Some(SwitcherItem {
                    id,
                    label,
                    icon: face.icon,
                })
            })
            .collect();
        let action = if backward {
            Action::SwitcherBack
        } else {
            Action::SwitcherNext
        };
        let hold = shortcuts::parse(&shortcuts::binding(
            &self.state.settings.keybindings,
            action.id(),
        ))
        .map_or(Modifiers::NONE, |chord| chord.modifiers);
        self.switcher = Switcher::open(project, items, backward, hold);
    }

    /// Start of frame: Escape cancels, Enter or releasing the held
    /// modifier selects.
    pub(crate) fn switcher_input(&mut self, ctx: &egui::Context) {
        let Some(switcher) = self.switcher.as_ref() else {
            return;
        };
        // The held modifier is still down, so match Escape and Enter with
        // any modifiers and keep them from the focused terminal.
        let take = |key| {
            let pressed = ctx.input(|input| input.key_pressed(key));
            if pressed {
                Self::drop_key_events(ctx, key);
            }
            pressed
        };
        if take(egui::Key::Escape) {
            self.switcher = None;
            return;
        }
        let enter = take(egui::Key::Enter);
        if enter || !switcher.held(ctx.input(|input| input.modifiers)) {
            self.commit_switcher();
        }
    }

    /// Activates the selected tab in the project that opened the switcher.
    /// A project change or a closed tab drops the selection.
    pub(crate) fn commit_switcher(&mut self) {
        let Some(switcher) = self.switcher.take() else {
            return;
        };
        if self.selected.as_deref() != Some(switcher.project.as_str()) {
            return;
        }
        let Some(id) = switcher.selected().map(str::to_owned) else {
            return;
        };
        let Some(workspace) = self.layouts.get_mut(&switcher.project) else {
            return;
        };
        if workspace.active == id || !workspace.tabs.iter().any(|tab| tab.id == id) {
            return;
        }
        workspace.active = id;
        workspace.record_active();
        if self.rename_surface == RenameSurface::Pane {
            self.finish_rename(true);
        }
        self.hover_popup = None;
        self.hide_center_overlay();
    }

    pub(crate) fn paint_switcher(&mut self, ctx: &egui::Context) {
        let Some(switcher) = self.switcher.as_ref() else {
            return;
        };
        let mut clicked = None;
        egui::Area::new(egui::Id::unique("tab-switcher"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(320.0);
                    ui.weak("Recent tabs");
                    ui.add_space(4.0);
                    for (index, item) in switcher.items.iter().enumerate() {
                        let row = appearance::row(appearance::RowInput {
                            tint: ui.visuals().text_color(),
                            ui,
                            label: &item.label,
                            icon: item.icon,
                            selected: index == switcher.index,
                            height: 26.0,
                            trailing: "",
                        });
                        if row.clicked() {
                            clicked = Some(index);
                        }
                    }
                });
            });
        if let Some(index) = clicked
            && let Some(switcher) = self.switcher.as_mut()
        {
            switcher.index = index;
            self.commit_switcher();
        }
    }
}
