use super::super::*;
use super::viewer::Viewer;
use crate::app::TerminalActionInput;
pub(crate) struct StripTerminalClicksInput {
    pub(crate) sid: Option<String>,
    pub(crate) path: egui_dock::NodePath,
    pub(crate) git_clicked: bool,
    pub(crate) vertical_clicked: bool,
    pub(crate) horizontal_clicked: bool,
}
impl Viewer<'_> {
    /// The strip leaf's terminal the actions target: the active session
    /// when it lives in this leaf, else the leaf's first terminal.
    fn strip_pane_session(&self, path: egui_dock::NodePath) -> Option<String> {
        let tabs = self
            .app
            .strip_pane_tabs
            .get(&path)
            .cloned()
            .unwrap_or_default();
        self.app
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
            })
    }

    pub(super) fn strip_terminal_actions(&mut self, ui: &mut egui::Ui, path: egui_dock::NodePath) {
        let sid = self.strip_pane_session(path);
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
        let vertical_tip = self.app.action_tip("Split vertically", Action::SplitRight);
        let horizontal_tip = self.app.action_tip("Split horizontally", Action::SplitDown);
        // Same rule as pane captions: the split buttons show while this
        // leaf has focus or the pointer is on its controls.
        let show = sid.is_some() && sid == self.app.active_session
            || ui.rect_contains_pointer(ui.max_rect());
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
        // The branch is plain weak text; a click opens Git.
        let git = ui
            .add_sized(
                [appearance::TERMINAL_BRANCH_MAX, appearance::TERMINAL_BUTTON],
                egui::Label::new(
                    egui::RichText::new(branch.as_deref().unwrap_or(""))
                        .size(appearance::FONT_SMALL)
                        .color(ui.visuals().weak_text_color()),
                )
                .truncate()
                .sense(egui::Sense::click()),
            )
            .on_hover_text(&git_tip);
        let action = |ui: &mut egui::Ui, icon: &str, tip: &str| {
            if show {
                appearance::sidebar_action(ui, icon, tip)
            } else {
                // Keep the slot so the row does not move when it shows.
                ui.allocate_exact_size(
                    egui::Vec2::splat(appearance::TERMINAL_BUTTON),
                    egui::Sense::hover(),
                )
                .1
            }
        };
        let split_vertical = action(ui, "Columns2", &vertical_tip);
        let split_horizontal = action(ui, "Rows2", &horizontal_tip);
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
        self.strip_terminal_clicks(StripTerminalClicksInput {
            sid,
            path,
            git_clicked: git.clicked(),
            vertical_clicked: split_vertical.clicked(),
            horizontal_clicked: split_horizontal.clicked(),
        });
    }

    fn strip_terminal_clicks(&mut self, input: StripTerminalClicksInput) {
        let StripTerminalClicksInput {
            sid,
            path,
            git_clicked,
            vertical_clicked,
            horizontal_clicked,
        } = input;
        if git_clicked {
            if let Some(sid) = sid.clone() {
                self.app.focus_terminal(sid.clone());
                self.app.queue_focus(DockTarget::Strip, Tab::Terminal(sid));
            }
            self.app.show_git_sidebar();
        }
        if vertical_clicked || horizontal_clicked {
            if let Some(sid) = &sid {
                self.app.focus_terminal(sid.clone());
                self.app
                    .queue_focus(DockTarget::Strip, Tab::Terminal(sid.clone()));
            }
            let direction = if vertical_clicked { "right" } else { "down" };
            self.app
                .queue_add(DockTarget::Strip, path, Some(direction.into()));
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
        let header = appearance::markdown_header(appearance::MarkdownHeaderInput {
            ui,
            title: &session.label,
            active: self.app.active_session.as_ref() == Some(sid),
            editing,
            mode,
        });
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
                self.app.focus_terminal(sid.clone());
                self.app.queue_main_focus(Tab::Terminal(sid.clone()));
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
                egui::Panel::left(egui::Id::unique(("markdown-editor", sid)))
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
                markdown::Link::File(path) => self.app.terminal_action(TerminalActionInput {
                    ctx: ui.ctx(),
                    session,
                    target: &services::Target::File(path, None, None),
                    action: FileAction::Open,
                }),
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
    /// retained scrollback and never writes to the PTY. It rescans only when
    /// the query or case option changes, or when new output arrived (#176).
    pub(super) fn terminal_find_bar(&mut self, ui: &mut egui::Ui, sid: &str) {
        let mut close = false;
        let mut reveal: Option<i32> = None;
        {
            let Some(find) = self.app.terminal_find.get_mut(sid) else {
                return;
            };
            let Some(backend) = self.app.backends.get_mut(sid) else {
                return;
            };
            let id = egui::Id::unique(("terminal-find", sid));
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
                Self::find_case_toggle(ui, find);
                reveal = Self::find_rescan(ui, find, backend);
                let (nav_reveal, nav_close) = Self::find_bar_nav(ui, find, &response);
                if nav_reveal.is_some() {
                    reveal = nav_reveal;
                }
                close = nav_close;
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

    /// Frameless toggle that shows its selected state while matching case.
    fn find_case_toggle(ui: &mut egui::Ui, find: &mut TerminalFind) {
        if appearance::selectable_icon(appearance::SelectableIconInput {
            ui,
            icon: "CaseSensitive",
            tip: "Match case",
            selected: !find.case_insensitive,
        })
        .clicked()
        {
            find.case_insensitive = !find.case_insensitive;
            find.current = 0;
        }
    }

    /// Re-runs find when due; returns the line of the current hit to reveal.
    fn find_rescan(
        ui: &egui::Ui,
        find: &mut TerminalFind,
        backend: &egui_term::TerminalBackend,
    ) -> Option<i32> {
        let now = Instant::now();
        let wait = find.rescan_in(now)?;
        if !wait.is_zero() {
            ui.ctx().request_repaint_after(wait);
            return None;
        }
        let was_dirty = find.dirty();
        find.outcome = backend.find(&find.query, find.case_insensitive);
        find.searched_query.clone_from(&find.query);
        find.searched_case = find.case_insensitive;
        find.last_search = Some(now);
        find.content_stale = false;
        find.current = if was_dirty {
            0
        } else {
            find.current
                .min(find.outcome.matches.len().saturating_sub(1))
        };
        find.outcome.matches.get(find.current).map(|hit| hit.line)
    }

    pub(super) fn find_bar_nav(
        ui: &mut egui::Ui,
        find: &mut TerminalFind,
        response: &egui::Response,
    ) -> (Option<i32>, bool) {
        let mut reveal = None;
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
        // A single-line field gives up focus on Enter, and egui drops it on
        // Esc before the field runs: test `lost_focus` too.
        let field = response.has_focus() || response.lost_focus();
        let enter = field && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if enter {
            response.request_focus();
        }
        if appearance::sidebar_action(ui, "ArrowUp", "Previous (Shift+Enter)").clicked()
            || (enter && shift)
        {
            find.step(-1);
            reveal = find.outcome.matches.get(find.current).map(|hit| hit.line);
        }
        if appearance::sidebar_action(ui, "ArrowDown", "Next (Enter)").clicked()
            || (enter && !shift)
        {
            find.step(1);
            reveal = find.outcome.matches.get(find.current).map(|hit| hit.line);
        }
        let close = appearance::sidebar_action(ui, "X", "Close (Esc)").clicked()
            || (field && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)));
        (reveal, close)
    }
}
