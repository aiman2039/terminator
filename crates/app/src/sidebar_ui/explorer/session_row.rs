//! Session and worktree rows in the project sidebar.
use super::*;
use crate::app::missing_folder::{MISSING_ICON, MISSING_TOOLTIP, missing_folder_text};

impl App {
    pub(super) fn session_row_for_tab(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        rename_tab: Option<(&str, &str)>,
    ) -> Option<egui::Response> {
        self.session_selection.painted(&session.id);
        let presented = self.present_session(&session.id);
        let editing = self.renaming(&session.id, RenameSurface::Sidebar);
        let height = if !editing && presented.notice_preview.is_some() {
            appearance::SESSION_ROW_DETAIL_HEIGHT
        } else {
            appearance::SESSION_ROW_HEIGHT
        };
        if !editing && skip_clipped_row(ui, height) {
            return None;
        }
        let index = self.sidebar_index();
        let response = self.session_row_widget(
            ui,
            SessionRowView {
                session,
                presented: &presented,
                editing,
                note: index.terminal_notes.get(&session.id),
            },
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(
            ui.ctx(),
            &format!("session-row:{}", session.id),
            response.rect,
        );
        if editing {
            self.inline_rename(crate::workspace_ui::InlineRenameInput {
                ui,
                sid: &session.id,
                surface: RenameSurface::Sidebar,
                rect: egui::Rect::from_min_max(
                    egui::pos2(response.rect.min.x + 36.0, response.rect.min.y + 4.0),
                    egui::pos2(response.rect.max.x - 6.0, response.rect.max.y - 3.0),
                ),
            });
        }
        if response.clicked() && !editing {
            let modifiers = ui.input(|input| input.modifiers);
            let current = self.active_session.clone();
            if self
                .session_selection
                .click(&session.id, current.as_deref(), modifiers)
            {
                self.go_session(&session.id);
            }
        }
        appearance::context_menu(&response, |ui| {
            self.session_row_menu(ui, session, rename_tab);
        });
        Some(response)
    }

    /// The row itself: status icon, label, trailing state, and hover text.
    fn session_row_widget(&self, ui: &mut egui::Ui, view: SessionRowView<'_>) -> egui::Response {
        let SessionRowView {
            session,
            presented,
            editing,
            note,
        } = view;
        let unread_terminal = note.is_some_and(|n| !n.dismissed);
        let secondary = self.session_row_secondary(session);
        // A live agent row shows its state ("waiting 2m", "working") in the
        // state color; "ended" and "background" still win.
        let agent_state = presented
            .row_state(now())
            .filter(|_| secondary.is_empty() && !editing);
        let color = match (presented.lifecycle, &agent_state) {
            (Some(state), Some(_)) => state_color(state, &self.theme),
            _ => appearance::color(if unread_terminal {
                &self.theme.accent
            } else {
                &self.theme.secondary
            }),
        };
        // The status icon carries agent state; the dot is for terminal notices.
        let trailing = if editing {
            ""
        } else if !secondary.is_empty() {
            secondary
        } else if let Some(state) = &agent_state {
            state
        } else if unread_terminal {
            "●"
        } else {
            ""
        };
        appearance::session_row_spec(
            ui,
            appearance::SessionRowSpec {
                label: if editing { "" } else { &session.label },
                icon: match presented.lifecycle {
                    Some(_) => presented.status_icon,
                    None if session.kind == SessionKind::Editor => "FileCode",
                    None => "Terminal",
                },
                selected: self.active_session.as_ref() == Some(&session.id)
                    || self.session_selection.selected(&session.id),
                trailing,
                tint: color,
                icon_tint: presented
                    .lifecycle
                    .map(|state| state_color(state, &self.theme)),
                spin: presented.spin,
                subtitle: presented.notice_preview.as_deref().filter(|_| !editing),
                brand: presented.brand_icon,
            },
        )
        .on_hover_ui(|ui| {
            ui.label(format!(
                "{}{}{}",
                presented.diagnostics(now()),
                if secondary.is_empty() {
                    String::new()
                } else {
                    format!("\n{secondary}")
                },
                note.map(|n| format!("\nTerminal: {}\n{}", n.title, n.body))
                    .unwrap_or_default()
            ));
        })
    }

    /// "ended" or "background" for a session not live in its layout.
    fn session_row_secondary(&self, session: &Session) -> &'static str {
        let visible = self
            .layouts
            .get(&session.project_id)
            .is_some_and(|d| d.contains(&Tab::Terminal(session.id.clone())));
        if !session.lifecycle.live() {
            "ended"
        } else if !visible {
            "background"
        } else {
            ""
        }
    }

