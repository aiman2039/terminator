mod live;

use super::attention::{
    AttentionCard, NoticeGroup, attention_badge_label, attention_card, state_color,
};
use super::explorer_rows::{
    PaintHeaderCountInput, cached_variable_card, paint_header_count, skip_clipped_row,
};
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

    /// The left sidebar is visible and paints the Agents inbox. A left tool
    /// takes precedence over `left_agents`, so the flag alone is not enough.
    pub(crate) fn left_agents_shown(&self) -> bool {
        self.preferences.left_visible
            && match self.preferences.left_tool {
                Some(tool) => tool == SidebarTool::Agents,
                None => self.preferences.left_agents,
            }
    }

    pub(crate) fn toggle_left_agents(&mut self) {
        if self.left_agents_shown() {
            self.preferences.left_tool = None;
            self.preferences.left_agents = false;
            return;
        }
        if !self.preferences.left_agents {
            self.preferences.agents_tab = AgentsTab::NeedsAttention;
        }
        self.preferences.left_tool = None;
        self.preferences.left_agents = true;
        self.preferences.left_visible = true;
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
            selected: self.left_agents_shown(),
            tint: appearance::icon_color(),
        });
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Agents"));
        let count = waiting.saturating_add(unread);
        if count > 0 {
            paint_header_count(PaintHeaderCountInput {
                ui,
                rect: response.rect,
                count,
                color: appearance::color(if waiting > 0 {
                    &self.theme.status_waiting
                } else {
                    &self.theme.text
                }),
            });
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
        let mark = if self.left_agents_shown() { "✓" } else { "" };
        let clicked = appearance::menu_item(appearance::MenuItemInput {
            ui,
            label: &label,
            icon: "Bell",
            shortcut: mark,
        })
        .clicked();
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
