//! The All live agents tab: search, filter, and grouped live rows.
use super::*;

impl App {
    /// Verified live agents grouped by project/worktree, plus hook-only and
    /// unavailable-owner entries under Presence unverified.
    pub(in crate::sidebar_ui) fn agents_all_live(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| self.agents_search_bar(ui));
        let rows = self.cached_live_rows();
        let query_empty = self.preferences.agents_search.trim().is_empty();
        let kind_filter = self.preferences.agents_filter.clone();
        let unverified = &rows.unverified;
        appearance::sidebar_scroll("agents-live").show(ui, |ui| {
            if rows.groups.is_empty() && unverified.is_empty() {
                ui.weak(if query_empty && kind_filter.is_empty() {
                    "No live agents"
                } else {
                    "No matching agents"
                });
                return;
            }
            for (project, sessions) in &rows.groups {
                self.live_project_group(ui, project, sessions);
                ui.add_space(4.0);
            }
            if !unverified.is_empty() {
                self.live_unverified_group(ui, unverified);
            }
        });
    }

    pub(super) fn agents_search_bar(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.x = 4.0;
        let width = (ui.available_width() - 132.0).max(80.0);
        let response = ui.add_sized(
            egui::vec2(width, 22.0),
            appearance::singleline(&mut self.preferences.agents_search)
                .id(ui.scope_id().with("agents-search"))
                .hint_text("Search agents"),
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "agents-search", response.rect);
        #[cfg(not(feature = "test-support"))]
        let _ = &response;
        if !self.preferences.agents_search.is_empty()
            && appearance::sidebar_action(ui, "X", "Clear search").clicked()
        {
            self.preferences.agents_search.clear();
        }
        self.agents_filter_menu(ui);
    }

    /// Flip a collapsed key in the live-agents tree.
    pub(super) fn toggle_agents_collapsed(&mut self, key: &str, collapsed: bool) {
        if collapsed {
            self.preferences.agents_collapsed.remove(key);
        } else {
            self.preferences.agents_collapsed.insert(key.to_owned());
        }
    }

    pub(super) fn live_project_group(
        &mut self,
        ui: &mut egui::Ui,
        project: &terminator_core::Project,
        sessions: &[std::sync::Arc<Session>],
    ) {
        let collapsed = self.preferences.agents_collapsed.contains(&project.id);
        let header = appearance::row(appearance::RowInput {
            ui,
            label: &project.name,
            icon: if collapsed {
                "ChevronRight"
            } else {
                "ChevronDown"
            },
            selected: false,
            height: 26.0,
            trailing: &sessions.len().to_string(),
            tint: appearance::color(&self.theme.text),
        })
        .on_hover_text(format!(
            "{}{}{}",
            self.project_location_label(&project.id),
            if self.managed_worktree(&project.id).is_some() {
                "\nManaged Git worktree"
            } else {
                ""
            },
            "\nVerified live agents"
        ));
        #[cfg(feature = "test-support")]
        diagnostics::record(
            ui.ctx(),
            &format!("live-project:{}", project.id),
            header.rect,
        );
        if header.clicked() {
            self.toggle_agents_collapsed(&project.id, collapsed);
        }
        if !collapsed {
            ui.indent(("live-project", &project.id), |ui| {
                for session in sessions {
                    self.live_agent_row(ui, session, "live-row");
                }
            });
        }
    }

    pub(super) fn live_unverified_group(
        &mut self,
        ui: &mut egui::Ui,
        unverified: &[std::sync::Arc<Session>],
    ) {
        let collapsed = self.preferences.agents_collapsed.contains("unverified");
        let header = appearance::row(appearance::RowInput {
            ui,
            label: "Presence unverified",
            icon: if collapsed {
                "ChevronRight"
            } else {
                "ChevronDown"
            },
            selected: false,
            height: 26.0,
            trailing: &unverified.len().to_string(),
            tint: appearance::color(&self.theme.status_waiting),
        })
        .on_hover_text(
            "Hook-only or unavailable-owner sessions.\nLast reported hook state; no verified live process.",
        );
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "live-unverified", header.rect);
        if header.clicked() {
            self.toggle_agents_collapsed("unverified", collapsed);
        }
        if !collapsed {
            ui.indent("live-unverified", |ui| {
                for session in unverified {
                    self.live_agent_row(ui, session, "unverified-row");
                }
            });
        }
    }

    pub(super) fn live_agent_row(&mut self, ui: &mut egui::Ui, session: &Session, target: &str) {
        if skip_clipped_row(ui, appearance::SESSION_ROW_DETAIL_HEIGHT) {
            return;
        }
        let presented = self.present_session(&session.id);
        let mut subtitle = match (&presented.brand_label, presented.status_label.as_str()) {
            (Some(brand), status) => format!("{brand} · {status}"),
            (None, status) => status.to_string(),
        };
        if !presented.verified {
            subtitle.push_str(" · unverified");
        }
        let unread = if presented.unread > 0 {
            format!("{} unread", presented.unread)
        } else {
            String::new()
        };
        let response = appearance::session_row_spec(
            ui,
            appearance::SessionRowSpec {
                label: &session.label,
                icon: match presented.lifecycle {
                    Some(_) => presented.status_icon,
                    None => "Terminal",
                },
                selected: self.active_session.as_ref() == Some(&session.id),
                trailing: &unread,
                tint: appearance::color(&self.theme.secondary),
                icon_tint: presented
                    .lifecycle
                    .map(|state| state_color(state, &self.theme)),
                spin: presented.spin,
                subtitle: Some(&subtitle),
                brand: presented.brand_icon,
            },
        )
        .on_hover_ui(|ui| {
            ui.label(presented.diagnostics(now()));
        });
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), &format!("{target}:{}", session.id), response.rect);
        #[cfg(not(feature = "test-support"))]
        let _ = target;
        if response.clicked() {
            self.go_session(&session.id);
        }
    }

    pub(super) fn agents_filter_menu(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().interact_size.y = 22.0;
        ui.spacing_mut().button_padding = egui::vec2(6.0, 3.0);
        let current = self.preferences.agents_filter.clone();
        let label = if current.is_empty() {
            "All agents".to_string()
        } else {
            terminator_core::agents::display_name(&current).to_string()
        };
        let menu = appearance::menu_button(ui, &label, |ui| {
            let mut kinds: Vec<String> = self
                .state
                .agents
                .iter()
                .map(|a| a.kind.clone())
                .chain(
                    self.state
                        .presence
                        .iter()
                        .flat_map(|p| p.agents.iter().map(|a| a.kind.clone())),
                )
                .collect();
            kinds.sort();
            kinds.dedup();
            for kind in std::iter::once(String::new()).chain(kinds) {
                let name = if kind.is_empty() {
                    "All agents".to_string()
                } else {
                    terminator_core::agents::display_name(&kind).to_string()
                };
                let check = if current == kind { "✓" } else { "" };
                let response = appearance::menu_item(ui, &name, "Search", check);
                #[cfg(feature = "test-support")]
                diagnostics::record(
                    ui.ctx(),
                    &format!(
                        "agents-filter:{}",
                        if kind.is_empty() { "all" } else { &kind }
                    ),
                    response.rect,
                );
                if response.clicked() {
                    self.preferences.agents_filter = kind;
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("Filter by agent");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "agents-filter", menu.rect);
        let _ = menu;
    }
}
