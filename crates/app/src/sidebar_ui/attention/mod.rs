mod card;
mod status;

use super::super::agent_presence::{attention_status_icon, notice_preview as agent_notice_preview};
use super::explorer_rows::{
    ATTENTION_ACTION_COUNT, ATTENTION_ACTION_READ_COUNT, ATTENTION_ACTION_SIZE, skip_clipped_row,
};
#[cfg(feature = "test-support")]
use crate::diagnostics;
#[cfg(any(test, target_os = "macos"))]
use crate::updater;
use crate::{
    App, appearance, icons,
    preferences::{AgentsTab, SidebarTool},
    session_info,
};
pub(crate) use card::attention_card;
use eframe::egui::{self, Color32, RichText};
use terminator_core::appearance::AppearanceConfig;
use terminator_core::{AgentState, Notification, Request, Session, now};

impl App {
    // Inline selection is not a modal and must not take terminal keyboard focus.
    pub(crate) fn notice_detail_modal_open(&self) -> bool {
        self.detail.as_ref().is_some_and(|id| {
            self.state
                .notifications
                .iter()
                .find(|notice| &notice.id == id)
                .is_some_and(|notice| {
                    !self.agents_inbox_open()
                        || notice.dismissed
                        || notice.resolved
                        || notice.snoozed_until > now()
                        || !self.notice_in_scope(notice)
                })
        })
    }

    /// Brand for a historical notice. Terminal chrome hides the hook brand
    /// once presence verifies the agent exited, but inbox cards still name
    /// the agent that raised the event.
    pub(crate) fn notice_history_brand(
        &self,
        notice: &Notification,
    ) -> Option<(&'static str, String)> {
        self.sidebar_index()
            .history_brands
            .get(&(notice.session_id.clone(), notice.invocation_id.clone()))
            .cloned()
    }

