//! Per-frame driver: `logic` services queues and exit, `ui` paints the
//! window chrome and center, then syncs worker requests to what painted.

use std::time::{Duration, Instant};

use crate::*;

mod frame_sync;
mod input;
mod menu;
mod status_bar;

/// Env-gated idle-CPU probe. Set `TERMINATOR_REPAINT_LOG=1` and watch stderr:
/// one line per second with the frame count plus egui's own repaint causes
/// (`file:line reason`), which names the exact call site keeping the UI awake.
pub(crate) struct RepaintProbe {
    pub(crate) frames: u64,
    pub(crate) last: Instant,
}

impl RepaintProbe {
    pub(crate) fn enabled() -> Option<Self> {
        std::env::var_os("TERMINATOR_REPAINT_LOG").map(|_| Self {
            frames: 0,
            last: Instant::now(),
        })
    }

    /// Count one frame; about once per second render a one-line summary.
    pub(crate) fn sample(&mut self, now: Instant, causes: &[String]) -> Option<String> {
        self.frames = self.frames.saturating_add(1);
        if now.duration_since(self.last) < Duration::from_secs(1) {
            return None;
        }
        let line = format!(
            "terminator repaint-log: {} frames/s causes=[{}]",
            self.frames,
            causes.join(", ")
        );
        self.frames = 0;
        self.last = now;
        Some(line)
    }
}

impl eframe::App for App {
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        // No per-viewport `max_texture_side` override here: eframe builds
        // immediate-viewport (floating) input without calling this hook,
        // so a parent-only cap would alternate the shared font atlas
        // between two sizes every frame and damage main-window text.
        // Every viewport uses the renderer limit consistently.
        #[cfg(feature = "test-support")]
        self.diagnostics.input(ctx, input);
        #[cfg(not(feature = "test-support"))]
        let _ = (ctx, input);
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Diff builds run on workers; they highlight with this theme.
        crate::diff::set_dark(ctx.global_style().visuals.dark_mode);
        // eframe calls logic even while hidden/minimized; ui is rendering-only.
        // IPC and exit checkpoints must not depend on a visible window.
        if let Some(probe) = self.repaint_probe.as_mut() {
            let causes: Vec<String> = ctx
                .repaint_causes()
                .iter()
                .map(ToString::to_string)
                .collect();
            if let Some(line) = probe.sample(Instant::now(), &causes) {
                crate::console::Console::err(format_args!("{line}"));
            }
        }
        self.process_updates(ctx);
        // A terminal tab whose session record was pruned (an ended shell or
        // editor with no resume handle) should disappear on its own.
        self.prune_unavailable_tabs();
        if updater::termination_cancelled() {
            self.native_installation_cancelled();
        }
        if (updater::termination_requested() || ctx.input(|i| i.viewport().close_requested()))
            && !matches!(self.exit, exit::Exit::Ready)
        {
            // Unsaved native buffers die with the process. The prompt has to
            // finish the quit Sparkle is waiting on; dropping it leaves
            // Install and Relaunch up with a process that never exits.
            self.accept_app_quit(ctx);
        }
        self.continue_app_quit(ctx);
        self.advance_exit(ctx);
        if !self.exit.active() && self.last_heartbeat.elapsed() > Duration::from_secs(1) {
            self.send(Request::Heartbeat {
                focused: ctx.input(|i| {
                    i.viewport().focused.unwrap_or(false)
                        && !i.viewport().minimized.unwrap_or(false)
                }),
            });
            self.last_heartbeat = Instant::now();
            self.maybe_upgrade_idle_daemon();
        }
        if !self.exit.active() {
            self.poll_background_services(ctx);
        }
        ctx.request_repaint_after(Duration::from_secs(1));
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.popups.begin_frame(&ctx);
        // Clear before the exit early return too, so the last-painted terminals
        // stop scheduling repaints while the exit screen is shown.
        for backend in self.backends.values() {
            backend.set_painted(false);
        }
        if self.exit.active() {
            self.browser_host.hide_all();
            ui.centered_and_justified(|ui| {
                ui.label("Saving workspace before closing…");
            });
            return;
        }
        self.begin_frame_visibility(&ctx);
        self.handle_vnc_escape_keys(&ctx);
        self.run_shortcuts(&ctx);
        self.suspend_hidden_rename(&ctx);
        if self.state_loaded {
            self.migrate_attention();
        }
        if self.vnc_fullscreen.is_none() {
            self.window_chrome(ui);
        }
        self.save_changed_preferences();
        self.central_panel(ui);
        self.vnc_quit_dialog(&ctx);
        self.note_rename_frame(&ctx);
        self.preview_appearance(&ctx);
        self.prune_unpainted(&ctx);
        self.sync_terminal_focus();
        self.sync_metadata_request();
        self.sync_resource_sample();
        self.sync_refresh_request();
        if self.last_save.elapsed() > Duration::from_secs(1) {
            self.save_layouts();
            self.last_save = Instant::now();
        }
        if cfg!(target_os = "linux") {
            window_resize_edges(ui);
        }
        self.paint_windows(&ctx, frame);
        self.popups.end_frame();
        self.dismiss_stale_hover_popup(&ctx);
        self.sync_menu_bar(&ctx);
        appearance::click_cursor(&ctx);
        #[cfg(feature = "test-support")]
        self.diagnostics.capture(&ctx);
        ctx.request_repaint_after(Duration::from_secs(1));
    }
}

