#![forbid(unsafe_code)]
pub mod daemon_connection;
mod daemon_upgrade;
use daemon_connection::{can_restart_service, can_retire_daemon};
mod exit;
pub mod installation;
mod installation_ui;
mod updater {
    pub use terminator_updater::*;
}
mod workspace_ui;
use workspace_ui::Viewer;
mod dialogs_ui;
mod sidebar_cache;
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
mod markdown_images;
mod metadata_refresh;
mod native_editor;
mod notify_test;
mod nvim_rpc;
mod player;
mod refresh;
mod resource_sample;
mod retry_budget;
mod session_info;
mod settings_ui;
use settings_ui::{BrowseTarget, SettingsSection};
mod palette;
mod settings_controls;
mod shortcuts;
mod ui_control;
mod worktree_ui;
use file_actions::FileAction;
mod close_idle;
mod editor_close;
mod popup;
mod preferences;
#[cfg(any(windows, test))]
mod windows_updates;
mod workspace;
#[cfg(any(test, target_os = "macos"))]
use preferences::AgentsTab;
use preferences::{SidebarTool, UiPreferences};
use terminator_core::appearance::{AppearanceConfig, AppearanceFile};
use workspace::Workspace;
pub mod app;
pub(crate) use anyhow::Result;
mod clipboard;
#[cfg(feature = "test-support")]
mod diagnostics;
pub(crate) mod diff;
pub(crate) mod git_log;
mod gui_services;
#[cfg(any(test, target_os = "macos"))]
mod menu_bar;
mod native_jobs;
mod search;
mod services;
pub use app::app_state::App;
pub(crate) use app::app_state::{
    inventory_is_stale, is_missing_path_error, observation_is_stale, service_failure_update,
};
pub(crate) use app::frame::RepaintProbe;
pub(crate) use app::types::{
    After, FileActivation, HoverPopup, Job, PaneDropZone, RenameSurface, SpawnDiff, Tab, Update,
};
pub(crate) use app::window::{
    FloatingPane, PaneIndex, TerminalFind, begin_native_window_gesture, header_drag_space,
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
