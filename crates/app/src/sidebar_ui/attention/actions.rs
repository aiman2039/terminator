//! What an inbox card action does. Read state, dismissal, snooze, pins and
//! peeks only: nothing here resolves a notice, changes agent lifecycle or
//! writes to a terminal.
use super::{AttentionAction, NoticeGroup, TriageUndo};
use crate::App;
use terminator_core::Request;

impl App {
    /// One row acts as one unit: navigation, pin and peek need a single
    /// target, while read/snooze/dismiss apply to every notice in the row.
    pub(crate) fn apply_group_action(&mut self, group: &NoticeGroup, action: AttentionAction) {
        match action {
            AttentionAction::None => {}
            AttentionAction::Go
            | AttentionAction::Review
            | AttentionAction::Pin
            | AttentionAction::Peek => {
                if let Some(first) = group.notices.first() {
                    self.apply_notice_action(first.id.clone(), action);
                }
            }
            _ => {
                for notice in &group.notices {
                    self.apply_notice_action(notice.id.clone(), action);
                }
                self.remember_undo(group, action);
            }
        }
    }

    /// Dismiss and snooze can be undone once, when the daemon can restore.
    fn remember_undo(&mut self, group: &NoticeGroup, action: AttentionAction) {
        let label = match action {
            AttentionAction::Dismiss => "Dismissed",
            AttentionAction::Snooze | AttentionAction::SnoozeFor(_) => "Snoozed",
            _ => return,
        };
        if !self.triage_available() {
            return;
        }
        let session = group
            .notices
            .first()
            .and_then(|n| self.state.sessions.iter().find(|s| s.id == n.session_id))
            .map_or("Agent", |s| s.label.as_str());
        self.triage.undo = Some(TriageUndo {
            label: format!("{label}: {session}"),
            ids: group.notices.iter().map(|n| n.id.clone()).collect(),
        });
    }

    fn go_notice(&mut self, id: String) {
        if let Some(session) = self.notice_session(&id) {
            self.go_session(&session);
        }
        self.unread_selected = Some(id);
        self.detail = None;
    }

    fn notice_session(&self, id: &str) -> Option<String> {
        self.state
            .notifications
            .iter()
            .find(|notice| notice.id == id)
            .map(|notice| notice.session_id.clone())
    }

    pub(crate) fn apply_notice_action(&mut self, id: String, action: AttentionAction) {
        let presentation_changed = match action {
            AttentionAction::None => false,
            AttentionAction::Go => {
                self.go_notice(id);
                false
            }
            AttentionAction::Review => {
                self.go_notice(id);
                // Read-only: the Git sidebar follows the focused session's
                // working directory; no Git state changes.
                self.show_git_sidebar();
                false
            }
            AttentionAction::Read | AttentionAction::Unread => {
                self.set_notice_read(id, action == AttentionAction::Read)
            }
            AttentionAction::Snooze => {
                self.snooze_notice(&id, None);
                true
            }
            AttentionAction::SnoozeFor(secs) => {
                self.snooze_notice(&id, Some(secs));
                true
            }
            AttentionAction::Dismiss => {
                self.dismiss_notice(&id);
                true
            }
            AttentionAction::Pin => {
                if let Some(session) = self.notice_session(&id) {
                    self.toggle_session_pin(&session);
                }
                false
            }
            AttentionAction::Peek => {
                if let Some(session) = self.notice_session(&id) {
                    self.open_session_peek(&session);
                }
                false
            }
        };
        if presentation_changed {
            self.reconcile_presentations();
        }
    }

    /// Read state only: never resolve, dismiss, or touch lifecycle. Mark
    /// unread needs `notice-triage-v1`; older daemons keep the notice read.
    fn set_notice_read(&mut self, id: String, read: bool) -> bool {
        if !read && !self.triage_available() {
            return false;
        }
        self.send(Request::Notice {
            id: id.clone(),
            action: if read { "read" } else { "unread" }.into(),
        });
        let changed = if let Some(notice) = self
            .state
            .notifications
            .iter_mut()
            .find(|notice| notice.id == id)
        {
            notice.read = read;
            true
        } else {
            false
        };
        self.unread_selected = Some(id);
        changed
    }

    fn dismiss_notice(&mut self, id: &str) {
        self.send(Request::Notice {
            id: id.to_owned(),
            action: "dismiss".into(),
        });
        if let Some(notice) = self
            .state
            .notifications
            .iter_mut()
            .find(|notice| notice.id == id)
        {
            notice.dismissed = true;
        }
        if self.unread_selected.as_deref() == Some(id) {
            self.unread_selected = None;
        }
        if self.detail.as_deref() == Some(id) {
            self.detail = None;
        }
    }
}
