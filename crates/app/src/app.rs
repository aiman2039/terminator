mod actions;
pub(crate) mod app_state;
mod apply_state;
mod center;
mod client;
mod commands;
mod docking;
mod files;
pub(crate) mod frame;
mod ide;
#[cfg(test)]
mod layout_tests;
mod lifecycle;
#[cfg(test)]
mod nav_01;
#[cfg(test)]
mod nav_02;
#[cfg(test)]
mod nav_03;
#[cfg(test)]
mod nav_04;
#[cfg(test)]
mod nav_05;
#[cfg(test)]
mod nav_06;
#[cfg(test)]
mod nav_07;
#[cfg(test)]
mod nav_08;
#[cfg(test)]
mod nav_09;
#[cfg(test)]
mod nav_10;
#[cfg(test)]
mod nav_11;
#[cfg(test)]
mod nav_common;
#[cfg(test)]
pub(crate) use nav_common::drain_updates;
mod projects;
#[cfg(test)]
mod repaint_tests;
mod strip;
mod tabs;
#[cfg(test)]
mod terminal_find_tests;
pub(crate) mod types;
mod updates;
pub(crate) mod window;
