use super::*;
use view::VncLiveInput;

impl App {
    pub(super) fn vnc_live_screen(&mut self, ui: &mut egui::Ui, input: VncLiveInput<'_>) {
        let VncLiveInput {
            session_id: sid,
            view_only,
            features,
        } = input;
        self.poll_vnc_frame(sid);
        ui.ctx().request_repaint_after(Duration::from_millis(33));
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
            transfer::panel(self, ui, sid, view_only);
        }
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

    /// Poll for new framebuffer data when visible.
    fn poll_vnc_frame(&mut self, sid: &str) {
        let should_poll = {
            let view = self.vnc_views.entry(sid.to_owned()).or_default();
            !view.fetching
                && view
                    .last_poll
                    .is_none_or(|t| t.elapsed() >= Duration::from_millis(33))
        };
        if !should_poll {
            return;
        }
        // A view whose texture cannot take the incremental update (no
        // texture yet, wrong size, dropped decode) asks for the whole
        // screen once; the daemon answers anything else with the
        // changed rectangle only.
        let (since, regions) = self
            .vnc_views
            .get(sid)
            .map_or((0, false), |view| (view.revision, !view.force_full));
        if let Some(view) = self.vnc_views.get_mut(sid) {
            view.fetching = true;
            view.last_poll = Some(Instant::now());
            view.force_full = false;
        }
        let _ = self.jobs.send(Job::VncFrame {
            session: sid.to_owned(),
            since,
            regions,
        });
    }
}
