use super::super::super::*;
use super::super::header::filter_history_lines;
use super::Viewer;

impl Viewer<'_> {
    /// A session that is no longer live: its resume commands, a fresh
    /// shell button, and its saved scrollback with a filter.
    pub(super) fn dead_session_view(&mut self, ui: &mut egui::Ui, session: &Session) {
        let sid = &session.id;
        self.app.backends.remove(sid);
        ui.colored_label(
            appearance::color(&self.app.theme.status_waiting),
            format!(
                "{:?} session — commands will not be run automatically",
                session.lifecycle
            ),
        );
        self.agent_resume_rows(ui, sid);
        if ui.button("Open a fresh shell here").clicked() {
            self.open_fresh_shell(session);
        }
        if session.truncated {
            ui.weak("Some saved output was pruned or unavailable.");
        }
        let key = format!("history:{sid}");
        if !self.app.texts.contains_key(&key) && self.app.loading.insert(key.clone()) {
            let _ = self.app.jobs.send(Job::rpc(
                Request::History {
                    session: sid.clone(),
                },
                After::Text(key.clone()),
            ));
        }
        self.saved_scrollback(ui, sid, key);
    }

    fn agent_resume_rows(&self, ui: &mut egui::Ui, sid: &str) {
        for a in self
            .app
            .state
            .agents
            .iter()
            .filter(|a| a.session_id == *sid)
        {
            let Some(resume) = &a.resume else {
                ui.weak(format!("{}: resume command unavailable", a.kind));
                continue;
            };
            let text = resume.display();
            ui.horizontal(|ui| {
                ui.monospace(&text);
                if ui.small_button("Copy resume").clicked() {
                    ui.ctx().copy_text(text);
                }
            });
        }
    }

    fn open_fresh_shell(&mut self, session: &Session) {
        let pane = self.pane_for(&Tab::Terminal(session.id.clone()).key());
        if let (Some(viewport), Some(pane)) = (self.window, pane) {
            // A fresh shell for a dead floated session stays in its
            // window instead of the main dock.
            self.app.add_float_tab = Some((viewport, pane, None));
            return;
        }
        let _ = self.app.jobs.send(Job::rpc(
            Request::Create {
                project: session.project_id.clone(),
                cwd: Some(session.cwd.clone()),
                file: None,
                line: None,
                column: None,
                editor: false,
            },
            if self.strip {
                After::Strip
            } else {
                After::Create(None)
            },
        ));
    }

    /// The filter row and the virtualized saved scrollback lines.
    fn saved_scrollback(&mut self, ui: &mut egui::Ui, sid: &str, key: String) {
        let sid_key = sid.to_owned();
        let query = self
            .app
            .history_filter
            .entry(sid_key.clone())
            .or_default()
            .clone();
        ui.horizontal(|ui| {
            ui.add(
                appearance::singleline(self.app.history_filter.entry(sid_key.clone()).or_default())
                    .id(egui::Id::unique(("history-filter", sid_key)))
                    .hint_text("Filter saved scrollback")
                    .desired_width(220.0),
            );
            if appearance::sidebar_action(ui, "X", "Back").clicked() {
                self.app.search_session = None;
                self.app.history_filter.remove(sid);
            }
        });
        let Some(text) = self.app.texts.get(&key) else {
            return;
        };
        let lines = filter_history_lines(text, &query);
        if !query.is_empty() {
            ui.monospace(format!("{} matching lines", lines.len()));
        }
        egui::ScrollArea::both()
            .id_salt(key)
            .show_rows(ui, 18.0, lines.len(), |ui, range| {
                for row in range {
                    if let Some(line) = lines.get(row) {
                        ui.monospace(*line);
                    }
                }
            });
    }
}
