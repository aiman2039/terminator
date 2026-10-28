use super::super::*;
pub(super) struct WorkspaceTabMenu {
    pub(super) toggle_pin: bool,
    pub(super) close: bool,
    pub(super) close_all: bool,
    pub(super) close_left: bool,
    pub(super) close_right: bool,
    pub(super) add_left: bool,
    pub(super) add_right: bool,
}

pub(super) struct WorkspaceTabMenuSpec<'a> {
    pub(super) sid: Option<&'a str>,
    pub(super) index: usize,
    pub(super) count: usize,
    pub(super) pinned: bool,
}

pub(super) fn split_action(direction: Option<&str>) -> Action {
    match direction {
        Some("up") => Action::SplitUp,
        Some("down") => Action::SplitDown,
        Some("left") => Action::SplitLeft,
        Some("right") => Action::SplitRight,
        _ => Action::NewTerminal,
    }
}

pub(super) fn click_menu_item(ui: &mut egui::Ui, label: &str, icon: &str) -> bool {
    let clicked = appearance::menu_item(appearance::MenuItemInput {
        ui,
        label,
        icon,
        shortcut: "",
    })
    .clicked();
    if clicked {
        ui.close();
    }
    clicked
}
