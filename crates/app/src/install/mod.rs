//! Installation, self-update and daemon upgrades.
pub(super) mod daemon_upgrade;
pub mod installation;
pub(super) mod installation_ui;
#[cfg(any(not(target_os = "macos"), test))]
pub(super) mod windows_updates;
