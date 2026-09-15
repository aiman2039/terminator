//! One asynchronous preflight for pane, workspace, shortcut and sidebar close intents.
use super::*;
use terminator_core::idle_close::{Outcome, Status};

impl App {
    fn idle_target_tabs(&self, target: &editor_close::Target) -> Vec<Tab> {
        match target {
            editor_close::Target::Pane(sid) => vec![Tab::Terminal(sid.clone())],
            editor_close::Target::Workspace(project, tab) => self
                .layouts
                .get(project)
                .and_then(|workspace| workspace.tabs.iter().find(|t| &t.id == tab))
                .map(|t| {
                    t.layout
                        .iter_all_tabs()
                        .map(|(_, tab)| tab.clone())
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    pub(super) fn check_idle_close(
        &mut self,
        target: editor_close::Target,
        sessions: Vec<String>,
    ) -> bool {
        if !self
            .state
            .capabilities
            .iter()
            .any(|c| c == terminator_core::idle_close::CAPABILITY)
            || sessions.iter().any(|id| {
                self.state
                    .sessions
                    .iter()
                    .find(|s| &s.id == id)
                    .is_none_or(|s| s.kind != SessionKind::Shell)
            })
        {
            return false;
        }
        if self.idle_close_fallback.as_ref() == Some(&target) {
            return false;
        }
        if self.idle_close_pending.is_some() {
            return true;
        }
        self.idle_close_snapshot = self.idle_target_tabs(&target);
        self.idle_close_pending = Some(target.clone());
        if self
            .jobs
            .send(Job::CloseIdle(
                target.clone(),
                self.state.generation.clone(),
                sessions,
            ))
            .is_err()
        {
            self.idle_close_pending = None;
            self.idle_close_snapshot.clear();
            self.idle_close_fallback = Some(target);
            self.error = Some("Close worker is unavailable; sessions have been preserved.".into());
            return false;
        }
        true
    }

    pub(super) fn idle_closed(
        &mut self,
        target: editor_close::Target,
        ids: Vec<String>,
        result: Result<Vec<Outcome>, String>,
    ) {
        self.idle_close_pending = None;
        let changed = self.idle_target_tabs(&target) != self.idle_close_snapshot;
        self.idle_close_snapshot.clear();
        if idle_close_succeeded(&ids, &result, changed) {
            self.apply_idle_close(&target);
            self.idle_close_fallback = None;
            return;
        }
        self.idle_close_fallback = Some(target);
        if let Some(error) = idle_close_failure_message(changed, result) {
            self.error = Some(error);
        }
    }

    fn apply_idle_close(&mut self, target: &editor_close::Target) {
        match target {
            editor_close::Target::Pane(sid) => {
                self.remove_tab(sid);
                if self.close_session.as_ref() == Some(sid) {
                    self.close_session = None;
                }
            }
            editor_close::Target::Workspace(project, tab) => {
                if let Some(workspace) = self.layouts.get_mut(project) {
                    workspace.close(tab);
                }
                if self.close_workspace.as_ref() == Some(&(project.clone(), tab.clone())) {
                    self.close_workspace = None;
                }
            }
        }
    }
}

fn idle_close_succeeded(
    ids: &[String],
    result: &Result<Vec<Outcome>, String>,
    changed: bool,
) -> bool {
    !changed
        && result.as_ref().is_ok_and(|outcomes| {
            ids.iter().all(|id| {
                outcomes.iter().any(|outcome| {
                    &outcome.session == id
                        && matches!(outcome.status, Status::Closed | Status::AlreadyEnded)
                })
            })
        })
}

fn idle_close_failure_message(
    changed: bool,
    result: Result<Vec<Outcome>, String>,
) -> Option<String> {
    if changed {
        return Some(
            "Tab contents changed while closing. The view was preserved; any confirmed shell exits remain in effect.".into(),
        );
    }
    match result {
        Err(error) => Some(error),
        Ok(outcomes) => outcomes
            .iter()
            .any(|outcome| matches!(outcome.status, Status::Failed))
            .then(|| {
                outcomes
                    .into_iter()
                    .map(|outcome| {
                        let Outcome {
                            session,
                            status,
                            reason,
                        } = outcome;
                        format!("{session}: {status:?}: {reason}")
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            }),
    }
}
