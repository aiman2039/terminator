use super::super::appearance;
use eframe::egui::{self};
use std::time::{Duration, Instant};
use terminator_core::*;

use super::super::*;
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
        // eframe calls logic even while hidden/minimized; ui is rendering-only.
        // IPC and exit checkpoints must not depend on a visible window.
        if let Some(probe) = self.repaint_probe.as_mut() {
            let causes: Vec<String> = ctx
                .repaint_causes()
                .iter()
                .map(ToString::to_string)
                .collect();
            if let Some(line) = probe.sample(Instant::now(), &causes) {
                eprintln!("{line}");
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
        self.hover_popup_blocks_input = self.hover_popup.is_some()
            || (ctx.current_pass_index() > 0 && self.hover_popup_blocks_input);
        self.visible_dirs.clear();
        self.visible_sessions.clear();
        self.visible_images.clear();
        self.visible_browsers.clear();
        self.markdown.begin_frame();
        #[cfg(feature = "test-support")]
        self.diagnostics.frame(&ctx);
        for action in shortcuts::ACTIONS.iter().map(|(action, _)| *action) {
            if !self.shortcut_allowed(action) || !self.shortcut_applies(action) {
                continue;
            }
            if action == "select_all" && self.text_input_focused(&ctx) {
                continue;
            }
            let key = shortcuts::binding(&self.state.settings.keybindings, action);
            if key.is_empty() || !shortcuts::consume(&ctx, &key) {
                continue;
            }
            self.run_shortcut(&ctx, action);
        }
        self.suspend_hidden_rename(&ctx);
        if self.state_loaded {
            self.migrate_attention();
        }
        // Single drag-band row. The tab strip is a second top panel
        // shown after the sidebars, so it spans only the center and the
        // sidebars run full height. Both rows sit below the native
        // titlebar band, so tab drags never race window moves.
        egui::Panel::top("window-header")
            .exact_size(40.0)
            .frame(egui::Frame::NONE.fill(appearance::color(&self.theme.surface)))
            .show(ui, |ui| self.window_header(ui));
        if !self.state.settings.notifications_side {
            egui::Panel::top("attention").show(ui, |ui| self.notifications(ui));
        }
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.with_layout(
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                ui.colored_label(
                    appearance::color(if self.connected {
                        &self.theme.status_running
                    } else {
                        &self.theme.status_waiting
                    }),
                    if self.connected {
                        "● Connected"
                    } else {
                        "○ Connecting"
                    },
                )
                .on_hover_text(format!(
                    "GUI {}\nDaemon {}\nDaemon executable: {}\nAttachment helper: {}",
                    env!("CARGO_PKG_VERSION"),
                    self.state
                        .daemon_version
                        .as_deref()
                        .unwrap_or("unknown (older daemon)"),
                    self.state
                        .daemon_executable
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "not reported by this daemon".into()),
                    match self.state.attachment_helper_available {
                        Some(true) => "available",
                        Some(false) => "unavailable",
                        None => "not reported by this daemon",
                    }
                ));
                ui.separator();
                // A count only: holding the tab list across the chain would
                // borrow self while the banner bodies need `&mut self`.
                let unavailable_tabs = self.unavailable_tabs().len();
                if self.installation_problem() {
                    let repair = ui.small_button("Fix installation…");
                    #[cfg(feature = "test-support")]
                    diagnostics::record(ui.ctx(), "fix-installation", repair.rect);
                    if repair.clicked() {
                        self.open_installation_settings();
                    }
                    self.restart_session_button(ui, true);
                    ui.colored_label(
                        appearance::color(&self.theme.status_failed),
                        "Terminal helper needs repair. Existing sessions are preserved.",
                    );
                } else if self.service_disconnected() {
                    ui.horizontal_wrapped(|ui| {
                        self.start_service_button(ui, true);
                        ui.colored_label(
                            appearance::color(&self.theme.status_failed),
                            "Session service unreachable.",
                        );
                        if let Some(detail) = self
                            .error
                            .clone()
                            .filter(|error| !daemon_connection::is_connection_error(error))
                        {
                            ui.colored_label(
                                appearance::color(&self.theme.status_failed),
                                detail,
                            );
                        } else {
                            ui.weak("Start it to show and close sessions. Ended sessions remain in History.");
                        }
                    });
                } else if let Some(error) = self.error.clone() {
                    ui.horizontal_wrapped(|ui| {
                        if ui.small_button("Dismiss").clicked() {
                            self.dismiss_status_error();
                        }
                        ui.colored_label(appearance::color(&self.theme.status_failed), error);
                    });
                } else if unavailable_tabs > 0 {
                    ui.horizontal_wrapped(|ui| {
                        let close = ui.small_button("Close unavailable tabs");
                        #[cfg(feature = "test-support")]
                        diagnostics::record(ui.ctx(), "close-unavailable-tabs", close.rect);
                        if close.clicked() {
                            self.close_unavailable_tabs();
                        }
                        ui.weak(if unavailable_tabs == 1 {
                            "1 tab has no session record. It was left behind by sessions that already ended.".into()
                        } else {
                            format!("{unavailable_tabs} tabs have no session record. They were left behind by sessions that already ended.")
                        });
                    });
                } else if let Some(message) = &self.state.degraded {
                    ui.colored_label(appearance::color(&self.theme.status_waiting), message);
                } else if self.state_loaded
                    && (self.state.daemon_version.as_deref() != Some(env!("CARGO_PKG_VERSION"))
                        || !self
                            .state
                            .capabilities
                            .iter()
                            .any(|c| c == STABLE_HELPER_CAPABILITY))
                {
                    if ui.small_button("Review installation…").clicked() {
                        self.open_installation_settings();
                    }
                    self.restart_session_button(ui, true);
                    ui.weak("App and session service use different installations.");
                } else if let Some(info) = self.info.clone() {
                    ui.horizontal(|ui| {
                        if ui.small_button("×").clicked() {
                            self.info = None;
                        }
                        ui.label(info);
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.weak(format!(
                            "{} sessions running",
                            self.state
                                .sessions
                                .iter()
                                .filter(|s| s.lifecycle.live())
                                .count()
                        ));
                    });
                }
                if let Some(metadata) = self.metadata.clone() {
                    if let Some(branch) = metadata.branch {
                        ui.weak(if metadata.worktree {
                            format!("Worktree · {branch}")
                        } else {
                            branch
                        });
                    }
                    if let Some(pr) = metadata.pull_request
                        && ui
                            .link(format!("PR #{}", pr.number))
                            .on_hover_text(pr.title)
                            .clicked()
                    {
                        let _ = self.jobs.send(Job::Browser(pr.url));
                    }
                    for port in metadata.ports.iter().take(3) {
                        if ui
                            .link(format!(":{}", port.port))
                            .on_hover_text(&port.address)
                            .clicked()
                        {
                            let _ = self.jobs.send(Job::Browser(port.url()));
                        }
                    }
                }
                if self.preferences.ide_mode {
                    ui.separator();
                    self.player_status_row(ui);
                    self.notification_status_badge(ui);
                }
                // Single right-aligned block (see status_right_end): resource
                // readout, sidebar-height toggle, then the strip collapse.
                // Right-aligned app totals. Pure paint over the background
                // sample: no extra wakes, updates land with the heartbeat.
                self.status_right_end(ui);
            });
        });
        self.place_ide_columns(ui);
        // Center-only: shown after the sidebars so the tab strip sits beside
        // them instead of pushing them down.
        egui::Panel::top("workspace-tabs")
            .exact_size(36.0)
            .frame(egui::Frame::NONE.fill(appearance::color(&self.theme.surface)))
            .show(ui, |ui| self.window_header_tabs(ui));
        if self.preferences_writable
            && !self.preferences_pending
            && self.preferences != self.preferences_saved
        {
            let _ = self.jobs.send(Job::Preferences(self.preferences.clone()));
            self.preferences_pending = true;
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(appearance::color(&self.theme.window))
                    .inner_margin(2),
            )
            .show(ui, |ui| self.center_pane(ui));
        self.note_rename_frame(&ctx);
        self.preview_appearance(&ctx);
        // Floating panes render after these prunes (`paint_floating` runs
        // last), so a frame that only marked docked panes would drop every
        // floated backend/image/preview and re-attach it on the next paint:
        // an attach storm that stutters the whole window. Count floats as
        // visible up front; the float paint re-marks them anyway.
        self.seed_floating_visibility();
        self.images
            .retain(|path, _| self.visible_images.contains(path));
        self.markdown.end_frame(&ctx);
        self.backends
            .retain(|sid, _| self.visible_sessions.contains(sid));
        if let Some(session) =
            self.state.sessions.iter().find(|s| {
                Some(&s.id) == self.active_session.as_ref() && s.kind == SessionKind::Shell
            })
        {
            self.terminal_context
                .insert(session.project_id.clone(), session.id.clone());
        }
        if self.active_session != self.last_focus {
            if let Some(session) = &self.active_session {
                self.send(Request::Focus {
                    session: session.clone(),
                });
            }
            self.last_focus = self.active_session.clone();
        }
        let next_metadata = self.cwd().map(|cwd| metadata_refresh::Request {
            cwd,
            identity: self
                .context_session()
                .and_then(|s| s.pid.map(|pid| (pid, s.created))),
            include_pr: self.state.settings.pr_metadata,
            generation: self.metadata_generation,
        });
        if next_metadata != self.metadata_request {
            self.metadata_generation = self.metadata_generation.wrapping_add(1);
            self.metadata = None;
            self.metadata_request = next_metadata.map(|mut r| {
                r.generation = self.metadata_generation;
                r
            });
            let _ = self.metadata_jobs.send(self.metadata_request.clone());
        }
        self.sync_resource_sample();
        self.visible_dirs.sort();
        self.visible_dirs.dedup();
        let wants_files = self.preferences.visible
            && matches!(
                self.preferences.tool,
                SidebarTool::Explorer | SidebarTool::Git
            );
        let next = self
            .cwd()
            .filter(|_| wants_files)
            .map(|cwd| refresh::Request {
                cwd,
                generation: self.refresh_generation,
                directories: self.visible_dirs.clone(),
            });
        if next != self.refresh_request {
            self.refresh_generation = self.refresh_generation.saturating_add(1);
            let next = next.map(|mut r| {
                r.generation = self.refresh_generation;
                r
            });
            let cwd = next.as_ref().map(|r| r.cwd.clone());
            if self.context_path != cwd {
                self.context = None;
                self.dirs.clear();
                self.directory_errors.clear();
            }
            self.context_path = cwd;
            self.refresh_request.clone_from(&next);
            let _ = self.refresh.send(next);
        }
        if self.last_save.elapsed() > Duration::from_secs(1) {
            self.save_layouts();
            self.last_save = Instant::now();
        }
        if cfg!(target_os = "linux") {
            window_resize_edges(ui);
        }
        self.modals(&ctx, frame);
        self.apply_browser_submit();
        self.reconcile_gui_resources();
        self.sync_browsers(frame);
        // Floating panes render in their own OS windows after every dock
        // is checked back in, so a closed window can dock straight back.
        self.paint_floating(&ctx);
        self.popups.end_frame();
        if self.hover_popup.as_ref().is_some_and(|popup| {
            !self.visible_sessions.contains(&popup.session)
                || !self.backends.contains_key(&popup.session)
        }) {
            self.dismiss_hover_popup(&ctx);
        }
        self.sync_menu_bar(&ctx);
        appearance::click_cursor(&ctx);
        #[cfg(feature = "test-support")]
        self.diagnostics.capture(&ctx);
        ctx.request_repaint_after(Duration::from_secs(1));
    }
}

