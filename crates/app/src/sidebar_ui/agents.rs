use super::attention::{
    AttentionCard, NoticeGroup, attention_badge_label, attention_card, state_color,
};
use super::explorer_rows::{cached_variable_card, paint_header_count, skip_clipped_row};
#[cfg(feature = "test-support")]
use crate::diagnostics;
use crate::{
    App, appearance,
    preferences::{AgentsTab, SidebarTool},
    settings_ui::SettingsSection,
};
use eframe::egui::{self};
use terminator_core::{Request, Session, TERMINAL_NOTICES_CAPABILITY, now};

/// How one notice group card is drawn.
struct NoticeCardInput<'a> {
    session: Option<&'a Session>,
    highlight: bool,
    show_read: bool,
}

impl App {
    /// Expanded playback controls stay at the top of the project sidebar.
    /// The Player and Agents icons live in the project header.
    pub(crate) fn agent_bar(&mut self, ui: &mut egui::Ui) {
        if !self.player_chrome_expanded() {
            return;
        }
        let spacing = ui.spacing().item_spacing.y;
        ui.spacing_mut().item_spacing.y = 4.0;
        self.player_live_controls(ui);
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.add(egui::Separator::default().spacing(4.0));
        ui.spacing_mut().item_spacing.y = spacing;
    }

    pub(crate) fn agent_bar_badge(&self) -> String {
        let (waiting, unread) = self.attention_counts();
        attention_badge_label(waiting, unread)
    }

    pub(crate) fn note_agent_bar_badge(&self, ui: &egui::Ui) {
        let badge = self.agent_bar_badge();
        #[cfg(feature = "test-support")]
        ui.ctx().data_mut(|data| {
            data.insert_temp(egui::Id::unique("agent-bar-badge"), badge);
        });
        #[cfg(not(feature = "test-support"))]
        let _ = (ui, badge);
    }

    pub(crate) fn toggle_left_agents(&mut self) {
        if !self.preferences.left_agents {
            self.preferences.agents_tab = AgentsTab::NeedsAttention;
        }
        self.preferences.left_agents = !self.preferences.left_agents;
    }

