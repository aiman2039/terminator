//! Notification list, resource status, and the status badge.
use super::*;

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
                data.insert_temp(
                    egui::Id::unique("attention-state"),
                    (pending, popup.is_some()),
                )
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

    /// Agent counts as separate segments: waiting (always, as the bell),
    /// then unread, working and done when there are any. Each opens the
    /// Agents view that lists what it counts.
    pub(crate) fn notification_status_badge(&mut self, ui: &mut egui::Ui) {
        let (waiting, unread) = self.attention_counts();
        let (working, done) = self.activity_counts();
        for segment in status_segments([waiting, unread, working, done]) {
            let response = appearance::icon_label_button(appearance::IconLabelButtonInput {
                ui,
                icon: segment.kind.icon(),
                label: &segment.label,
                height: appearance::TOOLBAR_BUTTON,
            })
            .on_hover_text(segment.kind.tooltip());
            #[cfg(feature = "test-support")]
            diagnostics::record(ui.ctx(), segment.kind.target(), response.rect);
            if response.clicked() {
                self.open_agents_segment(segment.kind);
            }
        }
    }

    /// Show the Agents view behind a status-bar segment.
    fn open_agents_segment(&mut self, kind: AgentSegment) {
        self.open_agents_inbox();
        self.preferences.agents_tab = match kind {
            AgentSegment::Waiting => AgentsTab::NeedsAttention,
            AgentSegment::Unread => AgentsTab::Unread,
            AgentSegment::Working | AgentSegment::Done => AgentsTab::AllLive,
        };
        match kind {
            AgentSegment::Working => self.preferences.agents_state = AgentStateFilter::Working,
            AgentSegment::Done => self.preferences.agents_state = AgentStateFilter::Done,
            AgentSegment::Waiting | AgentSegment::Unread => {}
        }
    }
}

/// One count in the status-bar agent summary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AgentSegment {
    Waiting,
    Unread,
    Working,
    Done,
}

impl AgentSegment {
    const ALL: [Self; 4] = [Self::Waiting, Self::Unread, Self::Working, Self::Done];

    fn icon(self) -> &'static str {
        match self {
            Self::Waiting => "Bell",
            Self::Unread => "Circle",
            Self::Working => "LoaderCircle",
            Self::Done => "CircleCheck",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Unread => "unread",
            Self::Working => "working",
            Self::Done => "done",
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            Self::Waiting => "Agents waiting for you. Click to open Needs attention.",
            Self::Unread => "Unread agent events. Click to open Unread.",
            Self::Working => "Working agents. Click to list them in All live.",
            Self::Done => "Agents that are done. Click to list them in All live.",
        }
    }

    #[cfg(feature = "test-support")]
    fn target(self) -> &'static str {
        match self {
            Self::Waiting => "status-attention-bell",
            Self::Unread => "status-agents-unread",
            Self::Working => "status-agents-working",
            Self::Done => "status-agents-done",
        }
    }
}

pub(crate) struct StatusSegment {
    pub(crate) kind: AgentSegment,
    pub(crate) label: String,
}

/// Segments for `[waiting, unread, working, done]`. The waiting bell is
/// always there (with no count at zero) so the inbox stays one click away.
pub(crate) fn status_segments(counts: [usize; 4]) -> Vec<StatusSegment> {
    AgentSegment::ALL
        .into_iter()
        .zip(counts)
        .filter(|(kind, count)| *kind == AgentSegment::Waiting || *count > 0)
        .map(|(kind, count)| StatusSegment {
            kind,
            label: if count == 0 {
                String::new()
            } else {
                format!("{count} {}", kind.noun())
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_segments_show_the_bell_and_only_nonzero_counts() {
        let labels = |counts| {
            status_segments(counts)
                .into_iter()
                .map(|segment| (segment.kind, segment.label))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            labels([0, 0, 0, 0]),
            [(AgentSegment::Waiting, String::new())]
        );
        assert_eq!(
            labels([2, 0, 3, 1]),
            [
                (AgentSegment::Waiting, "2 waiting".to_string()),
                (AgentSegment::Working, "3 working".to_string()),
                (AgentSegment::Done, "1 done".to_string()),
            ]
        );
        assert_eq!(
            labels([0, 4, 0, 0]),
            [
                (AgentSegment::Waiting, String::new()),
                (AgentSegment::Unread, "4 unread".to_string()),
            ]
        );
    }
}
