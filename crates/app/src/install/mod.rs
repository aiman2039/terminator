//! Installation, self-update and daemon upgrades.
pub(super) mod daemon_upgrade;
pub mod installation;
pub(super) mod installation_ui;
#[cfg(any(windows, test))]
pub(super) mod windows_updates;
