//! Browser tab extras: page zoom, find in page, reopen a closed tab, and
//! the keys that reach them while a Browser tab is the focused pane.
use super::super::super::*;
use crate::browser::extras::{self, FindBar, ZoomChange};

impl App {
    /// Key of the selected project's focused pane when it is a Browser tab.
    pub(crate) fn active_browser_key(&self) -> Option<String> {
        let pane = self.layouts.get(self.selected.as_ref()?)?.active_pane()?;
        matches!(pane, Tab::Browser { .. }).then(|| pane.key())
    }

    /// Cmd +/-/0 and Cmd+F (Ctrl on Linux) while a Browser tab is the
    /// focused pane. A page that holds the native keyboard focus takes its
    /// own keys; the toolbar menu has the same commands.
    pub(crate) fn browser_keys(&mut self, ctx: &egui::Context) {
        // Same gate as other shortcuts: dialogs, palette and pickers win.
        if !self.shortcut_allowed(Action::NewBrowser) {
            return;
        }
        let Some(key) = self.active_browser_key() else {
            return;
        };
        let find_id = find_field_id(&key);
        let find_focused = ctx.memory(|memory| memory.has_focus(find_id));
        if shortcuts::terminal_has_focus(ctx) || (self.text_input_focused(ctx) && !find_focused) {
            return;
        }
        let command = egui::Modifiers::COMMAND;
        let pressed = |key: egui::Key| ctx.input_mut(|input| input.consume_key(command, key));
        let change = if pressed(egui::Key::Equals) || pressed(egui::Key::Plus) {
            Some(ZoomChange::In)
        } else if pressed(egui::Key::Minus) {
            Some(ZoomChange::Out)
        } else if pressed(egui::Key::Num0) {
            Some(ZoomChange::Reset)
        } else {
            None
        };
        if let Some(change) = change {
            self.zoom_browser(&key, change);
        }
        if pressed(egui::Key::F) {
            self.open_browser_find(&key);
        }
    }

    pub(crate) fn zoom_browser(&mut self, key: &str, change: ZoomChange) {
        let level = extras::zoom_step(self.browser_host.zoom(key), change);
        self.browser_host.set_zoom(key, level);
    }

    pub(crate) fn open_browser_find(&mut self, key: &str) {
        self.browser_extras.find.entry(key.to_owned()).or_default();
        self.browser_extras.focus_find = Some(key.to_owned());
    }

    /// Reopen the newest closed Browser tab in its project, or in the
    /// selected one when that project is gone.
    pub(crate) fn reopen_browser_tab(&mut self) {
        let Some((project, target)) = self.browser_extras.closed.pop() else {
            return;
        };
        let project = if self.layouts.contains_key(&project) {
            project
        } else if let Some(selected) = self.selected.clone() {
            selected
        } else {
            return;
        };
        match target {
            BrowserTarget::File(path) => self.open_html(&project, path, None),
            BrowserTarget::Url(url) => {
                if let Err(error) = self.open_browser_url(&project, &url, None) {
                    self.error.set(format!("Could not open {url}: {error:#}"));
                }
            }
        }
    }

    /// Find bar under the toolbar while open: Enter finds the next match,
    /// Shift+Enter the previous one, Esc closes it.
    pub(super) fn browser_find_bar(&mut self, ui: &mut egui::Ui, key: &str) {
        let Some(mut find) = self.browser_extras.find.remove(key) else {
            return;
        };
        let id = find_field_id(key);
        let mut step = None;
        let mut close = false;
        ui.horizontal(|ui| {
            let response = ui.add(
                appearance::singleline(&mut find.query)
                    .id(id)
                    .hint_text("Find in page")
                    .desired_width(220.0),
            );
            if self.browser_extras.focus_find.as_deref() == Some(key) {
                response.request_focus();
                self.browser_extras.focus_find = None;
            }
            let enter = response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if enter {
                step = Some(ui.input(|i| i.modifiers.shift));
                response.request_focus();
            }
            if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
            if appearance::sidebar_action(ui, "ArrowUp", "Previous match").clicked() {
                step = Some(true);
            }
            if appearance::sidebar_action(ui, "ArrowDown", "Next match").clicked() {
                step = Some(false);
            }
            close |= appearance::sidebar_action(ui, "X", "Close find bar").clicked();
        });
        if close {
            self.browser_host.run_script(key, extras::FIND_CLEAR_SCRIPT);
            return;
        }
        self.send_find(key, &mut find, step);
        self.browser_extras.find.insert(key.to_owned(), find);
    }

    /// A changed query searches again from the top; a step moves on.
    fn send_find(&self, key: &str, find: &mut FindBar, step: Option<bool>) {
        if find.query.is_empty() {
            find.sent.clear();
            return;
        }
        let fresh = find.query != find.sent;
        if fresh || step.is_some() {
            let backwards = step.unwrap_or(false);
            self.browser_host
                .run_script(key, &extras::find_script(&find.query, backwards, fresh));
            find.sent.clone_from(&find.query);
        }
    }
}

fn find_field_id(key: &str) -> egui::Id {
    egui::Id::unique(("browser-find", key))
}
