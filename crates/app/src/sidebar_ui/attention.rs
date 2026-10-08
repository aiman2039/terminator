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
use eframe::egui::{self, Color32, RichText};
use terminator_core::appearance::AppearanceConfig;
use terminator_core::{AgentState, Notification, Request, Session, now};

impl App {
    pub(crate) fn notifications(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let pending = self
                .state
                .notifications
                .iter()
                .filter(|n| notice_pending(n, now()))
                .count()
                .saturating_add(
                    self.state
                        .terminal_notices
                        .iter()
                        .filter(|n| !n.dismissed)
                        .count(),
                );
            let label = if pending == 0 {
                String::new()
            } else {
                pending.to_string()
            };
            let button = egui::Button::image_and_text(
                egui::Image::new(icons::source("Bell")).fit_to_exact_size(egui::vec2(16.0, 16.0)),
                label,
            )
            .frame(false);
            let response = ui
                .add(button)
                .on_hover_text(format!("{pending} pending notifications"));
            if response.clicked() {
                self.preferences.agents_tab = AgentsTab::NeedsAttention;
            }
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), "attention-bell", response.rect);
            let popup = egui::Popup::from_toggle_button_response(&response).show(|ui| {
                ui.set_width(300.0);
                ui.set_max_height(460.0);
                ui.push_id("attention-popup", |ui| self.agents_view(ui));
            });
            #[cfg(feature = "test-support")]
            ui.ctx().data_mut(|data| {
                data.insert_temp(egui::Id::new("attention-state"), (pending, popup.is_some()))
            });
            let _ = popup;
        });
    }

    /// IDE status-bar mirror of the left agent bell: always visible in IDE
    /// mode so waiting/unread counts survive collapsed sidebars. Clicking
    /// reveals the Agents inbox in the right sidebar.
    /// Right end of the bottom status strip: app CPU and memory across the
    /// GUI, daemon, and hook processes. Pure paint over the latest background
    /// sample (one every few seconds), so it schedules no repaints of its
    /// own — the strip already repaints on the one-second heartbeat.
    /// Clicking opens the Info tool, which breaks the same sample down per
    /// session and system.
    pub(crate) fn app_resource_status(&mut self, ui: &mut egui::Ui) {
        let Some(sample) = &self.resources else {
            return;
        };
        let total = sample.app.total();
        // The GUI process itself must always match; zero means the name
        // heuristic failed on this platform, and a stuck "0 MB" would be
        // worse than no readout.
        if total.processes == 0 {
            return;
        }
        let component = |name: &str, stats: crate::resource_sample::ComponentStats| {
            format!(
                "{} ({}): {} · {}",
                name,
                stats.processes,
                session_info::format_percent(stats.cpu),
                session_info::format_bytes(stats.memory)
            )
        };
        let label = format!(
            "{} · {}",
            session_info::format_percent(total.cpu),
            session_info::format_bytes(total.memory)
        );
        let response = ui
            .add(egui::Label::new(RichText::new(label).small().weak()).sense(egui::Sense::click()))
            .on_hover_text(format!(
                "Terminator app processes ({})\n{}\n{}\n{}\n\nClick to open Info → Resources.",
                total.processes,
                component("GUI", sample.app.gui),
                component("Daemon", sample.app.daemon),
                component("Hooks", sample.app.hooks),
            ))
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "status-resources", response.rect);
        if response.clicked() {
            self.preferences.tool = SidebarTool::Info;
            self.preferences.visible = true;
        }
    }

    pub(crate) fn notification_status_badge(&mut self, ui: &mut egui::Ui) {
        let (waiting, unread) = self.attention_counts();
        let label = attention_badge_label(waiting, unread);
        let response = ui
            .add(
                egui::Button::image_and_text(
                    egui::Image::new(icons::source("Bell"))
                        .fit_to_exact_size(egui::vec2(14.0, 14.0)),
                    label,
                )
                .frame(false),
            )
            .on_hover_text("Pending agent notifications. Click to open the Agents inbox.");
        #[cfg(feature = "test-support")]
        diagnostics::record(ui.ctx(), "status-attention-bell", response.rect);
        if response.clicked() {
            self.preferences.tool = SidebarTool::Agents;
            self.preferences.visible = true;
            self.preferences.agents_tab = AgentsTab::NeedsAttention;
        }
    }

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
                        let response = appearance::selectable_icon(ui, icon, label, selected);
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
            cwd_full: session.map(|session| session.cwd.display().to_string()),
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
        session_info::show(ui, &model, &mut toggles, &self.theme);
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

