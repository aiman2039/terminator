mod actions;
mod card;
mod card_actions;
mod peek;
mod restore;
#[cfg(test)]
mod restore_tests;
mod status;
mod triage;
#[cfg(test)]
mod triage_tests;

use super::super::agent_presence::{attention_status_icon, notice_preview as agent_notice_preview};
use super::explorer_rows::{ATTENTION_ACTION_SIZE, skip_clipped_row};
#[cfg(feature = "test-support")]
use crate::diagnostics;
#[cfg(any(test, target_os = "macos", target_os = "linux"))]
use crate::updater;
use crate::{
    App, appearance, icons,
    preferences::{AgentStateFilter, AgentsTab, SidebarTool},
    session_info,
};
pub(crate) use card::{attention_card, attention_card_in};
use eframe::egui::{self, Color32, RichText};
use terminator_core::appearance::AppearanceConfig;
use terminator_core::{AgentState, Notification, Session, now};
pub(crate) use triage::{SNOOZE_CHOICES, Triage, TriageUndo};

impl App {
    // Inline selection is not a modal and must not take terminal keyboard focus.
    // The session peek is a modal too: while it shows, no key reaches a PTY.
    pub(crate) fn notice_detail_modal_open(&self) -> bool {
        self.triage.peek.is_some() || self.notice_detail_open()
    }

    pub(crate) fn notice_detail_open(&self) -> bool {
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
        self.agents_tabs(ui);
        match self.preferences.agents_tab {
            AgentsTab::NeedsAttention => self.agents_needs_attention(ui),
            AgentsTab::AllLive => self.agents_all_live(ui),
            AgentsTab::Unread => self.agents_unread(ui),
        }
    }

    /// Inbox | Live | Unread as a text segmented control with counts. A
    /// narrow sidebar drops the counts, then shows the icons, so nothing
    /// is clipped.
    fn agents_tabs(&mut self, ui: &mut egui::Ui) {
        let index = self.sidebar_index();
        let inbox = index
            .pending_groups
            .len()
            .saturating_add(index.terminal_pending.len());
        let live = self.cached_live_rows();
        let live = live
            .groups
            .iter()
            .map(|(_, sessions)| sessions.len())
            .sum::<usize>()
            .saturating_add(live.unverified.len());
        let unread = self.cached_unread_groups().len();
        let tabs = [
            (
                AgentsTab::NeedsAttention,
                "Inbox",
                inbox,
                "CircleAlert",
                "Needs attention",
                "needs",
            ),
            (
                AgentsTab::AllLive,
                "Live",
                live,
                "CircleCheck",
                "All live",
                "live",
            ),
            (
                AgentsTab::Unread,
                "Unread",
                unread,
                "Bell",
                "Unread",
                "unread",
            ),
        ];
        let labels: Vec<String> = tabs
            .iter()
            .map(|(_, name, count, ..)| format!("{name} {count}"))
            .collect();
        let cells: Vec<appearance::SegmentCell<'_>> = tabs
            .iter()
            .zip(&labels)
            .map(
                |((_, short, _, icon, tip, _), label)| appearance::SegmentCell {
                    label,
                    short,
                    icon,
                    tip,
                },
            )
            .collect();
        let selected = tabs
            .iter()
            .position(|(tab, ..)| *tab == self.preferences.agents_tab)
            .unwrap_or(0);
        let responses = appearance::segmented(ui, &cells, selected);
        for ((tab, .., name), response) in tabs.iter().zip(&responses) {
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), &format!("agent-tab:{name}"), response.rect);
            #[cfg(not(feature = "test-support"))]
            let _ = name;
            if response.clicked() {
                self.preferences.agents_tab = *tab;
                self.unread_selected = None;
            }
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
    #[cfg(any(test, target_os = "macos", target_os = "linux"))]
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

/// The one colour of an agent state. Completed uses the text colour, not the
/// accent: the accent marks focus and unread results, and in Carbon it is
/// almost the red of `status_failed`.
pub(crate) fn state_color(state: AgentState, theme: &AppearanceConfig) -> Color32 {
    appearance::color(match state {
        AgentState::Running => &theme.status_running,
        AgentState::WaitingInput | AgentState::WaitingPermission => &theme.status_waiting,
        AgentState::Failed => &theme.status_failed,
        AgentState::Completed => &theme.text,
        _ => &theme.secondary,
    })
}

/// Row state counters: icon and count in the state color, in the order
/// waiting, failed, working, done. Only the waiting counter is kept when
/// a row is narrow.
pub(crate) fn state_counters(
    counts: crate::agent_presence::StateCounts,
    theme: &AppearanceConfig,
) -> Vec<appearance::StateCounter> {
    counts
        .ordered()
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .map(|(state, count)| appearance::StateCounter {
            icon: attention_status_icon(state),
            count,
            color: state_color(state, theme),
            keep: state == AgentState::WaitingPermission,
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum AttentionAction {
    None,
    Go,
    /// Go to the agent's terminal and show its working-tree changes.
    Review,
    /// The fixed 10-minute snooze of daemons without `notice-triage-v1`.
    Snooze,
    /// Snooze for this many seconds (`notice-triage-v1`).
    SnoozeFor(u64),
    Dismiss,
    /// Mark read without resolving, dismissing, or changing lifecycle.
    Read,
    /// Mark unread again (`notice-triage-v1`); read state only.
    Unread,
    /// Keep the session at the top of the inbox and live list (GUI only).
    Pin,
    /// Show a read-only snapshot of the session's screen.
    Peek,
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
    /// The session is pinned to the top.
    pub pinned: bool,
    /// The daemon takes the triage verbs (`notice-triage-v1`).
    pub triage: bool,
    /// Model and mode chips the agent reported (`agent-usage-v1`).
    pub usage: &'a [String],
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

/// "2 working · 1 done" for live agents. Empty when there are none.
pub(super) fn activity_label(working: usize, done: usize) -> String {
    match (working, done) {
        (0, 0) => String::new(),
        (working, 0) => format!("{working} working"),
        (0, done) => format!("{done} done"),
        (working, done) => format!("{working} working · {done} done"),
    }
}

/// Joins the attention badge and the activity label with " · ".
pub(super) fn join_labels(first: &str, second: &str) -> String {
    match (first.is_empty(), second.is_empty()) {
        (true, _) => second.to_owned(),
        (_, true) => first.to_owned(),
        _ => format!("{first} · {second}"),
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
