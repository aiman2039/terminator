#![forbid(unsafe_code)]
mod chrome;
mod link;
mod system;
pub(crate) use chrome::git_log;
pub use chrome::installation;
pub(crate) use chrome::{
    hosts_ui, markdown_images, popup, remote_ui, session_info, settings_controls, shortcuts,
    sidebar_cache, transfers_ui, worktree_ui,
};
pub(crate) use link::{
    daemon_upgrade, forwards_ui, metadata_refresh, native_jobs, nvim_rpc, refresh, services,
    ui_control,
};
#[cfg(feature = "test-support")]
pub(crate) use system::diagnostics;
#[cfg(any(test, target_os = "macos"))]
pub(crate) use system::merge;
#[cfg(any(windows, test))]
pub(crate) use system::windows_updates;
pub(crate) use system::{exit, notify_test, resource_sample, retry_budget};
pub mod console;
pub mod daemon_connection;
use daemon_connection::{can_restart_service, can_retire_daemon};
mod updater {
    pub use terminator_updater::*;
}
mod workspace_ui;
use workspace_ui::Viewer;
mod sidebar_ui;
use sidebar_ui::{AttentionAction, AttentionCard, attention_card};
mod agent_presence;
mod appearance;
mod browser;
mod browser_host;
mod external_editor;
mod file_actions;
mod workspace_ops;
pub(crate) use browser::{BrowserTarget, rewrite_html_tabs};
mod icons;
mod image_preview;
mod lsp_manager;
mod markdown;
mod native_editor;
mod player;
mod settings_ui;
use settings_ui::{BrowseTarget, SettingsSection};
mod palette;
use shortcuts::Action;
mod vnc_ui;
use file_actions::FileAction;
mod editor_close;
mod preferences;
mod workspace;
use preferences::{AgentsTab, SidebarTool, UiPreferences};
use terminator_core::appearance::{AppearanceConfig, AppearanceFile};
use workspace::Workspace;
pub mod app;
pub(crate) use anyhow::Result;
mod clipboard;
pub(crate) mod diff;
mod gui_services;
mod perforce;
mod search;
pub use app::app_state::App;
pub(crate) use app::app_state::{
    inventory_is_stale, is_missing_path_error, is_session_auth_error, observation_is_stale,
    service_failure_update,
};
pub(crate) use app::frame::RepaintProbe;
pub(crate) use app::types::{
    After, FileActivation, GitView, HoverPopup, Job, PaneDropZone, PaneSource, RenameSurface,
    SpawnDiff, Tab, Update,
};
pub(crate) use app::window::{
    FloatingWindow, PaneIndex, TerminalFind, begin_native_window_gesture, header_drag_space,
    window_resize_edges,
};
pub(crate) use eframe::egui::{self, Color32, RichText};
#[cfg(test)]
pub(crate) use egui_dock::DockState;
pub(crate) use egui_dock::tab_viewer::OnCloseResponse;
pub(crate) use egui_dock::{NodeIndex, TabViewer};
pub(crate) use egui_term::TerminalView;
#[cfg(test)]
pub(crate) use std::thread;
pub(crate) use std::{
    path::{Path, PathBuf},
    sync::mpsc::Sender,
    time::{Duration, Instant},
};
pub(crate) use terminator_core::*;

/// Give synthetic test paths a drive prefix on Windows.
#[cfg(test)]
fn test_path(path: &str) -> std::path::PathBuf {
    std::path::absolute(path).expect("absolute test path")
}
