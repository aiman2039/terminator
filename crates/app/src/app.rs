mod actions;
pub(crate) mod app_state;
mod apply_state;
mod center;
mod client;
mod commands;
mod docking;
pub(crate) mod drag_state;
#[cfg(test)]
mod editor_drag_tests;
#[cfg(test)]
mod file_reuse_tests;
mod files;
pub(crate) mod frame;
pub(crate) mod git_panel_state;
mod ide;
#[cfg(test)]
mod layout_tests;
mod lifecycle;
pub(crate) mod palette_state;
mod projects;
#[cfg(test)]
mod repaint_tests;
mod strip;
mod tabs;
#[cfg(test)]
mod terminal_find_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use tests::support::{drain_updates, painted_text};
pub(crate) mod types;
mod updates;
#[cfg(test)]
mod vnc_tests;
pub(crate) mod window;
