use super::super::*;
use super::header::click_enabled_menu_item;
use super::tab_menu::{WorkspaceTabMenu, WorkspaceTabMenuSpec, click_menu_item, split_action};
impl App {
    pub(super) fn workspace_tab_menu(
        &mut self,
        ui: &mut egui::Ui,
        spec: WorkspaceTabMenuSpec<'_>,
    ) -> WorkspaceTabMenu {
        let WorkspaceTabMenuSpec { sid, index, count } = spec;
        if let Some(sid) = sid {
            self.rename_action(ui, sid, RenameSurface::Workspace);
        }
        let close = click_menu_item(ui, "Close tab…", "X");
        ui.separator();
        let close_all = click_enabled_menu_item(ui, count > 1, "Close all tabs…", "X");
        let close_left = click_enabled_menu_item(ui, index > 0, "Close all tabs to the left…", "X");
        let close_right = click_enabled_menu_item(
            ui,
            index.saturating_add(1) < count,
            "Close all tabs to the right…",
            "X",
        );
        ui.separator();
        let add_left = click_menu_item(ui, "Add tab to the left", "Plus");
        let add_right = click_menu_item(ui, "Add tab to the right", "Plus");
        WorkspaceTabMenu {
            close,
            close_all,
            close_left,
            close_right,
            add_left,
            add_right,
        }
    }
    pub(super) fn new_terminal_menu(
        &mut self,
        ui: &mut egui::Ui,
        pane: Option<egui_dock::NodePath>,
        strip: bool,
    ) {
        for (label, split) in [
            ("New tab", None),
            ("Split up", Some("up")),
            ("Split down", Some("down")),
            ("Split left", Some("left")),
            ("Split right", Some("right")),
        ] {
            let action = split_action(split);
            let icon = match split {
                Some("up") => "PanelTopClose",
                Some("down") => "PanelBottomClose",
                Some("left") => "PanelLeftClose",
                Some("right") => "PanelRightClose",
                _ => "Plus",
            };
            let shortcut = self.shortcut_label(action);
            if appearance::menu_item(ui, label, icon, &shortcut).clicked() {
                if strip {
                    if let Some(pane) = pane {
                        self.add_strip_tab = Some((pane, split.map(str::to_owned)));
                    } else {
                        self.create_strip_split(split);
                    }
                } else if let Some(pane) = pane {
                    self.add_tab = Some((pane, split.map(str::to_owned)));
                } else {
                    self.create(split);
                }
                ui.close();
            }
        }
        let tabs_in_pane = pane.and_then(|pane| {
            if strip {
                self.strip_pane_tabs.get(&pane).cloned()
            } else {
                self.pane_tabs.get(&pane).cloned()
            }
        });
        if let Some(tabs) = tabs_in_pane.filter(|tabs| tabs.len() > 1) {
            ui.separator();
            ui.weak("Tabs in this pane");
            for tab in tabs {
                let label = match &tab {
                    Tab::Terminal(sid) => self
                        .state
                        .sessions
                        .iter()
                        .find(|s| &s.id == sid)
                        .map(|s| s.label.clone())
                        .unwrap_or_else(|| "Terminal".into()),
                    Tab::Diff { path, .. } | Tab::Image { path } => path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    Tab::NativeEditor { path } => path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    Tab::Browser { target, .. } => target.title(),
                    Tab::Player => "Player".into(),
                    Tab::CommitLog { .. } => "Commit Log".into(),
                    Tab::Blame { path, .. } => format!(
                        "Blame {}",
                        path.file_name().unwrap_or_default().to_string_lossy()
                    ),
                };
                if appearance::menu_item(ui, &label, "Terminal", "").clicked() {
                    if strip {
                        self.focus_strip_tab = Some(tab);
                    } else {
                        self.focus_tab = Some(tab);
                    }
                    ui.close();
                }
            }
        }
    }
    pub(crate) fn rename_action(&mut self, ui: &mut egui::Ui, sid: &str, surface: RenameSurface) {
        let shortcut = self.shortcut_label("rename_terminal");
        if appearance::menu_item(ui, "Rename terminal…", "Pencil", &shortcut).clicked() {
            self.begin_rename(sid, surface);
            ui.close();
        }
    }
    pub(crate) fn inline_rename(
        &mut self,
        ui: &mut egui::Ui,
        sid: &str,
        surface: RenameSurface,
        rect: egui::Rect,
    ) {
        if !self.renaming(sid, surface) {
            return;
        }
        let Some(title) = self.rename_session.as_ref().map(|(_, title)| title.clone()) else {
            return;
        };
        let mut title = title;
        let starting = self.rename_focus;
        let response = ui.put(
            rect,
            appearance::singleline(&mut title)
                .id_salt(("inline-terminal-title", sid, surface))
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::ZERO)
                .desired_width(rect.width()),
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "rename-input", response.rect);
        self.rename_painted = true;
        self.rename_field_id = Some(response.id);
        if starting {
            response.request_focus();
            self.rename_focus = false;
            if let Some(mut state) = egui::text_edit::TextEditState::load(ui.ctx(), response.id) {
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(title.chars().count()),
                    )));
                state.store(ui.ctx(), response.id);
            }
        }
        let valid = !title.trim().is_empty() && title.trim().len() <= 256;
        let escape = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let enter = ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter));
        // The field owns this frame's keyboard input, including when Enter or
        // blur commits it before a terminal widget is rendered later in the frame.
        ui.input_mut(|input| {
            input.events.retain(|event| {
                !matches!(
                    event,
                    egui::Event::Text(_)
                        | egui::Event::Paste(_)
                        | egui::Event::Copy
                        | egui::Event::Cut
                        | egui::Event::Key { .. }
                )
            });
        });
        if escape || (!starting && response.lost_focus() && !valid) {
            self.rename_session = None;
            response.surrender_focus();
        } else if valid && (enter || (!starting && response.lost_focus())) {
            self.send(Request::Rename {
                session: sid.into(),
                label: title.trim().into(),
            });
            self.rename_session = None;
            response.surrender_focus();
        } else {
            self.rename_session = Some((sid.into(), title));
            if enter {
                response.request_focus();
            }
            response.on_hover_text(if valid {
                "Enter to save · Esc to cancel"
            } else {
                "Enter a non-empty, shorter title"
            });
        }
    }
}