impl App {
    /// Bring the window forward from the menu-bar status item.
    #[cfg(any(test, target_os = "macos"))]
    pub(crate) fn focus_window_from_menu(ctx: &egui::Context) {
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }
    #[cfg(any(test, target_os = "macos"))]
    pub(crate) fn open_agents_inbox(&mut self) {
        self.preferences.tool = SidebarTool::Agents;
        self.preferences.visible = true;
        self.preferences.agents_tab = AgentsTab::NeedsAttention;
    }
    /// A status-menu pick: the inbox Go button plus opening the inbox
    /// behind it. A stale id just opens the inbox.
    #[cfg(any(test, target_os = "macos"))]
    pub(crate) fn focus_status_notice(&mut self, ctx: &egui::Context, id: &str) {
        Self::focus_window_from_menu(ctx);
        self.open_agents_inbox();
        if let Some(session) = self
            .state
            .notifications
            .iter()
            .find(|notice| notice.id == id)
            .map(|notice| notice.session_id.clone())
        {
            self.go_session(&session);
        }
        self.detail = None;
    }
    #[cfg(all(not(test), target_os = "macos"))]
    pub(crate) fn sync_menu_bar(&mut self, ctx: &egui::Context) {
        if updater::take_status_click() {
            Self::focus_window_from_menu(ctx);
            if self.waiting_notice_count() > 0 {
                self.open_agents_inbox();
            }
        }
        updater::sync_status_menu(&self.status_menu_items());
        if let Some(id) = updater::take_status_selection() {
            self.focus_status_notice(ctx, &id);
        }
        let waiting = self.waiting_notice_count();
        if self.status_waiting_shown == Some(waiting) {
            return;
        }
        self.status_waiting_shown = Some(waiting);
        let icon = menu_bar::status_icon(waiting);
        updater::sync_status_item(&icon.png, icon.width_pt, icon.height_pt);
    }

    #[cfg(not(all(not(test), target_os = "macos")))]
    pub(crate) fn sync_menu_bar(&mut self, _ctx: &egui::Context) {}
}
