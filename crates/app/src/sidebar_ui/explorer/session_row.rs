//! Session and worktree rows in the project sidebar.
use super::*;

impl App {
    pub(super) fn session_row_for_tab(
        &mut self,
        ui: &mut egui::Ui,
        session: &Session,
        rename_tab: Option<(&str, &str)>,
    ) {
        let presented = self.present_session(&session.id);
        let editing = self.renaming(&session.id, RenameSurface::Sidebar);
        let height = if !editing && presented.notice_preview.is_some() {
            appearance::SESSION_ROW_DETAIL_HEIGHT
        } else {
            appearance::SESSION_ROW_HEIGHT
        };
        if !editing && skip_clipped_row(ui, height) {
            return;
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
            self.go_session(&session.id);
        }
        appearance::context_menu(&response, |ui| {
            self.session_row_menu(ui, session, rename_tab);
        });
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
        let color = appearance::color(if unread_terminal {
            &self.theme.accent
        } else {
            &self.theme.secondary
        });
        let secondary = self.session_row_secondary(session);
        // The status icon carries agent state; the dot is for terminal notices.
        let trailing = if editing {
            ""
        } else if !secondary.is_empty() {
            secondary
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
                selected: self.active_session.as_ref() == Some(&session.id),
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
        self.rename_action(ui, &session.id, RenameSurface::Sidebar);
        ui.separator();
        if appearance::menu_item(ui, "Open session", "Terminal", "").clicked() {
            self.go_session(&session.id);
            ui.close();
        }
        if let Some((project_id, tab_id)) = rename_tab
            && appearance::menu_item(ui, "Name tab…", "Pencil", "").clicked()
        {
            self.rename_group = Some((project_id.to_string(), tab_id.to_string()));
            self.rename_group_text.clear();
            self.rename_group_focus = true;
            ui.close();
        }
        ui.separator();
        if session.lifecycle.live() {
            if appearance::menu_item(
                ui,
                "Close session…",
                "X",
                &self.shortcut_label(Action::CloseSession),
            )
            .clicked()
            {
                self.close_session = Some(session.id.clone());
                ui.close();
            }
        } else if appearance::menu_item(ui, "Remove historical record", "X", "").clicked() {
            self.send(Request::Remove {
                session: session.id.clone(),
            });
            self.remove_tab(&session.id);
            ui.close();
        }
    }

    pub(in crate::sidebar_ui) fn worktree_card(&mut self, ui: &mut egui::Ui, project: &Project) {
        let index = self.sidebar_index();
        let live = index.live_counts.get(&project.id).copied().unwrap_or(0);
        let (waiting, running) = index
            .worktree_states
            .get(&project.id)
            .copied()
            .unwrap_or_default();
        let tint = appearance::color(if waiting {
            &self.theme.status_waiting
        } else if running {
            &self.theme.status_running
        } else {
            &self.theme.secondary
        });
        let count = live.to_string();
        let selected = self.selected.as_ref() == Some(&project.id);
        let response = appearance::project_row(appearance::ProjectRowInput {
            ui,
            label: &project.name,
            icon: "GitBranch",
            selected,
            height: self.theme.row_height(),
            trailing: &count,
            tint,
        })
        .on_hover_text(format!(
            "{}\nManaged Git worktree\n{} live terminal(s)",
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
            self.select_project(project.id.clone());
        }
        appearance::context_menu(&response, |ui| {
            if appearance::menu_item(ui, "Open", "FolderOpen", "").clicked() {
                self.select_project(project.id.clone());
                ui.close();
            }
            if appearance::menu_item(
                ui,
                "New terminal",
                "Terminal",
                &self.shortcut_label(Action::NewTerminal),
            )
            .clicked()
            {
                self.select_project(project.id.clone());
                self.create(None);
                ui.close();
            }
            ui.separator();
            if appearance::menu_item(ui, "Remove worktree…", "X", "").clicked() {
                self.confirm_remove_worktree(&project.id);
                ui.close();
            }
        });
    }
}

/// What one session row shows.
struct SessionRowView<'a> {
    session: &'a Session,
    presented: &'a crate::agent_presence::AgentPresentation,
    editing: bool,
    note: Option<&'a terminator_core::TerminalNotice>,
}
