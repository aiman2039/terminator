//! Session peek: a read-only snapshot of one session's screen in a popup.
//! It reads with `Request::Screen` (`screen-v1`) or, on older daemons, the
//! saved scrollback. Nothing is sent to the PTY: while the popup shows,
//! every terminal input gate is closed, and Esc closes it.
use crate::{After, App, Job, appearance};
use eframe::egui;
use terminator_core::{Request, SCREEN_CAPABILITY};

fn peek_key(session: &str) -> String {
    format!("peek:{session}")
}

impl App {
    /// Opens (or refreshes) the peek of `session`.
    pub(crate) fn open_session_peek(&mut self, session: &str) {
        let key = peek_key(session);
        self.texts.remove(&key);
        self.loading.insert(key.clone());
        let screen = self
            .state
            .capabilities
            .iter()
            .any(|c| c == SCREEN_CAPABILITY);
        let request = if screen {
            Request::Screen {
                session: session.to_owned(),
            }
        } else {
            Request::History {
                session: session.to_owned(),
            }
        };
        let _ = self.jobs.send(Job::rpc(request, After::Text(key)));
        self.triage.peek = Some(session.to_owned());
    }

    pub(crate) fn close_session_peek(&mut self) {
        if let Some(session) = self.triage.peek.take() {
            let key = peek_key(&session);
            self.texts.remove(&key);
            self.loading.remove(&key);
        }
    }

    /// The peek popup. Esc and Close end it; Refresh reads the screen again.
    pub(crate) fn session_peek_modal(&mut self, ctx: &egui::Context) {
        let Some(session) = self.triage.peek.clone() else {
            return;
        };
        if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.close_session_peek();
            return;
        }
        let label = self
            .state
            .sessions
            .iter()
            .find(|s| s.id == session)
            .map_or_else(|| "Session".to_owned(), |s| s.label.clone());
        let key = peek_key(&session);
        let mut open = true;
        let mut refresh = false;
        let mut close = false;
        let text = self.texts.get(&key).cloned();
        self.popups
            .centered(ctx, "Peek")
            .id(egui::Id::unique("session-peek"))
            .title_bar(false)
            .collapsible(false)
            .resizable(true)
            .default_size(egui::vec2(640.0, 400.0))
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.strong(format!("{label} (read only)"));
                    appearance::footer_row(ui, |ui| {
                        close = appearance::sidebar_action(ui, "X", "Close (Esc)").clicked();
                        refresh =
                            appearance::sidebar_action(ui, "RefreshCw", "Read the screen again")
                                .clicked();
                    });
                });
                ui.separator();
                peek_body(ui, text.as_deref());
            });
        if refresh {
            self.open_session_peek(&session);
        } else if close || !open {
            self.close_session_peek();
        }
    }
}

fn peek_body(ui: &mut egui::Ui, text: Option<&str>) {
    let Some(text) = text else {
        ui.weak("Reading the screen…");
        return;
    };
    egui::ScrollArea::both()
        .id_salt("session-peek-text")
        .stick_to_bottom(true)
        .auto_shrink(false)
        .show(ui, |ui| {
            for line in text.lines() {
                ui.monospace(line);
            }
        });
}
