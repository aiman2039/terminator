//! One asynchronous preflight for pane, workspace, shortcut and sidebar close intents.
use crate::*;
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

    pub(crate) fn check_idle_close(
        &mut self,
        target: editor_close::Target,
        sessions: Vec<String>,
    ) -> bool {
        if !self.idle_close_applies(&target, &sessions) {
            return false;
        }
        // One preflight at a time. A different target waits: its caller
        // keeps the request (`close_session` / `close_workspace`) and asks
        // again each frame, so it starts once the pending one completes.
        if self.idle_close_pending.is_some() {
            return true;
        }
        let owners = self.session_owners(&sessions);
        self.idle_close_snapshot = self.idle_target_tabs(&target);
        self.idle_close_pending = Some(target.clone());
        if self
            .jobs
            .send(Job::CloseIdle(
                target.clone(),
                owners
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| self.state.generation.clone()),
                sessions,
            ))
            .is_err()
        {
            self.idle_close_pending = None;
            self.idle_close_snapshot.clear();
            self.idle_close_fallback = Some(target);
            self.error
                .set("Close worker is unavailable; sessions have been preserved.");
            return false;
        }
        true
    }

    /// Whether `target` goes through the daemon idle close rather than a
    /// confirmation: supported, and not already fallen back.
    fn idle_close_applies(&self, target: &editor_close::Target, sessions: &[String]) -> bool {
        self.idle_close_fallback.as_ref() != Some(target)
            && self.idle_close_supported(sessions, &self.session_owners(sessions))
    }

    /// Whether a pane or workspace close shows its confirmation now. A
    /// close in its idle preflight (or queued behind another one), in its
    /// editor check, or with nothing live left shows none and leaves
    /// terminal input alone. Each close answers for itself, so one close's
    /// preflight never opens the gate for the other's confirmation.
    pub(crate) fn close_confirm_blocks_input(&self) -> bool {
        let confirming = |target: editor_close::Target, live: Vec<String>| {
            !live.is_empty()
                && !self.editors_only(&live)
                && !(self.idle_close_pending.is_some() && self.idle_close_applies(&target, &live))
        };
        let pane = self.close_session.as_ref().is_some_and(|sid| {
            confirming(editor_close::Target::Pane(sid.clone()), vec![sid.clone()])
        });
        pane || self.close_workspace.as_ref().is_some_and(|(project, tab)| {
            confirming(
                editor_close::Target::Workspace(project.clone(), tab.clone()),
                self.live_workspace_sessions(project, tab),
            )
        })
    }

    fn session_owners(&self, sessions: &[String]) -> std::collections::BTreeSet<String> {
        sessions
            .iter()
            .filter_map(|id| {
                self.state
                    .sessions
                    .iter()
                    .find(|s| &s.id == id)
                    .map(|s| s.generation.clone())
            })
            .collect()
    }

    /// Daemon idle close applies only to live-owner shell sessions of one
    /// owner that advertises it. A mixed-owner batch needs a distributed
    /// preflight, so it keeps the normal confirmation flow instead of
    /// partially closing across owners.
    fn idle_close_supported(
        &self,
        sessions: &[String],
        owners: &std::collections::BTreeSet<String>,
    ) -> bool {
        if sessions.iter().any(|id| {
            self.state
                .sessions
                .iter()
                .find(|session| &session.id == id)
                .is_some_and(|session| {
                    !daemon_connection::session_owner_available(&self.state, session)
                })
        }) {
            return false;
        }
        if !self.state.generations.is_empty()
            && (owners.len() != 1
                || !self.state.generations.iter().any(|g| {
                    owners.contains(&g.owner.id)
                        && g.capabilities
                            .iter()
                            .any(|c| c == terminator_core::idle_close::CAPABILITY)
                }))
        {
            return false;
        }
        self.state
            .capabilities
            .iter()
            .any(|c| c == terminator_core::idle_close::CAPABILITY)
            && !sessions.iter().any(|id| {
                self.state
                    .sessions
                    .iter()
                    .find(|s| &s.id == id)
                    .is_none_or(|s| s.kind != SessionKind::Shell)
            })
    }

    pub(crate) fn idle_closed(
        &mut self,
        target: editor_close::Target,
        ids: Vec<String>,
        result: Result<Vec<Outcome>, String>,
    ) {
        self.idle_close_pending = None;
        // The daemon's own state broadcast often prunes a closing shell's
        // tab before this completion arrives; that removal is the close
        // working, not a change. Only foreign additions/removals count.
        let changed = idle_contents_changed(
            &self.idle_close_snapshot,
            &self.idle_target_tabs(&target),
            &ids,
        );
        self.idle_close_snapshot.clear();
        if idle_close_succeeded(&ids, &result, changed) {
            self.apply_idle_close(&target);
            self.idle_close_fallback = None;
            return;
        }
        self.idle_close_fallback = Some(target);
        if let Some(error) = idle_close_failure_message(changed, result) {
            self.error.set(error);
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
                self.close_workspace_tab_now(project, tab);
            }
        }
    }
}

/// Whether the target's contents changed in a way the close did not
/// cause: a tab appeared, or a tab vanished that is not one of the
/// sessions being closed (those are pruned by the daemon's state sync
/// before the close completion lands, notably when closing many tabs).
fn idle_contents_changed(snapshot: &[Tab], current: &[Tab], ids: &[String]) -> bool {
    current.iter().any(|tab| !snapshot.contains(tab))
        || snapshot.iter().any(|tab| {
            !current.contains(tab)
                && !matches!(tab, Tab::Terminal(sid) if ids.iter().any(|id| id == sid))
        })
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
