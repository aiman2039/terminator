//! Workspace and terminal rendering.
mod diff_paint;
#[cfg(test)]
mod diff_tests;
mod drag;
mod header;
mod hover;
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