    pub(crate) fn agents_view(&mut self, ui: &mut egui::Ui) {
        ui.heading("Agents");
        // The tab bar scrolls instead of widening narrow sidebars.
        egui::ScrollArea::horizontal()
            .id_salt("agents-tabs")
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (tab, icon, label, name) in [
                        (
                            AgentsTab::NeedsAttention,
                            "CircleAlert",
                            "Needs attention",
                            "needs",
                        ),
                        (AgentsTab::AllLive, "CircleCheck", "All live", "live"),
                        (AgentsTab::Unread, "Bell", "Unread", "unread"),
                    ] {
                        let selected = self.preferences.agents_tab == tab;
                        let response =
                            appearance::selectable_icon(appearance::SelectableIconInput {
                                ui,
                                icon,
                                tip: label,
                                selected,
                            });
                        #[cfg(feature = "test-support")]
                        diagnostics::record(ui.ctx(), &format!("agent-tab:{name}"), response.rect);
                        #[cfg(not(feature = "test-support"))]
                        let _ = name;
                        if response.clicked() {
                            self.preferences.agents_tab = tab;
                            self.unread_selected = None;
                        }
                    }
                });
            });
        match self.preferences.agents_tab {
            AgentsTab::NeedsAttention => self.agents_needs_attention(ui),
            AgentsTab::AllLive => self.agents_all_live(ui),
            AgentsTab::Unread => self.agents_unread(ui),
        }
    }

    #[cfg(test)]
    pub(crate) fn unread_notices(&self) -> Vec<Notification> {
        let index = self.sidebar_index();
        let mut notices: Vec<_> = index
            .pending
            .iter()
            .filter(|n| !n.read || self.unread_selected.as_deref() == Some(n.id.as_str()))
            .cloned()
            .collect();
        notices.sort_by_key(|n| std::cmp::Reverse(n.created));
        notices
    }

    /// Pending agent notices as menu-bar items, in inbox order (waiting
    /// first). Titles are single-line and capped so the native menu stays
    /// readable. Same list the Agents inbox renders.
    #[cfg(any(test, target_os = "macos"))]
    pub(crate) fn status_menu_items(&self) -> Vec<updater::StatusMenuItem> {
        const MAX_ITEMS: usize = 12;
        const MAX_TITLE: usize = 90;
        let index = self.sidebar_index();
        index
            .pending
            .iter()
            .take(MAX_ITEMS)
            .map(|notice| {
                let session = index
                    .sessions
                    .get(&notice.session_id)
                    .map(|session| session.label.as_str())
                    .unwrap_or("Terminal");
                let title = format!("{} — {}: {}", session, notice.state.label(), notice.summary);
                let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
                let title = if title.chars().count() > MAX_TITLE {
                    format!("{}…", title.chars().take(MAX_TITLE - 1).collect::<String>())
                } else {
                    title
                };
                updater::StatusMenuItem {
                    id: notice.id.clone(),
                    title,
                }
            })
            .collect()
    }

    /// A notice is in scope while its session exists. The inbox always
    /// spans all projects; there is no per-project filter.
    fn notice_in_scope(&self, notice: &Notification) -> bool {
        self.sidebar_index()
            .sessions
            .contains_key(&notice.session_id)
    }

    /// One row acts as one unit: navigation needs a single target, while
    /// read/snooze/dismiss apply to every notice folded into the row.
    pub(super) fn apply_group_action(&mut self, group: &NoticeGroup, action: AttentionAction) {
        match action {
            AttentionAction::Go => {
                if let Some(first) = group.notices.first() {
                    self.apply_notice_action(first.id.clone(), action);
                }
            }
            _ => {
                for notice in &group.notices {
                    self.apply_notice_action(notice.id.clone(), action);
                }
            }
        }
    }

    pub(crate) fn apply_notice_action(&mut self, id: String, action: AttentionAction) {
        let mut presentation_changed = false;
        match action {
            AttentionAction::None => {}
            AttentionAction::Go => {
                if let Some(session) = self
                    .state
                    .notifications
                    .iter()
                    .find(|notice| notice.id == id)
                    .map(|notice| notice.session_id.clone())
                {
                    self.go_session(&session);
                }
                self.unread_selected = Some(id.clone());
                self.detail = None;
            }
            AttentionAction::Read => {
                self.send(Request::Notice {
                    id: id.clone(),
                    action: "read".into(),
                });
                if let Some(notice) = self
                    .state
                    .notifications
                    .iter_mut()
                    .find(|notice| notice.id == id)
                {
                    // Read state only: never resolve, dismiss, or touch lifecycle.
                    notice.read = true;
                    presentation_changed = true;
                }
                self.unread_selected = Some(id);
            }
            AttentionAction::Snooze => {
                self.send(Request::Notice {
                    id: id.clone(),
                    action: "snooze".into(),
                });
                if let Some(notice) = self
                    .state
                    .notifications
                    .iter_mut()
                    .find(|notice| notice.id == id)
                {
                    notice.snoozed_until = now().saturating_add(600);
                    presentation_changed = true;
                }
                if self.detail.as_ref() == Some(&id) {
                    self.detail = None;
                }
            }
            AttentionAction::Dismiss => {
                self.send(Request::Notice {
                    id: id.clone(),
                    action: "dismiss".into(),
                });
                if let Some(notice) = self
                    .state
                    .notifications
                    .iter_mut()
                    .find(|notice| notice.id == id)
                {
                    notice.dismissed = true;
                    presentation_changed = true;
                }
                if self.unread_selected.as_deref() == Some(id.as_str()) {
                    self.unread_selected = None;
                }
                if self.detail.as_ref() == Some(&id) {
                    self.detail = None;
                }
            }
        }
        if presentation_changed {
            self.reconcile_presentations();
        }
    }

    pub(super) fn info_panel(&mut self, ui: &mut egui::Ui) {
        let session = self.context_session();
        let live = session.is_some_and(|session| session.lifecycle.live() && session.pid.is_some());
        let pid = session.and_then(|session| session.pid);
        let created = session.map(|session| session.created).unwrap_or(0);
        let sample = self
            .resources
            .as_ref()
            .filter(|sample| sample.pid == pid && sample.started == created);
        let input = session_info::Input {
            label: session.map(|session| session.label.clone()),
            cwd: session.map(|session| crate::services::compact_path(&session.cwd)),
            cwd_full: session.map(crate::remote_ui::location_label),
            branch: session.and_then(|session| self.branch_at(&session.cwd)),
            started: session_info::started_label(created, now(), live),
            unconfirmed: session.is_some_and(|session| !session.cwd_confirmed),
            session_cpu: sample
                .and_then(|sample| sample.session)
                .map(|stats| stats.cpu),
            session_memory: sample
                .and_then(|sample| sample.session)
                .map(|stats| stats.memory),
            system: self.resources.as_ref().map(|sample| sample.system.clone()),
        };
        let mut toggles = session_info::Toggles {
            process_open: self.preferences.info_process_open,
            resources_open: self.preferences.info_resources_open,
            show_system: self.preferences.info_show_system,
        };
        let model = session_info::model(&input);
        session_info::show(session_info::ShowInput {
            ui,
            model: &model,
            toggles: &mut toggles,
            theme: &self.theme,
        });
        self.preferences.info_process_open = toggles.process_open;
        self.preferences.info_resources_open = toggles.resources_open;
        self.preferences.info_show_system = toggles.show_system;
    }
}

