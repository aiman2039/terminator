use super::*;
use view::VncLiveInput;

/// How long the daemon holds a shared-memory frame request open.
const SHM_WAIT_MS: u32 = 400;
/// Repaint fallback while the long poll is in use.
const SHM_REPAINT_FALLBACK: Duration = Duration::from_millis(250);

impl App {
    pub(super) fn vnc_live_screen(&mut self, ui: &mut egui::Ui, input: VncLiveInput<'_>) {
        let VncLiveInput {
            session_id: sid,
            view_only,
            features,
        } = input;
        // A shared-memory long poll wakes the GUI when its frame arrives; the
        // timer is only the fallback that keeps the poll going.
        let shm = self.poll_vnc_frame(sid);
        ui.ctx().request_repaint_after(if shm {
            SHM_REPAINT_FALLBACK
        } else {
            Duration::from_millis(33)
        });
        let Some(texture) = self
            .vnc_views
            .entry(sid.to_owned())
            .or_default()
            .texture
            .clone()
        else {
            ui.centered_and_justified(|ui| {
                ui.label("Connecting to VNC framebuffer…");
            });
            return;
        };
        if features {
            transfer::panel(transfer::PanelInput {
                app: self,
                ui,
                sid,
                view_only,
            });
        }
        let screen_area = ui.available_rect_before_wrap();
        let outcome = {
            let view = self.vnc_views.entry(sid.to_owned()).or_default();
            vnc_screen(VncScreenInput {
                ui,
                view,
                texture: &texture,
                view_only,
                paste_shortcut: self.preferences.vnc_paste_shortcut,
            })
        };
        self.paint_reconnecting(ui, sid, screen_area);
        if features && !view_only {
            transfer::drop_upload(transfer::DropInput {
                app: self,
                ui,
                sid,
                area: screen_area,
            });
        }
        if outcome.activated {
            self.active_session = None;
            self.non_terminal_selected = true;
        }
        if !outcome.events.is_empty() {
            self.send(Request::VncInput {
                session: sid.to_owned(),
                events: outcome.events,
            });
        }
    }

    /// While the daemon waits to reconnect, dim the frozen last frame and
    /// say when the next attempt starts.
    fn paint_reconnecting(&self, ui: &egui::Ui, sid: &str, area: egui::Rect) {
        let Some(session) = self.state.vnc_sessions.iter().find(|s| s.id == sid) else {
            return;
        };
        if session.status != VncSessionStatus::Connecting || session.retry_attempt == 0 {
            return;
        }
        let painter = ui.painter_at(area);
        painter.rect_filled(area, 0.0, egui::Color32::from_black_alpha(140));
        painter.text(
            area.center(),
            egui::Align2::CENTER_CENTER,
            format!(
                "Reconnecting… attempt {} in {}s",
                session.retry_attempt, session.retry_in_seconds
            ),
            egui::FontId::proportional(16.0),
            egui::Color32::WHITE,
        );
    }

    /// Poll for new framebuffer data when visible. Returns whether the
    /// shared-memory long poll is in use.
    fn poll_vnc_frame(&mut self, sid: &str) -> bool {
        let shm = !self.vnc_views.get(sid).is_some_and(|view| view.shm_failed)
            && self.session_capability(sid, terminator_core::vnc::SHM_CAPABILITY);
        // The long poll waits in the daemon, so it needs no client pacing.
        let interval = if shm {
            Duration::ZERO
        } else {
            Duration::from_millis(33)
        };
        let should_poll = {
            let view = self.vnc_views.entry(sid.to_owned()).or_default();
            !view.fetching && view.last_poll.is_none_or(|t| t.elapsed() >= interval)
        };
        if !should_poll {
            return shm;
        }
        // A view whose texture cannot take the incremental update (no
        // texture yet, wrong size, dropped decode) asks for the whole
        // screen once; the daemon answers anything else with the
        // changed rectangle only.
        let (mut since, regions, cursor_serial) =
            self.vnc_views.get(sid).map_or((0, false, 0), |view| {
                (view.revision, !view.force_full, view.cursor_serial)
            });
        if shm && !regions {
            // The daemon sends the whole frame to a viewer that is not at
            // its last published revision.
            since = 0;
        }
        if let Some(view) = self.vnc_views.get_mut(sid) {
            view.fetching = true;
            view.last_poll = Some(Instant::now());
            view.force_full = false;
        }
        let _ = self.jobs.send(Job::VncFrame {
            session: sid.to_owned(),
            since,
            regions,
            shm: shm.then_some(terminator_core::vnc::VncShmRequest {
                wait_ms: SHM_WAIT_MS,
                cursor_serial,
            }),
        });
        shm
    }
}
