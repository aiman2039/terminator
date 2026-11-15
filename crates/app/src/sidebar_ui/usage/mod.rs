//! Usage tool: each agent account's quota windows, read only from what the
//! agent CLI reports. An agent that reports nothing shows `n/a`.
use crate::quota_refresh::window_text;
use crate::*;
use terminator_core::agents::{display_name, quota::Quota};

/// Agents the Usage tool lists.
const AGENTS: [&str; 2] = ["claude", "codex"];

/// GUI state of the Usage tool. The worker reads while it shows or while a
/// quota alert channel is on.
#[derive(Default)]
pub(crate) struct UsageTool {
    drawn: bool,
    read: Option<Vec<Quota>>,
}

/// Quota alert channels, each an explicit opt-in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct QuotaAlerts {
    pub inbox: bool,
    pub os: bool,
    pub ntfy: bool,
}

impl QuotaAlerts {
    fn any(self) -> bool {
        self.inbox || self.os || self.ntfy
    }
}

/// A quota change worth an alert: a window reached its limit, or a window
/// that had reached it reset. Only changes between two reads count, so
/// opening the app never alerts.
pub(crate) fn quota_alerts(before: &[Quota], after: &[Quota], now: u64) -> Vec<(String, String)> {
    let mut alerts = Vec::new();
    for quota in after {
        let Some(old) = before.iter().find(|q| q.agent == quota.agent) else {
            continue;
        };
        let name = display_name(&quota.agent);
        for window in &quota.windows {
            let Some(was) = old.windows.iter().find(|w| w.label == window.label) else {
                continue;
            };
            let full = |w: &terminator_core::agents::quota::QuotaWindow| {
                w.used_percent >= 100.0 && w.resets_at.is_none_or(|at| at > now)
            };
            if full(window) && !full(was) {
                alerts.push((
                    quota.agent.clone(),
                    format!("{name}: {} limit reached", window.label),
                ));
            } else if full(was) && !full(window) {
                alerts.push((
                    quota.agent.clone(),
                    format!("{name}: {} window reset", window.label),
                ));
            }
        }
    }
    alerts
}

impl App {
    pub(crate) fn sync_quota_request(&mut self) {
        let drawn = std::mem::take(&mut self.quota.drawn);
        let polling = drawn || self.preferences.quota_alerts.any();
        if *self.quota_jobs.borrow() != polling {
            let _ = self.quota_jobs.send(polling);
        }
    }

    pub(crate) fn handle_quota(&mut self, read: Vec<Quota>) {
        if let Some(before) = &self.quota.read {
            for (agent, title) in quota_alerts(before, &read, now()) {
                self.send_quota_alert(&agent, title);
            }
        }
        self.quota.read = Some(read);
    }

    /// Raises one alert on the opted-in channels, in the terminal of the
    /// agent's newest live run. A terminal of an older daemon, or no live
    /// terminal, gets a status note only.
    fn send_quota_alert(&mut self, agent: &str, title: String) {
        let channels = self.preferences.quota_alerts;
        if !channels.any() {
            return;
        }
        let session = self
            .state
            .agents
            .iter()
            .filter(|a| a.kind == agent)
            .filter(|a| {
                self.state
                    .sessions
                    .iter()
                    .any(|s| s.id == a.session_id && s.lifecycle.live())
            })
            .max_by_key(|a| a.updated)
            .map(|a| a.session_id.clone());
        let session = session
            .filter(|sid| self.terminal_capability(sid, terminator_core::AGENT_QUOTA_CAPABILITY));
        match session {
            Some(session) => self.send(Request::QuotaNotify {
                session,
                title,
                inbox: channels.inbox,
                os: channels.os,
                ntfy: channels.ntfy,
            }),
            None => self.info = Some(title),
        }
    }

    pub(crate) fn usage_panel(&mut self, ui: &mut egui::Ui) {
        self.quota.drawn = true;
        ui.strong("Agent usage");
        ui.add_space(4.0);
        let Some(read) = &self.quota.read else {
            ui.weak("Reading agent usage…");
            return;
        };
        let now = now();
        appearance::sidebar_scroll("usage-tool").show(ui, |ui| {
            for key in AGENTS {
                let quota = read.iter().find(|q| q.agent == key);
                usage_agent(ui, display_name(key), quota, now);
                ui.add_space(8.0);
            }
            let alerts = &mut self.preferences.quota_alerts;
            quota_alert_choices(ui, alerts, &self.state.capabilities);
        });
    }
}

fn quota_alert_choices(ui: &mut egui::Ui, alerts: &mut QuotaAlerts, capabilities: &[String]) {
    ui.label("Alert when a limit is reached or resets:");
    ui.checkbox(&mut alerts.inbox, "Agents inbox");
    ui.checkbox(&mut alerts.os, "OS notification");
    ui.checkbox(&mut alerts.ntfy, "ntfy push (agent and window only)");
    if !capabilities
        .iter()
        .any(|c| c == terminator_core::AGENT_QUOTA_CAPABILITY)
    {
        ui.weak("Restart the session service to send alerts.");
    }
}

fn usage_agent(ui: &mut egui::Ui, name: &str, quota: Option<&Quota>, now: u64) {
    let Some(quota) = quota else {
        ui.horizontal(|ui| {
            ui.label(name);
            ui.weak("n/a");
        })
        .response
        .on_hover_text("This agent has not reported its usage.");
        return;
    };
    ui.horizontal(|ui| {
        ui.label(name);
        if let Some(plan) = &quota.plan {
            ui.weak(plan);
        }
    });
    for window in &quota.windows {
        #[expect(clippy::cast_possible_truncation, reason = "a percentage fits an f32")]
        let fraction = (window.used_percent / 100.0) as f32;
        ui.add(egui::ProgressBar::new(fraction).text(window_text(window, now)));
    }
    if quota.observed_at > 0 {
        let minutes = now.saturating_sub(quota.observed_at) / 60;
        ui.weak(format!("Reported {minutes} min ago"));
    }
}

#[cfg(test)]
mod tests;
