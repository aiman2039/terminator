use super::super::*;
use super::viewer::Viewer;
impl Viewer<'_> {
    pub(super) fn strip_terminal_actions(&mut self, ui: &mut egui::Ui, path: egui_dock::NodePath) {
        let tabs = self
            .app
            .strip_pane_tabs
            .get(&path)
            .cloned()
            .unwrap_or_default();
        let sid = self
            .app
            .active_session
            .as_ref()
            .filter(|sid| {
                tabs.iter()
                    .any(|tab| matches!(tab, Tab::Terminal(id) if id == *sid))
            })
            .cloned()
            .or_else(|| {
                tabs.iter().find_map(|tab| match tab {
                    Tab::Terminal(id) => Some(id.clone()),
                    _ => None,
                })
            });
        let branch = sid.as_ref().and_then(|sid| {
            self.app
                .state
                .sessions
                .iter()
                .find(|session| &session.id == sid)
                .and_then(|session| self.app.branch_at(&session.cwd))
        });
        let git_tip = match &branch {
            Some(name) => format!("{name}\nOpen Git"),
            None => "Open Git".into(),
        };
        let vertical_tip = self.app.action_tip("Split vertically", "split_right");
        let horizontal_tip = self.app.action_tip("Split horizontally", "split_down");
        let (dot, _) = ui.allocate_exact_size(egui::vec2(14.0, 22.0), egui::Sense::hover());
        if let Some(color) = sid.as_ref().and_then(|sid| {
            self.app
                .state
                .sessions
                .iter()
                .find(|session| &session.id == sid)
                .and_then(|session| self.app.terminal_status_color(session))
        }) {
            ui.painter().circle_filled(dot.center(), 3.5, color);
        }
        let git_icon = appearance::sidebar_action(ui, "GitBranch", &git_tip);
        let branch_label = ui
            .add_sized(
                [appearance::TERMINAL_BRANCH_MAX, appearance::TERMINAL_BUTTON],
                egui::Label::new(
                    egui::RichText::new(branch.as_deref().unwrap_or(""))
                        .size(12.0)
                        .color(ui.visuals().weak_text_color()),
                )
                .truncate()
                .sense(egui::Sense::click()),
            )
            .on_hover_text(&git_tip);
        let git = git_icon.union(branch_label);
        let split_vertical = appearance::sidebar_action(ui, "Columns2", &vertical_tip);
        let split_horizontal = appearance::sidebar_action(ui, "Rows2", &horizontal_tip);
        #[cfg(feature = "test-support")]
        if let Some(sid) = &sid {
            diagnostics::record(ui.ctx(), &format!("pane-git:{sid}"), git.rect);
            diagnostics::record(
                ui.ctx(),
                &format!("pane-split-vertical:{sid}"),
                split_vertical.rect,
            );
            diagnostics::record(
                ui.ctx(),
                &format!("pane-split-horizontal:{sid}"),
                split_horizontal.rect,
            );
        }
        if git.clicked() {
            if let Some(sid) = sid.clone() {
                self.app.active_session = Some(sid.clone());
                self.app.focus_strip_tab = Some(Tab::Terminal(sid));
            }
            self.app.show_git_sidebar();
        }
        if split_vertical.clicked() || split_horizontal.clicked() {
            if let Some(sid) = &sid {
                self.app.active_session = Some(sid.clone());
                self.app.focus_strip_tab = Some(Tab::Terminal(sid.clone()));
            }
            let direction = if split_vertical.clicked() {
                "right"
            } else {
                "down"
            };
            self.app.add_strip_tab = Some((path, Some(direction.into())));
        }
    }

    pub(super) fn markdown_header(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        editing: bool,
    ) -> (egui::Response, Option<egui::Response>) {
        let sid = &session.id;
        let mode = self
            .app
            .preferences
            .markdown_modes
            .get(sid)
            .copied()
            .unwrap_or_default();
        let header = appearance::markdown_header(
            ui,
            &session.label,
            self.app.active_session.as_ref() == Some(sid),
            editing,
            mode,
        );
        #[cfg(feature = "test-support")]
        {
            diagnostics::record(ui.ctx(), "markdown-title", header.title.rect);
            diagnostics::record(ui.ctx(), "markdown-refresh", header.refresh.rect);
        }
        for (option, response) in header.modes {
            #[cfg(feature = "test-support")]
            {
                diagnostics::record(
                    ui.ctx(),
                    &format!("markdown-mode:{}", option.label()),
                    response.rect,
                );
                diagnostics::record(
                    ui.ctx(),
                    &format!("markdown-mode:{sid}:{}", option.label()),
                    response.rect,
                );
            }
            if response.clicked() {
                self.app.markdown.retain(sid).editor_focused = option != markdown::Mode::Preview;
                self.app.active_session = Some(sid.clone());
                self.app.focus_tab = Some(Tab::Terminal(sid.clone()));
                if option == markdown::Mode::default() {
                    self.app.preferences.markdown_modes.remove(sid);
                } else {
                    self.app
                        .preferences
                        .markdown_modes
                        .insert(sid.clone(), option);
                }
            }
        }
        if header.refresh.clicked() {
            self.app.markdown.refresh(ui.ctx());
        }
        (header.title, Some(header.close))
    }
    pub(super) fn markdown_view(&mut self, ui: &mut egui::Ui, session: &Session) {
        let sid = &session.id;
        let mode = self
            .app
            .preferences
            .markdown_modes
            .get(sid)
            .copied()
            .unwrap_or_default();
        let preview = self.app.markdown.retain(sid);
        if mode == markdown::Mode::Edit {
            preview.editor_focused = true;
        } else {
            preview.pointer_focus(ui);
        }
        let mut link = None;
        if mode != markdown::Mode::Edit {
            self.app.markdown.watch(markdown::Source::new(
                &self.app.state.session_paths(&self.app.paths, &session.id),
                session,
            ));
        }
        match mode {
            markdown::Mode::Edit => self.terminal_view(ui, session),
            markdown::Mode::Preview => link = self.app.markdown.retain(sid).show(ui, sid),
            markdown::Mode::Split => {
                let width = ui.available_width();
                egui::Panel::left(egui::Id::new(("markdown-editor", sid)))
                    .resizable(true)
                    .default_size(width * 0.5)
                    .size_range(80.0..=(width - 80.0).max(80.0))
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| self.terminal_view(ui, session));
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE)
                    .show(ui, |ui| link = self.app.markdown.retain(sid).show(ui, sid));
            }
        }
        if let Some(link) = link {
            match link {
                markdown::Link::File(path) => self.app.terminal_action(
                    ui.ctx(),
                    session,
                    &services::Target::File(path, None, None),
                    FileAction::Open,
                ),
                markdown::Link::Web(url) => {
                    let _ = self.app.jobs.send(Job::Browser(url));
                }
            }
        }
    }
    pub(super) fn find_paint_for(&self, sid: &str) -> Option<egui_term::FindPaint> {
        let find = self.app.terminal_find.get(sid)?;
        if find.query.is_empty() || find.outcome.matches.is_empty() {
            return None;
        }
        Some(egui_term::FindPaint {
            matches: find.outcome.matches.clone(),
            current: find.current,
        })
    }

    /// In-terminal find bar. Search runs GUI-side over the live grid plus
    /// retained scrollback and never writes to the PTY.
    pub(super) fn terminal_find_bar(&mut self, ui: &mut egui::Ui, sid: &str) {
        if !self.app.terminal_find.contains_key(sid) {
            return;
        }
        let mut close = false;
        let mut reveal: Option<i32> = None;
        {
            let Some(find) = self.app.terminal_find.get_mut(sid) else {
                return;
            };
            let Some(backend) = self.app.backends.get_mut(sid) else {
                return;
            };
            let id = egui::Id::new(("terminal-find", sid));
            ui.horizontal(|ui| {
                let response = ui.add(
                    appearance::singleline(&mut find.query)
                        .id(id)
                        .hint_text("Find in terminal")
                        .desired_width(220.0),
                );
                if response.changed() {
                    find.current = 0;
                }
                let case_label = if find.case_insensitive { "aa" } else { "Aa" };
                if ui
                    .small_button(case_label)
                    .on_hover_text("Match case")
                    .clicked()
                {
                    find.case_insensitive = !find.case_insensitive;
                    find.current = 0;
                }
                let now = Instant::now();
                let stale = find
                    .last_search
                    .is_none_or(|t| now.duration_since(t).as_millis() > 250);
                if find.dirty() || (stale && !find.query.is_empty()) {
                    let was_dirty = find.dirty();
                    find.outcome = backend.find(&find.query, find.case_insensitive);
                    find.searched_query.clone_from(&find.query);
                    find.searched_case = find.case_insensitive;
                    find.last_search = Some(now);
                    if was_dirty {
                        find.current = 0;
                    } else {
                        find.current = find
                            .current
                            .min(find.outcome.matches.len().saturating_sub(1));
                    }
                    if let Some(hit) = find.outcome.matches.get(find.current) {
                        reveal = Some(hit.line);
                    }
                }
                let total = find.outcome.matches.len();
                let label = if find.query.is_empty() {
                    String::new()
                } else if total == 0 {
                    "No matches".to_string()
                } else {
                    let mut text = format!("{}/{}", find.current.saturating_add(1), total);
                    if find.outcome.truncated {
                        text.push('+');
                    }
                    text
                };
                ui.monospace(label);
                let shift = ui.input(|i| i.modifiers.shift);
                if ui
                    .small_button("↑")
                    .on_hover_text("Previous (Shift+Enter)")
                    .clicked()
                    || (response.has_focus()
                        && shift
                        && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                {
                    find.step(-1);
                    reveal = find.outcome.matches.get(find.current).map(|hit| hit.line);
                }
                if ui.small_button("↓").on_hover_text("Next (Enter)").clicked()
                    || (response.has_focus()
                        && !shift
                        && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                {
                    find.step(1);
                    reveal = find.outcome.matches.get(find.current).map(|hit| hit.line);
                }
                if ui.small_button("✕").on_hover_text("Close (Esc)").clicked() {
                    close = true;
                }
                if response.has_focus()
                    && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
                {
                    close = true;
                }
                if response.has_focus() {
                    ui.ctx().request_repaint_after(Duration::from_millis(250));
                }
            });
        }
        if let Some(line) = reveal
            && let Some(backend) = self.app.backends.get_mut(sid)
        {
            backend.reveal_grid_line(line);
        }
        if close {
            self.app.terminal_find.remove(sid);
        }
    }
}
