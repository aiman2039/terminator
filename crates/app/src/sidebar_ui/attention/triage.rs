//! Inbox triage state: the session peek, the last undoable dismiss or
//! snooze, session pins, and the snooze durations.
use crate::App;
use terminator_core::{Request, agent_workflow, now};

/// GUI-only triage state. Nothing here is sent to a PTY.
#[derive(Default)]
pub(crate) struct Triage {
    /// Session whose read-only screen snapshot shows in the peek popup.
    pub peek: Option<String>,
    /// The last dismiss or snooze, for Undo.
    pub undo: Option<TriageUndo>,
    /// Interrupted shells waiting for their saved output before a restore.
    pub restores: Vec<super::restore::PendingRestore>,
}

/// One undoable inbox action and the notices it changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TriageUndo {
    pub label: String,
    pub ids: Vec<String>,
}

/// Seconds to snooze, as a function of the current Unix time.
type SnoozeSecs = fn(u64) -> u64;

/// Snooze menu entries: label and a function of the current time.
pub(crate) const SNOOZE_CHOICES: [(&str, SnoozeSecs); 3] = [
    ("Snooze 15 minutes", |_| 15 * 60),
    ("Snooze 1 hour", |_| 60 * 60),
    ("Snooze until tomorrow", seconds_until_tomorrow),
];

/// Seconds from `now` until 09:00 local time on the next day; one day when
/// the local time is not known.
pub(crate) fn seconds_until_tomorrow(now: u64) -> u64 {
    const DAY: u64 = 24 * 60 * 60;
    terminator_sys::seconds_until_tomorrow_at(now, 9)
        .filter(|secs| (1..=agent_workflow::MAX_SNOOZE_SECS).contains(secs))
        .unwrap_or(DAY)
}

impl App {
    /// The running daemon takes Mark unread, timed snooze and restore.
    pub(crate) fn triage_available(&self) -> bool {
        self.state
            .capabilities
            .iter()
            .any(|c| c == agent_workflow::TRIAGE_CAPABILITY)
    }

    pub(crate) fn session_pinned(&self, session: &str) -> bool {
        self.preferences.pinned_agent_sessions.contains(session)
    }

    /// Pins or unpins a session in the Agents views (GUI preference only).
    pub(crate) fn toggle_session_pin(&mut self, session: &str) {
        let pins = &mut self.preferences.pinned_agent_sessions;
        if !pins.remove(session) {
            pins.insert(session.to_owned());
        }
        // Forget pins of sessions the daemon no longer knows.
        let known: std::collections::HashSet<&str> =
            self.state.sessions.iter().map(|s| s.id.as_str()).collect();
        pins.retain(|id| known.contains(id.as_str()));
    }

    /// Notices are pending again: dismissal and snooze of the last action
    /// are undone. Read state and lifecycle are left as they are.
    pub(crate) fn undo_triage(&mut self) {
        let Some(undo) = self.triage.undo.take() else {
            return;
        };
        if !self.triage_available() {
            return;
        }
        for id in &undo.ids {
            self.send(Request::Notice {
                id: id.clone(),
                action: "restore".into(),
            });
            if let Some(notice) = self.state.notifications.iter_mut().find(|n| &n.id == id) {
                notice.dismissed = false;
                notice.snoozed_until = 0;
            }
        }
        self.reconcile_presentations();
    }

    /// Snoozes one notice locally until the next snapshot confirms it.
    pub(super) fn snooze_notice(&mut self, id: &str, secs: Option<u64>) {
        let (action, secs) = match secs {
            Some(secs) if self.triage_available() => (agent_workflow::snooze_for(secs), secs),
            _ => ("snooze".to_owned(), agent_workflow::DEFAULT_SNOOZE_SECS),
        };
        self.send(Request::Notice {
            id: id.to_owned(),
            action,
        });
        if let Some(notice) = self.state.notifications.iter_mut().find(|n| n.id == id) {
            notice.snoozed_until = now().saturating_add(secs);
        }
        if self.detail.as_deref() == Some(id) {
            self.detail = None;
        }
    }

    /// "Dismissed: label  [Undo]" above the inbox after a dismiss or snooze.
    pub(crate) fn triage_undo_bar(&mut self, ui: &mut eframe::egui::Ui) {
        let Some(undo) = self.triage.undo.clone() else {
            return;
        };
        let mut clicked = false;
        ui.horizontal(|ui| {
            let response =
                crate::appearance::icon_label_button(crate::appearance::IconLabelButtonInput {
                    ui,
                    icon: "Undo2",
                    label: "Undo",
                    height: crate::appearance::TOOLBAR_BUTTON,
                })
                .on_hover_text("Show the notices in the inbox again");
            #[cfg(feature = "test-support")]
            crate::diagnostics::record(ui.ctx(), "agents-undo", response.rect);
            clicked = response.clicked();
            ui.weak(&undo.label);
        });
        if clicked {
            self.undo_triage();
        }
    }

    /// Pin and Peek on a session row: its context menu, and P or Space
    /// while it has keyboard focus. True when Space peeked, so the row's
    /// keyboard click does not also go to the session.
    pub(crate) fn session_row_triage(
        &mut self,
        response: &eframe::egui::Response,
        session: &str,
        pinned: bool,
    ) -> bool {
        use eframe::egui::{Key, Modifiers};
        let mut pin = false;
        let mut peek = false;
        if response.has_focus() {
            response.ctx.input_mut(|input| {
                peek = input.key_pressed(Key::Space);
                pin = input.consume_key(Modifiers::NONE, Key::P);
            });
        }
        response.context_menu(|ui| {
            let label = if pinned { "Unpin" } else { "Pin to top" };
            if crate::appearance::menu_item(crate::appearance::MenuItemInput {
                ui,
                label,
                icon: "Pin",
                shortcut: "P",
            })
            .clicked()
            {
                pin = true;
                ui.close();
            }
            if crate::appearance::menu_item(crate::appearance::MenuItemInput {
                ui,
                label: "Peek screen",
                icon: "Eye",
                shortcut: "Space",
            })
            .clicked()
            {
                peek = true;
                ui.close();
            }
        });
        if pin {
            self.toggle_session_pin(session);
        }
        if peek {
            self.open_session_peek(session);
        }
        peek
    }

    /// Pinned items first, the rest in their order.
    pub(crate) fn pinned_first<'a, T>(
        &self,
        items: &'a [T],
        session: impl Fn(&T) -> Option<&str>,
    ) -> Vec<&'a T> {
        let pinned = |item: &&T| session(item).is_some_and(|sid| self.session_pinned(sid));
        let (mut first, rest): (Vec<&T>, Vec<&T>) = items.iter().partition(pinned);
        first.extend(rest);
        first
    }
}
