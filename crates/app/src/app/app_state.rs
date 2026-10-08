use super::super::ui_control;
use anyhow::Result;
use eframe::egui::{self};
use egui_term::{PtyEvent, TerminalBackend};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::mpsc::{Receiver, Sender},
    time::Instant,
};
use terminator_core::*;

use super::super::*;
pub struct App {
    pub(crate) file_activation: Option<FileActivation>,
    pub(crate) git_commit: String,
    pub(crate) git_history: bool,
    pub(crate) git_branches: Vec<String>,
    pub(crate) git_log: Vec<(String, String)>,
    pub(crate) git_list_root: Option<PathBuf>,
    pub(crate) git_collapse: u64,
    /// Branch comparison for the current Git root (upstream, ahead/behind,
    /// and files committed since the base ref).
    pub(crate) git_compare: Option<workspace_ops::CompareData>,
    pub(crate) git_compare_root: Option<PathBuf>,
    /// User-picked base ref; falls back to the upstream branch when `None`.
    pub(crate) git_base_ref: Option<String>,
    /// Explorer search results (Contents mode) and the query that produced them.
    pub(crate) explorer_search: Vec<search::Hit>,
    pub(crate) explorer_search_error: Option<String>,
    pub(crate) explorer_search_generation: u64,
    pub(crate) explorer_search_pending: bool,
    pub(crate) explorer_search_last: Option<search::Query>,
    pub(crate) explorer_query: String,
    pub(crate) name_prompt: Option<workspace_ops::NamePrompt>,
    /// The name field takes keyboard focus on its next paint, then this clears.
    pub(crate) name_prompt_focus: bool,
    pub(crate) pending_delete: Option<PathBuf>,
    pub(crate) services: gui_services::Services,
    pub(crate) service_owner: gui_services::Owner,
    pub(crate) service_completion: Option<async_service::Completion<Vec<Update>>>,
    pub(crate) service_ready: std::collections::VecDeque<Update>,
    pub(crate) ui_service_peak_ms: f64,
    pub(crate) installation_error: Option<String>,
    pub(crate) repair_pending: bool,
    pub(crate) restart_pending: bool,
    pub(crate) service_start_pending: bool,
    pub(crate) restart_confirm: bool,
    pub(crate) automatic_repair_attempt: Option<String>,
    pub(crate) exit: exit::Exit,
    pub(crate) exit_attempt: u64,
    pub(crate) updater: updater::Updater,
    #[cfg(any(windows, test))]
    pub(crate) windows_updates: windows_updates::WindowsUpdateCheck,
    /// Last waiting count painted on the macOS menu-bar icon.
    #[cfg(all(not(test), target_os = "macos"))]
    pub(crate) status_waiting_shown: Option<usize>,
    #[cfg(feature = "test-support")]
    pub(crate) diagnostics: diagnostics::Diagnostics,
    pub(crate) paths: Paths,
    pub(crate) preferences: UiPreferences,
    pub(crate) preferences_saved: UiPreferences,
    pub(crate) preferences_writable: bool,
    pub(crate) preferences_pending: bool,
    pub(crate) project_width: f32,
    pub(crate) migration_requested: bool,
    pub(crate) attention_requested: Option<Instant>,
    pub(crate) attention_pending: bool,
    pub(crate) selection_generation: u64,
    pub(crate) state: State,
    pub(crate) state_loaded: bool,
    pub(crate) idle_close_pending: Option<editor_close::Target>,
    pub(crate) idle_close_snapshot: Vec<Tab>,
    pub(crate) idle_close_fallback: Option<editor_close::Target>,
    pub(crate) layouts: HashMap<String, Workspace>,
    pub(crate) layout_readonly: HashSet<String>,
    pub(crate) close_workspace: Option<(String, String)>,
    pub(crate) close_workspace_queue: Vec<String>,
    pub(crate) workspace_insert: HashMap<String, usize>,
    pub(crate) workspace_visible: Option<(String, String)>,
    pub(crate) layout_saved: HashMap<String, String>,
    pub(crate) layout_pending: HashMap<String, String>,
    /// Serialized signature of the last persistable layouts sent to the daemon.
    /// `save_layouts` compares against it so an unchanged workspace set does not
    /// clone workspaces, cross a thread, or wake the GUI every second.
    pub(crate) layout_signature: Option<Vec<(String, String)>>,
    pub(crate) layout_generation: u64,
    pub(crate) selected: Option<String>,
    pub(crate) active_session: Option<String>,
    /// Focused tabs `sync_active_session` last saw. Sync only follows focus
    /// *moves* in either dock so clicking the other dock is never clobbered.
    /// Strip moves are recorded while the strip is hidden and applied only
    /// while it is on screen, so revealing it does not replay a stale move.
    /// A move in both docks on the same call is a project switch: the main
    /// pane wins, and the strip focus is only recorded.
    pub(crate) last_main_focus: Option<Tab>,
    pub(crate) last_strip_focus: Option<Tab>,
    /// Set when focus lands on an image, browser, diff, player, or editor.
    /// Closing the active terminal clears it so recovery can run.
    pub(crate) non_terminal_selected: bool,
    pub(crate) images: HashMap<PathBuf, image_preview::Preview>,
    pub(crate) browser_host: browser_host::BrowserHost,
    pub(crate) visible_browsers: Vec<browser_host::VisibleBrowser>,
    pub(crate) browser_urls: HashMap<String, String>,
    pub(crate) browser_submit: Option<(String, BrowserTarget)>,
    pub(crate) player: player::Controller,
    pub(crate) markdown: markdown::Previews,
    pub(crate) visible_images: HashSet<PathBuf>,
    pub(crate) image_generation: u64,
    pub(crate) image_jobs: gui_services::ImageJobs,
    pub(crate) backends: HashMap<String, TerminalBackend>,
    pub(crate) visible_sessions: HashSet<String>,
    pub(crate) backend_ids: HashMap<u64, String>,
    pub(crate) next_backend: u64,
    /// Failed attaches for a session, keyed by its working directory.
    pub(crate) attach_budget: HashMap<String, retry_budget::RetryBudget>,
    pub(crate) attach_started: HashMap<String, Instant>,
    pub(crate) attach_failed: HashSet<u64>,
    pub(crate) attach_error: HashMap<String, String>,
    pub(crate) pty_tx: Sender<(u64, PtyEvent)>,
    pub(crate) pty_rx: Receiver<(u64, PtyEvent)>,
    pub(crate) jobs: exit::JobQueue,
    pub(crate) updates: Receiver<Update>,
    pub(crate) update_tx: Sender<Update>,
    pub(crate) picker_active: bool,
    pub(crate) add_tab: Option<(egui_dock::NodePath, Option<String>)>,
    /// Queued strip-dock tab creation (the strip's `on_add`/menu counterpart
    /// to [`Self::add_tab`]); consumed right after the strip renders.
    pub(crate) add_strip_tab: Option<(egui_dock::NodePath, Option<String>)>,
    /// Strip-dock tab to focus after the strip renders.
    pub(crate) focus_strip_tab: Option<Tab>,
    /// Strip shell to move into the main pane once the strip dock is back in
    /// `ide_strip_docks`. The menu runs while that dock is checked out for
    /// paint, so the removal has to wait until it is inserted again.
    pub(crate) move_strip_to_main: Option<String>,
    /// Main-pane shell to move into the strip once the workspace dock is
    /// checked back in. The caption menu runs while that dock is checked out.
    pub(crate) move_main_to_strip: Option<String>,
    /// Main-dock pane queued for detach into a fresh top-level tab. The
    /// pane menu runs while the workspace dock is checked out for paint,
    /// so the move waits until it is checked back in.
    pub(crate) detach_pane: Option<Tab>,
    /// Main-dock pane plus destination top-level tab id, queued by the
    /// pane menu to dock a detached pane back. Same checkout reason as
    /// [`Self::detach_pane`].
    pub(crate) dock_back_pane: Option<(Tab, String)>,
    /// Sibling top-level tabs as (id, label, icon), snapshotted by the
    /// workspace strip before the main dock paints so the pane menu can
    /// offer dock-back targets while the workspace is checked out.
    /// Excludes the active tab.
    pub(crate) dock_back_targets: Vec<(String, String, &'static str)>,
    /// Main-dock pane queued for floating outside the app window. Same
    /// checkout reason as [`Self::detach_pane`].
    pub(crate) float_pane: Option<Tab>,
    /// Floating tab creation queued by a floating window's own `+`/split
    /// controls: the issuing window plus the leaf path inside its dock.
    /// Consumed after that window's dock is checked back in.
    pub(crate) add_float_tab: Option<(egui::ViewportId, egui_dock::NodePath, Option<String>)>,
    /// Floating-dock tab to focus after its window renders.
    pub(crate) focus_float_tab: Option<(egui::ViewportId, Tab)>,
    /// Docks detached into their own OS windows, rendered every frame by
    /// [`Self::paint_floating`]. Closing a window docks its layout back.
    pub(crate) floating: Vec<FloatingWindow>,
    /// Floating-dock pane lookup, rebuilt every floating render (small
    /// docks; no cache). Reads the window under render only.
    pub(crate) float_pane_by_tab: HashMap<String, egui_dock::NodePath>,
    pub(crate) float_pane_tabs: HashMap<egui_dock::NodePath, Vec<Tab>>,
    /// Strip tab whose drag has started. Promoted to [`Self::pane_drag`] once
    /// the pointer leaves the strip, so reordering inside the strip stays
    /// with the dock.
    pub(crate) strip_tab_drag: Option<String>,
    /// Strip-dock pane lookup, rebuilt every strip render (the main-dock
    /// maps only ever cover the main dock; paths are meaningless across
    /// docks).
    pub(crate) strip_pane_by_tab: HashMap<String, egui_dock::NodePath>,
    pub(crate) strip_pane_tabs: HashMap<egui_dock::NodePath, Vec<Tab>>,
    pub(crate) pane_by_tab: HashMap<String, egui_dock::NodePath>,

    pub(crate) pane_tabs: HashMap<egui_dock::NodePath, Vec<Tab>>,
    pub(crate) pane_index: Option<PaneIndex>,
    /// Terminal pane currently dragged by its caption header or, once the
    /// pointer leaves the IDE strip, by a strip tab. Dropped onto another
    /// split leaf, a workspace tab, or the opposite dock.
    pub(crate) pane_drag: Option<Tab>,
    /// The dragged pane started in the IDE strip. A drop on the main pane
    /// moves it there; a drop back on the strip cancels that move.
    pub(crate) pane_drag_from_strip: bool,
    /// A cross-dock move changed a dock that was checked out for paint.
    /// Baselines and layout save run after that dock is inserted again.
    pub(crate) pending_layout_save: bool,
    /// Top-level tab currently dragged by its strip tab. Dropping it over
    /// the strip reorders it to the insertion slot; releasing elsewhere
    /// cancels. Only one of `pane_drag` and `tab_drag` is active at a time.
    pub(crate) tab_drag: Option<String>,
    /// Last text snapshot of the dragged terminal, taken when its drag
    /// starts and shown in the floating ghost.
    pub(crate) pane_drag_snapshot: Vec<String>,
    /// Group previewed while a pane drag hovers its strip tab, as
    /// (project, origin group) so a cancelled drag can switch back.
    pub(crate) drop_preview_origin: Option<(String, String)>,
    /// A pane drag hovers the interior of a strip tab this frame. The dock
    /// paints the previewed tab's focused leaf at real size so the
    /// move-into outcome is visible, not just the strip outline.
    pub(crate) strip_tab_hover: bool,
    /// A pane drag hovers a strip gap, "+", or empty strip background this
    /// frame, where a release opens a fresh top-level tab. The tab ghost
    /// (not the pane snapshot ghost) follows the pointer there.
    pub(crate) strip_new_tab_hover: bool,
    pub(crate) focus_tab: Option<Tab>,
    pub(crate) terminal_context: HashMap<String, String>,
    pub(crate) texts: HashMap<String, String>,
    pub(crate) diffs: HashMap<String, Result<std::sync::Arc<diff::DiffDocument>, String>>,
    pub(crate) diff_split: HashSet<String>,
    pub(crate) diff_split_ratio: f32,
    pub(crate) diff_split_scroll: HashMap<String, f32>,
    pub(crate) diff_preview: HashSet<String>,
    pub(crate) diff_ignore_ws: HashSet<String>,
    pub(crate) loading: HashSet<String>,
    pub(crate) git_logs: HashMap<String, git_log::CommitLog>,
    pub(crate) selected_commit: Option<String>,
    pub(crate) dirs: HashMap<PathBuf, Vec<terminator_git::Entry>>,
    pub(crate) directory_errors: HashMap<PathBuf, services::DirectoryError>,
    pub(crate) context: Option<services::ContextData>,
    pub(crate) metadata: Option<metadata::Metadata>,
    pub(crate) metadata_jobs: tokio::sync::watch::Sender<Option<metadata_refresh::Request>>,
    pub(crate) metadata_request: Option<metadata_refresh::Request>,
    pub(crate) metadata_generation: u64,
    pub(crate) resources: Option<resource_sample::Sample>,
    pub(crate) resource_tx: std::sync::mpsc::Sender<Option<resource_sample::Request>>,
    pub(crate) resource_request: Option<resource_sample::Request>,
    pub(crate) context_path: Option<PathBuf>,
    pub(crate) error: Option<String>,
    /// Same status text the user dismissed. Identical reports stay hidden.
    pub(crate) dismissed_error: Option<String>,
    /// Working directory the dismiss latch and missing-path budget belong to.
    pub(crate) error_cwd: Option<PathBuf>,
    pub(crate) missing_path_reports: u8,
    pub(crate) info: Option<String>,
    pub(crate) add_project: bool,
    pub(crate) settings_open: bool,
    pub(crate) settings_session: bool,
    pub(crate) settings_pending: Option<settings_ui::SettingsPending>,
    pub(crate) player_open: bool,
    pub(crate) editor_preset: usize,
    pub(crate) test_editor: bool,
    pub(crate) settings_draft: Settings,
    pub(crate) settings_section: SettingsSection,
    pub(crate) settings_search: String,
    pub(crate) custom_shell: bool,
    pub(crate) custom_editor: bool,
    pub(crate) shortcut_capture: Option<String>,
    pub(crate) browse_target: Option<BrowseTarget>,
    pub(crate) palette_open: bool,
    pub(crate) palette_query: String,
    pub(crate) palette_index: usize,
    pub(crate) worktree_draft: Option<worktree_ui::WorktreeDraft>,
    pub(crate) worktree_remove: Option<String>,
    pub(crate) theme: AppearanceConfig,
    pub(crate) terminal_theme: Option<(String, String, egui_term::TerminalTheme)>,
    pub(crate) theme_committed: AppearanceConfig,
    pub(crate) theme_draft: AppearanceConfig,
    pub(crate) theme_source: String,
    pub(crate) theme_conflict: bool,
    pub(crate) hook_status: HashMap<String, bool>,
    pub(crate) detail: Option<String>,
    pub(crate) close_session: Option<String>,
    pub(crate) popups: popup::Popups,
    /// Dialog shown when user picks "Save layout" from the palette.
    /// `Some(name)` = dialog open with pre-filled name.
    pub(crate) layout_save_name: Option<String>,
    pub(crate) editor_close_sessions: HashSet<String>,
    pub(crate) editor_close_prompts: Vec<(editor_close::Target, Vec<String>, String)>,
    pub(crate) rename_session: Option<(String, String)>,
    pub(crate) rename_focus: bool,
    pub(crate) rename_surface: RenameSurface,
    /// The menu starts a rename after that row's field slot, so the first
    /// frame has no field. This skips the "field was not drawn" check once.
    pub(crate) rename_seen: bool,
    /// `inline_rename` painted during this frame.
    pub(crate) rename_painted: bool,
    /// The field was expected and was not drawn. Terminals stay live until it
    /// paints again.
    pub(crate) rename_missed: bool,
    /// Last painted rename field, so a hidden field can drop keyboard focus.
    pub(crate) rename_field_id: Option<egui::Id>,
    pub(crate) editor_origins: HashMap<String, Vec<Tab>>,
    pub(crate) search: String,
    pub(crate) search_session: Option<String>,
    pub(crate) search_open: bool,
    pub(crate) worktree_open: bool,
    pub(crate) terminal_find: HashMap<String, TerminalFind>,
    pub(crate) history_filter: HashMap<String, String>,
    pub(crate) native_docs: HashMap<PathBuf, native_editor::NativeDoc>,
    pub(crate) native_pending_line: HashMap<PathBuf, usize>,
    pub(crate) native_pending_col: HashMap<PathBuf, usize>,
    pub(crate) lsp: lsp_manager::LspManager,
    pub(crate) native_close_prompt: Option<PathBuf>,
    pub(crate) native_close_after_save: Option<PathBuf>,
    /// Quit is waiting on the unsaved-native prompt. Save or discard continues it.
    pub(crate) pending_app_quit: bool,
    /// Deferred native closes: path, force flag, and the issuing view.
    /// `:q!` takes every view; a plain close removes only its issuing
    /// view, preserving other copies of the same file.
    pub(crate) pending_native_close: Vec<(PathBuf, bool, Option<native_editor::CloseIssuer>)>,
    /// View that issued the pending `:wq`: captured when the save
    /// starts, since the save settles frames later under whichever view
    /// renders first.
    pub(crate) native_close_after_save_issuer: Option<native_editor::CloseIssuer>,
    pub(crate) pending_native_splits: Vec<(egui::ViewportId, PathBuf, native_editor::NativeSplit)>,
    /// Finished cross-file goto jumps waiting for the workspace to be
    /// checked back in: the file view renders while its workspace is
    /// checked out, so looking up the originating project there always
    /// misses and drops the jump.
    pub(crate) pending_goto: Vec<lsp_manager::GotoJump>,
    pub(crate) file_index: terminator_native_edit::finder::FileIndex,
    pub(crate) file_index_rx:
        Option<std::sync::mpsc::Receiver<terminator_native_edit::finder::FileIndex>>,
    pub(crate) file_index_roots: Vec<PathBuf>,
    pub(crate) file_index_at: Option<std::time::Instant>,
    pub(crate) pending_unavailable_close: Vec<String>,
    pub(crate) pending_quit_all: Option<bool>,
    pub(crate) open_path: bool,
    pub(crate) pick_audio: bool,
    pub(crate) pick_audio_dir: bool,
    pub(crate) path_text: String,
    pub(crate) last_save: Instant,
    pub(crate) refresh: tokio::sync::watch::Sender<Option<refresh::Request>>,
    pub(crate) refresh_request: Option<refresh::Request>,
    pub(crate) refresh_generation: u64,
    pub(crate) visible_dirs: Vec<PathBuf>,
    pub(crate) expanded_dirs: HashSet<PathBuf>,
    pub(crate) watch_fallback: bool,
    pub(crate) targets: HashMap<String, Option<services::Target>>,
    pub(crate) hover: Option<(String, Instant)>,
    pub(crate) hover_popup: Option<HoverPopup>,
    pub(crate) dismissed_hover: Option<(String, String)>,
    pub(crate) hover_popup_blocks_input: bool,
    pub(crate) pending_target_action: Option<(String, Session)>,
    pub(crate) last_heartbeat: Instant,
    /// Env-gated (`TERMINATOR_REPAINT_LOG`) idle-CPU probe. `None` unless the
    /// variable is set, so release builds pay nothing for it.
    pub(crate) repaint_probe: Option<RepaintProbe>,
    pub(crate) last_focus: Option<String>,
    pub(crate) highlight_session: Option<String>,
    pub(crate) highlight_since: Instant,
    /// Shared render data, invalidated by snapshots and local notice changes.
    pub(crate) presentations: std::cell::RefCell<agent_presence::PresentationCache>,
    pub(crate) sidebar_projects: std::cell::RefCell<HashMap<String, std::sync::Arc<Project>>>,
    pub(crate) sidebar_cache: std::cell::RefCell<sidebar_cache::Cache>,
    /// Unread-view row retained after marking read, until selection/filter changes.
    pub(crate) unread_selected: Option<String>,
    pub(crate) connected: bool,
    pub(crate) control_server: Option<ui_control::Server>,
}

pub(crate) fn is_missing_path_error(error: &str) -> bool {
    if daemon_connection::is_connection_error(error) {
        return false;
    }
    error
        .lines()
        .any(|line| line.trim() == "No such file or directory (os error 2)")
}

pub(crate) fn observation_is_stale(current: &State, incoming: &State) -> bool {
    incoming.client_observation != 0
        && current.client_observation > incoming.client_observation
        && incoming.catalog_revision <= current.catalog_revision
        && (incoming.generation != current.generation || incoming.revision <= current.revision)
}

pub(crate) fn inventory_is_stale(current: &State, incoming: &State) -> bool {
    incoming.catalog_revision < current.catalog_revision
        || (incoming.generation == current.generation && incoming.revision < current.revision)
}

pub(crate) fn service_failure_update(
    context: &async_service::OperationContext,
    error: &async_service::Failure,
) -> Option<Update> {
    match context.subsystem {
        "diff" => Some(Update::Diff(
            context.resource.clone(),
            Err(error.to_string()),
        )),
        "files" => Some(Update::ResolvedTarget(context.resource.clone(), None)),
        "images" => Some(Update::Image(
            PathBuf::from(&context.resource),
            context.generation,
            Err(error.to_string()),
        )),
        "audio-spectrum" => None,
        _ => Some(Update::Error(error.to_string())),
    }
}