impl App {
    /// Browser navigation, GUI resources, the player, and update checks,
    /// polled while the app is not exiting.
    fn poll_background_services(&mut self, ctx: &egui::Context) {
        for (key, target) in self.browser_host.take_navigations() {
            self.apply_browser_navigation(&key, target);
        }
        self.reconcile_gui_resources();
        self.poll_player();
        if self.player.needs_poll() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        self.updater.poll();
        #[cfg(any(windows, test))]
        if self
            .windows_updates
            .poll(self.settings_draft.automatic_update_checks)
        {
            ctx.request_repaint();
        }
    }

    /// Single drag-band row. The tab strip is a second top panel
    /// shown after the sidebars, so it spans only the center and the
    /// sidebars run full height. Both rows sit below the native
    /// titlebar band, so tab drags never race window moves.
    fn window_chrome(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("window-header")
            .exact_size(40.0)
            .frame(egui::Frame::NONE.fill(appearance::color(&self.theme.surface)))
            .show(ui, |ui| self.window_header(ui));
        if !self.state.settings.notifications_side {
            egui::Panel::top("attention").show(ui, |ui| self.notifications(ui));
        }
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        self.place_ide_columns(ui);
        // Center-only: shown after the sidebars so the tab strip sits beside
        // them instead of pushing them down.
        egui::Panel::top("workspace-tabs")
            .exact_size(36.0)
            .frame(egui::Frame::NONE.fill(appearance::color(&self.theme.surface)))
            .show(ui, |ui| self.window_header_tabs(ui));
    }

    fn central_panel(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(appearance::color(&self.theme.window))
                    .inner_margin(2),
            )
            .show(ui, |ui| {
                if self.vnc_fullscreen.is_some() {
                    self.fullscreen_vnc(ui);
                } else {
                    self.center_pane(ui);
                }
            });
    }

    fn fullscreen_vnc(&mut self, ui: &mut egui::Ui) {
        let Some(mut tab) = self.vnc_fullscreen.clone() else {
            return;
        };
        if let Tab::Vnc {
            id,
            session,
            profile_id,
            host,
            port,
            label,
        } = &mut tab
        {
            self.vnc_view(
                ui,
                crate::vnc_ui::VncViewInput {
                    tab_id: id,
                    session_id: session,
                    profile_id,
                    host,
                    port,
                    label,
                },
            );
        }
        if self.vnc_fullscreen.is_some() {
            self.vnc_fullscreen = Some(tab);
        }
    }

    /// Modals and tool windows, browser overlays, then the floating panes.
    fn paint_windows(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.modals(ctx, frame);
        self.show_native_rename_dialog(ctx);
        self.show_usages_window(ctx);
        self.show_hierarchy_window(ctx);
        self.show_merge_window(ctx);
        self.apply_browser_submit();
        self.reconcile_gui_resources();
        self.sync_browsers(ctx, frame);
        // Floating panes render in their own OS windows after every dock
        // is checked back in, so a closed window can dock straight back.
        self.paint_floating(ctx);
    }
}