pub(crate) fn state_color(state: AgentState, theme: &AppearanceConfig) -> Color32 {
    appearance::color(match state {
        AgentState::Running => &theme.status_running,
        AgentState::WaitingInput | AgentState::WaitingPermission => &theme.status_waiting,
        AgentState::Failed => &theme.status_failed,
        AgentState::Completed => &theme.accent,
        _ => &theme.secondary,
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttentionAction {
    None,
    Go,
    Snooze,
    Dismiss,
    /// Mark read without resolving, dismissing, or changing lifecycle.
    Read,
}

pub(crate) struct AttentionCard<'a> {
    pub theme: &'a AppearanceConfig,
    pub notice: &'a Notification,
    pub session: Option<&'a Session>,
    pub selected: bool,
    pub highlight: bool,
    /// Stable agent brand beside the status glyph, from the shared model.
    pub brand_icon: Option<&'static str>,
    pub brand_label: Option<&'a str>,
    /// Show the mark-read button (Unread view only).
    pub show_read: bool,
    /// Further notices from the same agent run folded into this row.
    pub group_extra: &'a [Notification],
}

pub(crate) fn notice_waiting(notice: &Notification) -> bool {
    !notice.resolved
        && matches!(
            notice.state,
            AgentState::WaitingInput | AgentState::WaitingPermission
        )
}

pub(crate) fn notice_rank(state: AgentState) -> u8 {
    match state {
        AgentState::WaitingInput | AgentState::WaitingPermission => 0,
        AgentState::Failed => 1,
        AgentState::Completed => 2,
        _ => 3,
    }
}

/// One inbox row: every pending notice from a single session. Focus
/// dismissal already clears the whole session at once, so the row acts as
/// one unit while the inbox keeps each underlying notice.
pub(crate) struct NoticeGroup {
    pub(super) notices: Vec<Notification>,
}

/// Groups by session, preserving input order; the first notice of each
/// group is its representative. Callers pass urgency/newest-sorted input.
pub(crate) fn group_notices(notices: Vec<Notification>) -> Vec<NoticeGroup> {
    let mut groups: Vec<NoticeGroup> = Vec::new();
    for notice in notices {
        if let Some(group) = groups.iter_mut().find(|group| {
            group
                .notices
                .first()
                .is_some_and(|first| first.session_id == notice.session_id)
        }) {
            group.notices.push(notice);
        } else {
            groups.push(NoticeGroup {
                notices: vec![notice],
            });
        }
    }
    groups
}

pub(crate) fn notice_pending(notice: &Notification, timestamp: u64) -> bool {
    !notice.dismissed && !notice.resolved && notice.snoozed_until <= timestamp
}

pub(super) fn attention_badge_label(waiting: usize, unread: usize) -> String {
    match (waiting, unread) {
        (0, 0) => String::new(),
        (0, unread) => format!("{unread} unread"),
        (waiting, 0) => format!("{waiting} waiting"),
        (waiting, unread) => format!("{waiting} waiting · {unread} unread"),
    }
}

pub(super) fn notice_preview(markdown: &str) -> String {
    agent_notice_preview(markdown)
}

pub(super) fn attention_status_detail(state: AgentState) -> &'static str {
    match state {
        AgentState::WaitingInput => "Reply in the terminal to continue.",
        AgentState::WaitingPermission => "Approve or deny the agent's request.",
        AgentState::Completed => "This turn finished.",
        AgentState::Failed => "The agent reported an error.",
        AgentState::Running => "The agent is working.",
        AgentState::Stopped => "The agent was stopped.",
        AgentState::Unknown => "No status has been reported yet.",
    }
}

/// Width left for the session label after the status glyph (and optional
/// brand) are already placed. `remaining` is that leftover row width.
pub(super) fn attention_label_width(remaining: f32, spacing: f32, show_read: bool) -> f32 {
    let actions = if show_read {
        ATTENTION_ACTION_READ_COUNT
    } else {
        ATTENTION_ACTION_COUNT
    };
    (remaining - spacing - ATTENTION_ACTION_SIZE * actions).max(0.0)
}