fn attention_status_glyph(
    ui: &mut egui::Ui,
    state: AgentState,
    theme: &AppearanceConfig,
) -> egui::Response {
    let tint = state_color(state, theme);
    // An exact button, not `Button::image`: the button minimum follows
    // `interact_size` and inflated this glyph past the 22px row, pushing its
    // center below the label and action icons.
    let response = appearance::toolbar_button(ui).on_hover_ui(|ui| {
        ui.set_max_width(240.0);
        ui.colored_label(tint, state.label());
        ui.label(attention_status_detail(state));
    });
    appearance::paint_centered_icon(ui, response.rect, attention_status_icon(state), 14.0, tint);
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, state.label()));
    response
}

fn attention_title(
    ui: &mut egui::Ui,
    notice: &Notification,
    group_extra: &[Notification],
    session: Option<&Session>,
    theme: &AppearanceConfig,
    brand: Option<(&'static str, &str)>,
    show_read: bool,
) -> egui::Response {
    let spacing = ui.spacing().item_spacing.x;
    let brand_response = brand.map(|(icon, label)| {
        let (rect, response) = ui.allocate_exact_size(
            egui::vec2(18.0, ATTENTION_ACTION_SIZE),
            egui::Sense::hover(),
        );
        appearance::paint_centered_icon(ui, rect, icon, 14.0, Color32::WHITE);
        response.on_hover_text(label)
    });
    let icon = attention_status_glyph(ui, notice.state, theme);
    #[cfg(feature = "test-support")]
    diagnostics::record(
        ui.ctx(),
        &format!("agent-status:{}", notice.session_id),
        icon.rect,
    );
    let session_label = session.map_or("Unknown session", |session| session.label.as_str());
    let text = if group_extra.is_empty() {
        session_label.to_string()
    } else {
        format!(
            "{session_label} · {} events",
            group_extra.len().saturating_add(1)
        )
    };
    // Measure after the leading icons so a brand glyph cannot push the
    // actions onto a second row.
    let label_width = attention_label_width(ui.available_width(), spacing, show_read);
    let label = ui
        .add_sized(
            [label_width, ATTENTION_ACTION_SIZE],
            egui::Label::new(RichText::new(text).size(12.0))
                .halign(egui::Align::LEFT)
                .truncate()
                .sense(egui::Sense::click()),
        )
        .on_hover_ui(|ui| {
            ui.set_max_width(360.0);
            if let Some(session) = session {
                ui.weak(session.cwd.display().to_string());
            }
            ui.label(notice_preview(&notice.summary));
            for extra in group_extra.iter().take(8) {
                ui.label(format!(
                    "{}: {}",
                    extra.state.label(),
                    notice_preview(&extra.summary)
                ));
            }
            if group_extra.len() > 8 {
                ui.weak(format!("…and {} more", group_extra.len().saturating_sub(8)));
            }
        });
    match brand_response {
        Some(brand) => brand.union(icon).union(label),
        None => icon.union(label),
    }
}

fn attention_actions(ui: &mut egui::Ui, session_id: &str, show_read: bool) -> AttentionAction {
    let count = if show_read {
        ATTENTION_ACTION_READ_COUNT
    } else {
        ATTENTION_ACTION_COUNT
    };
    let width = ATTENTION_ACTION_SIZE * count;
    ui.push_id(session_id, |ui| {
        ui.horizontal(|ui| {
            ui.set_max_width(width);
            ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
            ui.spacing_mut().interact_size =
                egui::vec2(ATTENTION_ACTION_SIZE, ATTENTION_ACTION_SIZE);
            let mut action = AttentionAction::None;
            let mut buttons = vec![
                ("ArrowRight", "Go to context", "go", AttentionAction::Go),
                (
                    "Moon",
                    "Snooze 10 minutes",
                    "snooze",
                    AttentionAction::Snooze,
                ),
                ("X", "Dismiss", "dismiss", AttentionAction::Dismiss),
            ];
            if show_read {
                buttons.insert(
                    1,
                    (
                        "CircleCheck",
                        "Mark read (keeps agent state)",
                        "read",
                        AttentionAction::Read,
                    ),
                );
            }
            for (icon, tip, name, next) in buttons {
                let response = appearance::sidebar_action(ui, icon, tip);
                #[cfg(feature = "test-support")]
                diagnostics::record(
                    ui.ctx(),
                    &format!("agent-{name}:{session_id}"),
                    response.rect,
                );
                #[cfg(not(feature = "test-support"))]
                let _ = name;
                if response.clicked() {
                    action = next;
                }
            }
            action
        })
        .inner
    })
    .inner
}

pub(crate) fn attention_card(ui: &mut egui::Ui, input: AttentionCard<'_>) -> AttentionAction {
    ui.push_id(("attention-card", &input.notice.session_id), |ui| {
        attention_card_contents(ui, input)
    })
    .inner
}

fn attention_card_contents(ui: &mut egui::Ui, input: AttentionCard<'_>) -> AttentionAction {
    let AttentionCard {
        theme,
        notice,
        session,
        selected,
        highlight,
        brand_icon,
        brand_label,
        show_read,
        group_extra,
    } = input;
    let brand = match (brand_icon, brand_label) {
        (Some(icon), Some(label)) => Some((icon, label)),
        (Some(icon), None) => Some((icon, "Agent")),
        _ => None,
    };
    let stroke = if highlight {
        egui::Stroke::new(1.0, appearance::color(&theme.accent))
    } else if selected {
        egui::Stroke::new(1.0, appearance::color(&theme.selection))
    } else {
        egui::Stroke::new(theme.border_width, appearance::color(&theme.border))
    };
    let frame = egui::Frame::group(ui.style())
        .stroke(stroke)
        .corner_radius(6)
        .fill(appearance::color(&theme.window))
        .inner_margin(egui::Margin::symmetric(6, 2));
    let height = ATTENTION_ACTION_SIZE + frame.total_margin().sum().y;
    if !notice.resolved && skip_clipped_row(ui, height) {
        return AttentionAction::None;
    }
    let inner = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 2.0);
        ui.spacing_mut().interact_size.y = ATTENTION_ACTION_SIZE;
        let action = ui
            .horizontal(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 0.0);
                let header =
                    attention_title(ui, notice, group_extra, session, theme, brand, show_read);
                #[cfg(feature = "test-support")]
                diagnostics::record(
                    ui.ctx(),
                    &format!("agent-row:{}", notice.session_id),
                    header.rect,
                );
                let mut action = if header.clicked() {
                    AttentionAction::Go
                } else {
                    AttentionAction::None
                };
                let buttons = attention_actions(ui, &notice.session_id, show_read);
                if buttons != AttentionAction::None {
                    action = buttons;
                }
                action
            })
            .inner;
        if notice.resolved {
            ui.weak("This event has resolved.");
        }
        action
    });
    // Header and action buttons own clicks. A later frame-wide click target
    // sits on top of those buttons in egui and would steal Dismiss/Snooze as Go.
    inner.inner
}
