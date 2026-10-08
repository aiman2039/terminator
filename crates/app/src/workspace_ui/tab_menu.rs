use super::super::*;
pub(super) struct WorkspaceTabMenu {
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
}

pub(super) fn split_action(direction: Option<&str>) -> &'static str {
    match direction {
        Some("up") => "split_up",
        Some("down") => "split_down",
        Some("left") => "split_left",
        Some("right") => "split_right",
        _ => "new_terminal",
    }
}

pub(super) fn click_menu_item(ui: &mut egui::Ui, label: &str, icon: &str) -> bool {
    let clicked = appearance::menu_item(ui, label, icon, "").clicked();
    if clicked {
        ui.close();
    }
    clicked
}