    fn session_row_menu(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        rename_tab: Option<(&str, &str)>,
    ) {
        if let Some(sids) = self.session_selection.bulk(&session.id) {
            self.session_bulk_menu(ui, &session.project_id, &sids);
            return;
        }
        self.rename_action(ui, &session.id, RenameSurface::Sidebar);
        ui.separator();
        if appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: "Open session",
            icon: "Terminal",
            shortcut: "",
        })
        .clicked()
        {
            self.go_session(&session.id);
            ui.close();
        }
        if session.lifecycle.live() {
            self.open_in_split_items(ui, &session.id);
        }
        if let Some((project_id, tab_id)) = rename_tab
            && appearance::menu_item(appearance::MenuItemInput {
                ui,
                label: "Name tab…",
                icon: "Pencil",
                shortcut: "",
            })
            .clicked()
        {
            self.begin_group_rename(project_id, tab_id, String::new());
            ui.close();
        }
        ui.separator();
        if session.lifecycle.live() {
            if appearance::menu_item(appearance::MenuItemInput {
                ui,
                label: "Close session…",
                icon: "X",
                shortcut: &self.shortcut_label(Action::CloseSession),
            })
            .clicked()
            {
                self.close_session = Some(session.id.clone());
                ui.close();
            }
        } else {
            if let Some(command) = self.resume_command(&session.id)
                && appearance::menu_item(appearance::MenuItemInput {
                    ui,
                    label: "Resume conversation",
                    icon: "Play",
                    shortcut: "",
                })
                .on_hover_text(&command)
                .clicked()
            {
                self.resume_conversation(session, command);
                ui.close();
            }
            self.remove_record_item(ui, &session.id);
        }
    }

    fn open_in_split_items(&mut self, ui: &mut egui::Ui, sid: &str) {
        for (label, icon, split) in [
            (
                "Open in split right",
                "PanelRightClose",
                egui_dock::Split::Right,
            ),
            (
                "Open in split down",
                "PanelBottomClose",
                egui_dock::Split::Below,
            ),
        ] {
            if appearance::menu_item(appearance::MenuItemInput {
                ui,
                label,
                icon,
                shortcut: "",
            })
            .clicked()
            {
                self.open_session_in_split(sid, split);
                ui.close();
            }
        }
    }

    /// Removing a historical record cannot be undone: the first click arms
    /// the row (the menu stays open), the second removes.
    fn remove_record_item(&mut self, ui: &mut egui::Ui, sid: &str) {
        let armed_id = egui::Id::unique(("remove-history-armed", sid));
        let armed = ui
            .data(|data| data.get_temp::<bool>(armed_id))
            .unwrap_or(false);
        let label = if armed {
            "Remove record: click again to confirm"
        } else {
            "Remove historical record…"
        };
        if !appearance::menu_item(appearance::MenuItemInput {
            ui,
            label,
            icon: "X",
            shortcut: "",
        })
        .clicked()
        {
            return;
        }
        if armed {
            ui.data_mut(|data| data.remove::<bool>(armed_id));
            self.send(Request::Remove {
                session: sid.to_owned(),
            });
            self.remove_tab(sid);
            ui.close();
        } else {
            ui.data_mut(|data| data.insert_temp(armed_id, true));
        }
    }

    /// A two-line worktree row: branch leaf (else the folder name) and state
    /// counters, then the newest agent state and `+added −deleted` from a
    /// read-only local Git job (none for SSH projects).
    pub(in crate::sidebar_ui) fn worktree_card(&mut self, ui: &mut egui::Ui, project: &Project) {
        let live = self
            .sidebar_index()
            .live_counts
            .get(&project.id)
            .copied()
            .unwrap_or(0);
        let count = live.to_string();
        let selected = self.selected.as_ref() == Some(&project.id);
        // A deleted checkout: dim name and a warning icon, like other projects.
        let missing = self.project_folder_missing(&project.id);
        self.ask_worktree_stats(project, missing);
        let git = self.worktree_stats.get(&project.path);
        let label = git
            .and_then(|git| git.branch_leaf())
            .unwrap_or(&project.name)
            .to_owned();
        let counters = self.project_state_counters(&project.id);
        let detail = self.worktree_detail(project, git);
        let detail: Vec<(&str, egui::Color32)> = detail
            .iter()
            .map(|(text, color)| (text.as_str(), *color))
            .collect();
        let height = if detail.is_empty() {
            self.theme.row_height()
        } else {
            appearance::SESSION_ROW_DETAIL_HEIGHT
        };
        let response = ui
            .scope(|ui| {
                if missing {
                    ui.visuals_mut().override_text_color = Some(ui.visuals().weak_text_color());
                }
                appearance::project_row_with(
                    appearance::ProjectRowInput {
                        color: None,
                        ui,
                        label: &label,
                        icon: if missing { MISSING_ICON } else { "GitBranch" },
                        selected,
                        height,
                        trailing: &count,
                        tint: appearance::color(&self.theme.secondary),
                    },
                    appearance::RowExtras {
                        counters: &counters,
                        detail: &detail,
                        tint_words: false,
                    },
                )
            })
            .inner
            .on_hover_text(format!(
                "{}{}\nManaged Git worktree\n{} live terminal(s)",
                if missing {
                    format!("{MISSING_TOOLTIP}\n")
                } else {
                    String::new()
                },
                self.project_location_label(&project.id),
                live
            ));
        #[cfg(feature = "test-support")]
        diagnostics::record(
            ui.ctx(),
            &format!("worktree-row:{}", project.id),
            response.rect,
        );
        if response.clicked() {
            self.open_worktree_card(project);
        }
        appearance::context_menu(&response, |ui| self.worktree_card_menu(ui, project));
    }

    /// Starts the worktree's read-only Git job when due (rate-limited per
    /// worktree). SSH and missing checkouts ask nothing.
    fn ask_worktree_stats(&mut self, project: &Project, missing: bool) {
        let remote = self
            .state
            .remote_projects
            .iter()
            .any(|remote| remote.id == project.id);
        if !remote
            && !missing
            && self
                .worktree_stats
                .take_due(&project.path, std::time::Instant::now())
        {
            let base = self
                .preferences
                .task_bases
                .get(&project.path)
                .map(|base| base.commit.clone());
            let _ = self
                .jobs
                .send(crate::Job::WorktreeStats(project.path.clone(), base));
        }
    }

    /// The worktree row's second line: the newest agent state in its
    /// color, then `+added −deleted` in the Git colors.
    fn worktree_detail(
        &self,
        project: &Project,
        git: Option<&crate::worktree_stats::WorktreeGit>,
    ) -> Vec<(String, egui::Color32)> {
        let index = self.sidebar_index();
        let state = index
            .by_project
            .get(&project.id)
            .into_iter()
            .flatten()
            .filter(|session| session.lifecycle.live())
            .filter_map(|session| {
                let presented = self.present_session(&session.id);
                let state = presented.lifecycle?;
                Some((state, presented.row_state(now())?))
            })
            .min_by_key(|(state, _)| detail_rank(*state));
        let mut parts = Vec::new();
        if let Some((state, text)) = state {
            parts.push((text, state_color(state, &self.theme)));
        }
        if let Some(git) = git.filter(|git| git.added > 0 || git.deleted > 0) {
            if !parts.is_empty() {
                parts.push((" · ".to_owned(), appearance::color(&self.theme.secondary)));
            }
            parts.push((
                format!("+{}", git.added),
                appearance::color(&self.theme.git_added),
            ));
            parts.push((
                format!(" −{}", git.deleted),
                appearance::color(&self.theme.git_deleted),
            ));
        }
        // Only a pull request the GUI already read; never a new call.
        let branch = git.and_then(|git| git.branch.as_deref());
        if let Some((number, _)) = branch.and_then(|b| self.known_pr(&project.path, Some(b))) {
            if !parts.is_empty() {
                parts.push((" · ".to_owned(), appearance::color(&self.theme.secondary)));
            }
            parts.push((
                format!("PR #{number}"),
                appearance::color(&self.theme.secondary),
            ));
        }
        parts
    }

    /// A missing checkout with no live terminal cannot open; say why.
    pub(crate) fn open_worktree_card(&mut self, project: &Project) {
        if !self.project_available(&project.id) && self.project_folder_missing(&project.id) {
            self.info = Some(missing_folder_text(&project.path));
        } else {
            self.select_project(project.id.clone());
        }
    }

    /// A deleted checkout cannot start a terminal; say why. A worktree that
    /// cannot be selected never gets a terminal in another project.
    pub(crate) fn new_worktree_terminal(&mut self, project: &Project) {
        if self.project_folder_missing(&project.id) {
            self.info = Some(missing_folder_text(&project.path));
            return;
        }
        self.select_project(project.id.clone());
        if self.selected.as_deref() == Some(project.id.as_str()) {
            self.create(None);
        }
    }
}

/// Which agent state a worktree row names first: waiting, failed, working,
/// done (the state counter order).
fn detail_rank(state: AgentState) -> u8 {
    match state {
        AgentState::WaitingInput | AgentState::WaitingPermission => 0,
        AgentState::Failed => 1,
        AgentState::Running => 2,
        _ => 3,
    }
}

/// What one session row shows.
struct SessionRowView<'a> {
    session: &'a Session,
    presented: &'a crate::agent_presence::AgentPresentation,
    editing: bool,
    note: Option<&'a terminator_core::TerminalNotice>,
}
