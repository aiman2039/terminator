//! Workspace and terminal rendering.
mod diff_controls;
mod diff_edit;
mod diff_paint;
#[cfg(test)]
mod diff_tests;
mod diff_view;
mod drag;
mod header;
mod hover;
pub(crate) use hover::link_menu_modifier_pressed;
mod menus;
mod tab_menu;
mod tabs;
mod terminal_panes;
mod terminal_view;
mod viewer;
mod views;
mod window_header;
mod workspace_bar;
#[cfg(test)]
mod workspace_tests;
pub(crate) use viewer::Viewer;

/// egui 0.37 keeps [`egui::scroll_area::State::offset`] private.
/// Serialize, replace the offset, and load the same state back.
pub(crate) fn write_scroll_offset(
    state: &mut eframe::egui::scroll_area::State,
    offset: eframe::egui::Vec2,
) {
    let Ok(mut value) = serde_json::to_value(*state) else {
        return;
    };
    let Some(map) = value.as_object_mut() else {
        return;
    };
    map.insert(
        "offset".to_owned(),
        serde_json::json!({ "x": offset.x, "y": offset.y }),
    );
    if let Ok(next) = serde_json::from_value(value) {
        *state = next;
    }
}