    /// Header bell. Same square as the hide-sidebar control, with a corner count.
    pub(crate) fn header_agents_button(&mut self, ui: &mut egui::Ui) {
        let (waiting, unread) = self.attention_counts();
        let badge = attention_badge_label(waiting, unread);
        let tip = if badge.is_empty() {
            "Pending agent notifications. Click to switch between Agents and Projects.".to_string()
        } else {
            format!("{badge}. Click to switch between Agents and Projects.")
        };
        let response = appearance::framed_icon_button(appearance::FramedIconButtonInput {
            ui,
            icon: "Bell",
            tip: &tip,
            selected: self.preferences.left_agents,
            tint: appearance::icon_color(),
        });
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Agents"));
        let count = waiting.saturating_add(unread);
        if count > 0 {
            paint_header_count(
                ui,
                response.rect,
                count,
                appearance::color(if waiting > 0 {
                    &self.theme.status_waiting
                } else {
                    &self.theme.text
                }),
            );
        }
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "left-agent-bar", response.rect);
        if response.clicked() {
            self.toggle_left_agents();
        }
    }

    pub(crate) fn header_agents_menu(&mut self, ui: &mut egui::Ui) -> bool {
        let badge = self.agent_bar_badge();
        let label = if badge.is_empty() {
            "Agents".to_string()
        } else {
            format!("Agents · {badge}")
        };
        let mark = if self.preferences.left_agents {
            "✓"
        } else {
            ""
        };
        let clicked = appearance::menu_item(ui, &label, "Bell", mark).clicked();
        if clicked {
            self.toggle_left_agents();
        }
        clicked
    }

    pub(crate) fn agents_inbox_open(&self) -> bool {
        self.preferences.left_agents
            || (self.preferences.visible && self.preferences.tool == SidebarTool::Agents)
    }

    /// The existing inbox: pending notifications plus terminal notices.
    pub(super) fn agents_needs_attention(&mut self, ui: &mut egui::Ui) {
        let index = self.sidebar_index();
        let notices = &index.pending_groups;
        let terminal_notices = &index.terminal_pending;
        appearance::sidebar_scroll("agents").show(ui, |ui| {
            if notices.is_empty() && terminal_notices.is_empty() {
                self.agents_empty(ui);
            }
            for group in notices {
                let Some(notice) = group.notices.first() else {
                    continue;
                };
                let highlight = group
                    .notices
                    .iter()
                    .any(|n| self.detail.as_ref() == Some(&n.id));
                self.notice_group_card(
                    ui,
                    group,
                    NoticeCardInput {
                        session: index
                            .sessions
                            .get(&notice.session_id)
                            .map(std::convert::AsRef::as_ref),
                        highlight,
                        show_read: false,
                    },
                );
            }
            for notice in terminal_notices {
                let Some(layout_key) = index.terminal_layout_keys.get(&notice.id).copied() else {
                    continue;
                };
                self.terminal_notice_card(ui, notice, layout_key);
            }
        });
    }

    /// One notice group as an attention card; applies the card's action.
    fn notice_group_card(
        &mut self,
        ui: &mut egui::Ui,
        group: &NoticeGroup,
        input: NoticeCardInput<'_>,
    ) {
        let NoticeCardInput {
            session,
            highlight,
            show_read,
        } = input;
        let Some(notice) = group.notices.first() else {
            return;
        };
        let selected = self.active_session.as_ref() == Some(&notice.session_id);
        let presented = self.present_session(&notice.session_id);
        let history = self.notice_history_brand(notice);
        let brand_icon = history
            .as_ref()
            .map(|(icon, _)| *icon)
            .or(presented.brand_icon);
        let history_label = history.map(|(_, label)| label);
        let brand_label = history_label
            .as_deref()
            .or(presented.brand_label.as_deref());
        let action = attention_card(
            ui,
            AttentionCard {
                theme: &self.theme,
                notice,
                session,
                selected,
                highlight,
                brand_icon,
                brand_label,
                show_read,
                group_extra: group.notices.get(1..).unwrap_or(&[]),
            },
        );
        self.apply_group_action(group, action);
    }

    /// A terminal notice card with Go to terminal and Dismiss.
    fn terminal_notice_card(
        &mut self,
        ui: &mut egui::Ui,
        notice: &terminator_core::TerminalNotice,
        layout_key: u64,
    ) {
        let row = cached_variable_card(ui, layout_key, |ui| {
            ui.group(|ui| self.terminal_notice_body(ui, notice))
                .response
                .rect
        });
        #[cfg(feature = "test-support")]
        if let Some(rect) = row {
            diagnostics::record(
                ui.ctx(),
                &format!("terminal-row:{}", notice.session_id),
                rect,
            );
        }
        let _ = row;
    }

    fn terminal_notice_body(
        &mut self,
        ui: &mut egui::Ui,
        notice: &terminator_core::TerminalNotice,
    ) {
        ui.label(if notice.title.is_empty() {
            "Terminal"
        } else {
            &notice.title
        });
        ui.label(&notice.body);
        ui.horizontal(|ui| {
            let go = ui.button("Go to terminal");
            #[cfg(feature = "test-support")]
            diagnostics::record(
                ui.ctx(),
                &format!("terminal-go:{}", notice.session_id),
                go.rect,
            );
            if go.clicked() {
                self.go_session(&notice.session_id);
            }
            if self
                .state
                .capabilities
                .iter()
                .any(|c| c == TERMINAL_NOTICES_CAPABILITY)
                && ui.button("Dismiss").clicked()
            {
                self.send(Request::DismissTerminalNotice {
                    id: notice.id.clone(),
                });
            }
        });
    }

    /// Read-state inbox. Marking read never resolves, dismisses, or changes
    /// lifecycle; the selected row stays until selection or filter changes.
    pub(super) fn agents_unread(&mut self, ui: &mut egui::Ui) {
        let index = self.sidebar_index();
        let notices = self.cached_unread_groups();
        appearance::sidebar_scroll("agents-unread").show(ui, |ui| {
            if notices.is_empty() {
                ui.weak("No unread agent events");
                return;
            }
            for group in notices.iter() {
                let Some(notice) = group.notices.first() else {
                    continue;
                };
                let highlight = group.notices.iter().any(|n| {
                    self.detail.as_ref() == Some(&n.id)
                        || self.unread_selected.as_deref() == Some(n.id.as_str())
                });
                self.notice_group_card(
                    ui,
                    group,
                    NoticeCardInput {
                        session: index
                            .sessions
                            .get(&notice.session_id)
                            .map(std::convert::AsRef::as_ref),
                        highlight,
                        show_read: true,
                    },
                );
            }
        });
    }

    /// Verified live agents grouped by project/worktree, plus hook-only and
    /// unavailable-owner entries under Presence unverified.
    pub(super) fn agents_all_live(&mut self, ui: &mut egui::Ui) {
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

    fn agents_search_bar(&mut self, ui: &mut egui::Ui) {
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
    fn toggle_agents_collapsed(&mut self, key: &str, collapsed: bool) {
        if collapsed {
            self.preferences.agents_collapsed.remove(key);
        } else {
            self.preferences.agents_collapsed.insert(key.to_owned());
        }
    }

    fn live_project_group(
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

    fn live_unverified_group(&mut self, ui: &mut egui::Ui, unverified: &[std::sync::Arc<Session>]) {
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

    fn live_agent_row(&mut self, ui: &mut egui::Ui, session: &Session, target: &str) {
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

    fn agents_filter_menu(&mut self, ui: &mut egui::Ui) {
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

    fn agents_empty(&mut self, ui: &mut egui::Ui) {
        if self.hook_status.is_empty() {
            ui.weak("Checking agent hooks…");
            return;
        }
        if self.state.agents.is_empty() && !self.hook_status.values().any(|installed| *installed) {
            ui.weak("Agent hooks are not configured");
            if ui.small_button("Set up hooks").clicked() {
                self.open_settings();
                self.settings_section = SettingsSection::AgentHooks;
            }
            return;
        }
        ui.weak("No pending agent events");
    }

    #[cfg(any(test, target_os = "macos"))]
    pub(crate) fn waiting_notice_count(&self) -> usize {
        self.sidebar_index().waiting_count
    }

    /// Single source for the waiting/unread counts rendered as bells in
    /// the left agent bar and the IDE status-bar mirror.
    pub(crate) fn attention_counts(&self) -> (usize, usize) {
        let index = self.sidebar_index();
        (index.waiting_count, index.unread_count)
    }
}
