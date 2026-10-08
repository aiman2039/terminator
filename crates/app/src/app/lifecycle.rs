use super::super::{appearance, ui_control};
use eframe::egui::{self};
use std::{
    collections::{HashMap, HashSet},
    fs,
    sync::mpsc::{self},
    time::Instant,
};
use terminator_core::*;

use super::super::*;
impl App {
    pub(crate) fn command_dialog_open(&self) -> bool {
        self.palette_open
            || self.search_open
            || self.worktree_open
            || self.worktree_draft.is_some()
            || self.worktree_remove.is_some()
    }
    pub fn new(cc: &eframe::CreationContext<'_>, paths: Paths) -> Self {
        let mut app = Self::with_context(&cc.egui_ctx, paths.clone());
        match ui_control::spawn(paths, app.services.clone()) {
            Ok(server) => app.control_server = Some(server),
            Err(error) => app.error = Some(format!("GUI control server: {error:#}")),
        }
        #[cfg(feature = "test-support")]
        if std::env::var_os("TERMINATOR_TEST_RESPONSIVENESS").is_some()
            && let Some(socket) = std::env::var_os("TERMINATOR_TEST_STALL_NVIM")
            && let Err(error) = app.services.fixture_stalls(socket.into())
        {
            app.error = Some(format!("Fixture stalls: {error:#}"));
        }
        app
    }
    pub(crate) fn with_context(ctx: &egui::Context, paths: Paths) -> Self {
        appearance::install(ctx);
        let loaded = UiPreferences::load(&paths.data);
        let preferences_writable = loaded.is_ok();
        let preference_error = loaded
            .as_ref()
            .err()
            .map(|e| format!("UI preferences: {e:#}"));
        let upgrade_error = fs::read_to_string(paths.data.join("service-upgrade-error.txt")).ok();
        let preferences = loaded.unwrap_or_default();
        let (tx, updates) = mpsc::channel();
        let (services, service_owner) =
            gui_services::Services::new(paths.clone(), ctx.clone(), tx.clone()).unwrap_or_else(
                |error| {
                    eprintln!("start GUI services: {error:#}");
                    std::process::exit(1);
                },
            );
        let markdown = markdown::Previews::with_services(ctx, services.clone());
        let jobs = exit::JobQueue::supervised(services.clone());
        let refresh = refresh::spawn_async(services.clone());
        let metadata_jobs = metadata_refresh::spawn(services.clone());
        let resource_tx = resource_sample::spawn(tx.clone(), {
            let ctx = ctx.clone();
            move || ctx.request_repaint()
        });
        let update_tx = tx.clone();
        let image_jobs = gui_services::ImageJobs(services.clone());
        let _ = jobs.send(Job::HookStatus);
        let (pty_tx, pty_rx) = mpsc::channel();
        // Language servers run as actors on the GUI service supervisor:
        // without this, per-frame sync would spawn Tokio processes from
        // the GUI thread, which owns no runtime and panics.
        let mut lsp = lsp_manager::LspManager::new();
        lsp.set_services(services.clone());
        Self {
            file_activation: None,
            git_commit: String::new(),
            git_history: false,
            git_branches: Vec::new(),
            git_log: Vec::new(),
            git_list_root: None,
            git_collapse: 0,
            git_compare: None,
            git_compare_root: None,
            git_base_ref: None,
            explorer_search: Vec::new(),
            explorer_search_error: None,
            explorer_search_generation: 0,
            explorer_search_pending: false,
            explorer_search_last: None,
            explorer_query: String::new(),
            name_prompt: None,
            name_prompt_focus: false,
            pending_delete: None,
            services: services.clone(),
            service_owner,
            service_completion: None,
            service_ready: std::collections::VecDeque::default(),
            ui_service_peak_ms: 0.0,
            exit: exit::Exit::default(),
            exit_attempt: 0,
            updater: updater::Updater::new(ctx),
            #[cfg(any(windows, test))]
            windows_updates: windows_updates::WindowsUpdateCheck::new(),
            #[cfg(all(not(test), target_os = "macos"))]
            status_waiting_shown: None,
            installation_error: None,
            repair_pending: false,
            restart_pending: false,
            service_start_pending: false,
            restart_confirm: false,
            automatic_repair_attempt: None,
            #[cfg(feature = "test-support")]
            diagnostics: diagnostics::Diagnostics::default(),
            preferences_saved: preferences.clone(),
            preferences,
            preferences_writable,
            preferences_pending: false,
            project_width: 225.0,
            migration_requested: false,
            attention_requested: None,
            attention_pending: false,
            selection_generation: 0,
            paths,
            state: State::default(),
            state_loaded: false,
            idle_close_pending: None,
            idle_close_snapshot: Vec::new(),
            idle_close_fallback: None,
            layouts: HashMap::new(),
            layout_readonly: HashSet::new(),
            close_workspace: None,
            close_workspace_queue: Vec::new(),
            workspace_insert: HashMap::new(),
            workspace_visible: None,
            layout_saved: HashMap::new(),
            layout_pending: HashMap::new(),
            layout_signature: None,
            layout_generation: 0,
            selected: None,
            active_session: None,
            last_main_focus: None,
            last_strip_focus: None,
            non_terminal_selected: false,
            images: HashMap::new(),
            browser_host: browser_host::BrowserHost::new(),
            visible_browsers: Vec::new(),
            browser_urls: HashMap::new(),
            browser_submit: None,
            player: player::Controller::new(services),
            markdown,
            visible_images: HashSet::new(),
            image_generation: 0,
            image_jobs,
            backends: HashMap::new(),
            visible_sessions: HashSet::new(),
            backend_ids: HashMap::new(),
            next_backend: 0,
            attach_budget: HashMap::new(),
            attach_started: HashMap::new(),
            attach_failed: HashSet::new(),
            attach_error: HashMap::new(),
            pty_tx,
            pty_rx,
            jobs,
            updates,
            update_tx,
            picker_active: false,
            add_tab: None,
            add_strip_tab: None,
            focus_strip_tab: None,
            move_strip_to_main: None,
            move_main_to_strip: None,
            detach_pane: None,
            dock_back_pane: None,
            dock_back_targets: Vec::new(),
            float_pane: None,
            floating: Vec::new(),
            strip_tab_drag: None,
            strip_pane_by_tab: HashMap::new(),
            strip_pane_tabs: HashMap::new(),
            pane_by_tab: HashMap::new(),

            pane_tabs: HashMap::new(),
            pane_index: None,
            pane_drag: None,
            pane_drag_from_strip: false,
            pending_layout_save: false,
            tab_drag: None,
            strip_tab_hover: false,
            strip_new_tab_hover: false,
            pane_drag_snapshot: Vec::new(),
            drop_preview_origin: None,
            focus_tab: None,
            terminal_context: HashMap::new(),
            texts: HashMap::new(),
            diffs: HashMap::new(),
            diff_split: HashSet::new(),
            diff_split_ratio: 0.5,
            diff_split_scroll: HashMap::new(),
            diff_preview: HashSet::new(),
            diff_ignore_ws: HashSet::new(),
            loading: HashSet::new(),
            git_logs: HashMap::new(),
            selected_commit: None,
            dirs: HashMap::new(),
            directory_errors: HashMap::new(),
            context: None,
            metadata: None,
            metadata_jobs,
            metadata_request: None,
            metadata_generation: 0,
            resources: None,
            resource_tx,
            resource_request: None,
            context_path: None,
            error: preference_error.or(upgrade_error),
            dismissed_error: None,
            error_cwd: None,
            missing_path_reports: 0,
            info: None,
            add_project: false,
            settings_open: false,
            settings_session: false,
            settings_pending: None,
            player_open: false,
            editor_preset: external_editor::CUSTOM,
            test_editor: false,
            settings_draft: Settings::default(),
            settings_section: SettingsSection::Appearance,
            settings_search: String::new(),
            custom_shell: false,
            custom_editor: false,
            shortcut_capture: None,
            browse_target: None,
            palette_open: false,
            palette_query: String::new(),
            palette_index: 0,
            worktree_draft: None,
            worktree_remove: None,
            theme: AppearanceConfig::default(),
            terminal_theme: None,
            theme_committed: AppearanceConfig::default(),
            theme_draft: AppearanceConfig::default(),
            theme_source: String::new(),
            theme_conflict: false,
            hook_status: HashMap::new(),
            detail: None,
            close_session: None,
            popups: popup::Popups::default(),
            layout_save_name: None,
            editor_close_sessions: HashSet::new(),
            editor_close_prompts: Vec::new(),
            rename_session: None,
            rename_focus: false,
            rename_surface: RenameSurface::Sidebar,
            rename_seen: false,
            rename_painted: false,
            rename_missed: false,
            rename_field_id: None,
            editor_origins: HashMap::new(),
            search: String::new(),
            search_session: None,
            search_open: false,
            worktree_open: false,
            terminal_find: HashMap::new(),
            history_filter: HashMap::new(),
            native_docs: HashMap::new(),
            native_pending_line: HashMap::new(),
            native_pending_col: HashMap::new(),
            lsp,
            native_close_prompt: None,
            native_close_after_save: None,
            native_close_after_save_issuer: None,
            pending_app_quit: false,
            pending_native_close: Vec::new(),
            pending_native_splits: Vec::new(),
            pending_goto: Vec::new(),
            file_index: terminator_native_edit::finder::FileIndex::empty(),
            file_index_rx: None,
            file_index_roots: Vec::new(),
            file_index_at: None,
            pending_unavailable_close: Vec::new(),
            pending_quit_all: None,
            open_path: false,
            pick_audio: false,
            pick_audio_dir: false,
            path_text: String::new(),
            last_save: Instant::now(),
            refresh,
            refresh_request: None,
            refresh_generation: 0,
            visible_dirs: Vec::new(),
            expanded_dirs: HashSet::new(),
            watch_fallback: false,
            targets: HashMap::new(),
            hover: None,
            hover_popup: None,
            dismissed_hover: None,
            hover_popup_blocks_input: false,
            pending_target_action: None,
            last_heartbeat: Instant::now(),
            repaint_probe: RepaintProbe::enabled(),
            last_focus: None,
            highlight_session: None,
            highlight_since: Instant::now(),
            presentations: std::cell::RefCell::default(),
            sidebar_projects: std::cell::RefCell::default(),
            sidebar_cache: std::cell::RefCell::default(),
            unread_selected: None,
            connected: false,
            control_server: None,
        }
    }
    pub(crate) fn fixture_rect(&self, ctx: &egui::Context, name: &str) -> Option<[f32; 4]> {
        ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::new(("fixture-target", name))))
            .map(|r| [r.min.x, r.min.y, r.width(), r.height()])
    }
}
