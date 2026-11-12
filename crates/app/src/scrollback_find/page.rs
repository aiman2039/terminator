//! The Search all scrollback page, in a center card.
use super::{FindPage, Hit};
use crate::{App, appearance};
use eframe::egui;

/// What a click on the page asked for.
enum PageAction {
    None,
    Search,
    Stop,
    Open(Hit),
}

fn field_id() -> egui::Id {
    egui::Id::unique("scrollback-find-field")
}

impl App {
    pub(crate) fn scrollback_find_center(&mut self, ui: &mut egui::Ui) {
        self.scrollback_find.poll();
        let mut page = std::mem::take(&mut self.scrollback_find);
        let mut action = PageAction::None;
        let style = appearance::CenterCardStyle::new(&self.theme);
        let ((), dismissed) = appearance::center_card(ui, style, |ui| {
            action = self.scrollback_find_card(ui, &mut page);
        });
        self.scrollback_find = page;
        match action {
            PageAction::None if dismissed => self.hide_center_overlay(),
            PageAction::None => {}
            PageAction::Search => self.start_scrollback_find(ui.ctx()),
            PageAction::Stop => self.scrollback_find.stop(),
            PageAction::Open(hit) => self.open_scrollback_hit(ui.ctx(), &hit),
        }
        if self.scrollback_find.running {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    fn scrollback_find_card(&self, ui: &mut egui::Ui, page: &mut FindPage) -> PageAction {
        let mut action = PageAction::None;
        ui.strong("Search all scrollback");
        ui.weak(
            "Saved terminal output of every session. Agent conversation files are not searched.",
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let field = ui.add(
                appearance::singleline(&mut page.query.text)
                    .id(field_id())
                    .hint_text("Search text…")
                    .desired_width(320.0),
            );
            if std::mem::take(&mut page.focus) {
                field.request_focus();
            }
            if field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                action = PageAction::Search;
            }
            toggle(
                ui,
                "CaseSensitive",
                "Match case",
                &mut page.query.match_case,
            );
            toggle(ui, "Regex", "Regular expression", &mut page.query.regex);
            if page.running {
                ui.spinner();
                if ui.button("Stop").clicked() {
                    action = PageAction::Stop;
                }
            } else if ui.button("Search").clicked() {
                action = PageAction::Search;
            }
        });
        ui.horizontal(|ui| {
            ui.radio_value(&mut page.all_projects, false, "This project");
            ui.radio_value(&mut page.all_projects, true, "All projects");
        });
        ui.add_space(6.0);
        if let Some(hit) = self.scrollback_find_results(ui, page) {
            action = PageAction::Open(hit);
        }
        action
    }

    /// The summary line and the result rows. Returns the clicked row.
    fn scrollback_find_results(&self, ui: &mut egui::Ui, page: &FindPage) -> Option<Hit> {
        let found = match &page.result {
            None if page.running => {
                ui.weak("Searching…");
                return None;
            }
            None => return None,
            Some(Err(error)) => {
                ui.colored_label(appearance::color(&self.theme.status_failed), error);
                return None;
            }
            Some(Ok(found)) => found,
        };
        ui.weak(summary(found));
        let mut clicked = None;
        let row_height = ui.text_style_height(&egui::TextStyle::Body);
        egui::ScrollArea::vertical()
            .id_salt("scrollback-find-rows")
            .auto_shrink([false, false])
            .show_rows(ui, row_height, found.hits.len(), |ui, range| {
                for hit in found.hits.get(range).unwrap_or_default() {
                    let line = hit.line.saturating_add(1);
                    let text = format!("{}  {line}: {}", self.hit_source(hit), hit.text);
                    if ui
                        .add(egui::Button::selectable(false, text).truncate())
                        .on_hover_text("Open this session at the match")
                        .clicked()
                    {
                        clicked = Some(hit.clone());
                    }
                }
            });
        clicked
    }

    /// "label · project · age" of the hit's session.
    fn hit_source(&self, hit: &Hit) -> String {
        let Some(session) = self.state.sessions.iter().find(|s| s.id == hit.session) else {
            return hit.session.clone();
        };
        let project = self
            .state
            .projects
            .iter()
            .find(|p| p.id == session.project_id)
            .map_or("", |p| p.name.as_str());
        let age = terminator_core::now().saturating_sub(session.created);
        format!("{} · {project} · {}", session.label, short_age(age))
    }

    /// A live session opens with the terminal find bar on the query; an
    /// ended one opens its scrollback search page on it.
    pub(crate) fn open_scrollback_hit(&mut self, ctx: &egui::Context, hit: &Hit) {
        let query = self.scrollback_find.searched.clone();
        let live = self
            .state
            .sessions
            .iter()
            .any(|s| s.id == hit.session && s.lifecycle.live())
            && self.backends.contains_key(&hit.session);
        if !live {
            self.open_scrollback_search(ctx, &hit.session);
            self.search = if query.regex {
                hit.text.trim().to_owned()
            } else {
                query.text
            };
            return;
        }
        self.go_session(&hit.session);
        let find = self.terminal_find.entry(hit.session.clone()).or_default();
        find.query = if query.regex {
            hit.text.trim().chars().take(80).collect()
        } else {
            query.text
        };
        find.case_insensitive = !query.match_case;
        let id = egui::Id::unique(("terminal-find", hit.session.clone()));
        ctx.memory_mut(|memory| memory.request_focus(id));
    }
}

fn toggle(ui: &mut egui::Ui, icon: &str, tip: &str, on: &mut bool) {
    if appearance::selectable_icon(appearance::SelectableIconInput {
        ui,
        icon,
        tip,
        selected: *on,
    })
    .clicked()
    {
        *on = !*on;
    }
}

pub(crate) fn summary(found: &super::Found) -> String {
    let mut text = format!(
        "{} match(es) in {} session(s).",
        found.hits.len(),
        found.searched
    );
    if found.truncated {
        text.push_str(&format!(
            " Only the first {} are shown.",
            super::MAX_RESULTS
        ));
    }
    if found.stopped {
        text.push_str(" The search stopped at its time or size limit.");
    }
    if found.failed > 0 {
        text.push_str(&format!(" {} session(s) could not be read.", found.failed));
    }
    text
}

fn short_age(secs: u64) -> String {
    match secs {
        0..3600 => format!("{}m ago", secs / 60),
        3600..172_800 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}
