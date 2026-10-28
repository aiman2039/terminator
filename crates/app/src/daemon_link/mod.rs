//! GUI-side daemon connection, services and refresh workers.
pub mod daemon_connection;
pub(super) mod metadata_refresh;
pub(super) mod native_jobs;
pub(super) mod nvim_rpc;
pub(super) mod refresh;
pub(super) mod services;
pub(super) mod ui_control;
