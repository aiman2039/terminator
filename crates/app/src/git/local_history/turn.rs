//! Last turn: the files an agent's newest turn changed (Local History saw
//! them change after the hook `Running` event that started the turn), a
//! diff of each against its turn-start snapshot, and a confirmed "Revert
//! last turn…" through the Local History revert path, which keeps the
//! replaced content as "Before revert". Never runs Git; never writes a
//! file Local History did not record. The daemon delimits turns
//! (`local-history-turn-v1`); an older daemon hides the scope.
use crate::{App, CenterPage, Job, diff::DiffDocument};
use std::{path::PathBuf, sync::Arc};
use terminator_core::async_service::Policy;
use terminator_core::local_history::{TURN_CAPABILITY, Turn};
use terminator_core::{AgentState, SessionKind};

/// Daemon work for the page; runs on the GUI services.
#[derive(Clone, Debug)]
pub(crate) enum TurnOp {
    Load(String),
    /// Diff the turn-start snapshot `base` (none: empty) against the file.
    Diff {
        path: PathBuf,
        base: Option<String>,
    },
    /// Confirmed revert of `session`'s turn.
    Revert(String, Vec<RevertItem>),
}

pub(crate) enum TurnReply {
    Loaded(String, Result<Option<Box<Turn>>, String>),
    Diff(PathBuf, Result<Arc<DiffDocument>, String>),
    Reverted(String, RevertReport),
}

/// One file to put back: snapshot `base` over `path`, only while the file
/// still holds `newest` (the content the list showed) or is gone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RevertItem {
    pub(crate) path: PathBuf,
    pub(crate) base: String,
    pub(crate) newest: Option<String>,
}

/// What a revert did. A refusal writes nothing; a failure stops the rest.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RevertReport {
    pub(crate) reverted: Vec<PathBuf>,
    pub(crate) refused: Vec<(PathBuf, String)>,
    pub(crate) failed: Option<(PathBuf, String)>,
    pub(crate) not_tried: Vec<PathBuf>,
}

impl RevertReport {
    pub(crate) fn complete(&self) -> bool {
        self.refused.is_empty() && self.failed.is_none() && self.not_tried.is_empty()
    }

    /// One line for the status bar; every file that was not reverted is
    /// named, so a partial result is never silent.
    pub(crate) fn summary(&self) -> String {
        let names = |paths: &mut dyn Iterator<Item = &PathBuf>| {
            paths
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        if !self.refused.is_empty() {
            let why: Vec<String> = self
                .refused
                .iter()
                .map(|(p, e)| format!("{}: {e}", p.display()))
                .collect();
            return format!("Revert refused; no file was changed. {}", why.join("; "));
        }
        let mut text = format!("Reverted {} file(s).", self.reverted.len());
        if let Some((path, error)) = &self.failed {
            text.push_str(&format!(" Failed on {}: {error}.", path.display()));
        }
        if !self.not_tried.is_empty() {
            text.push_str(&format!(
                " Not reverted: {}.",
                names(&mut self.not_tried.iter())
            ));
        }
        text
    }
}

/// The files a revert writes, and the ones it leaves.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RevertPlan {
    /// Turn-start snapshot known.
    pub(crate) items: Vec<RevertItem>,
    /// Only the Git index version from before the first seen change.
    pub(crate) index_items: Vec<RevertItem>,
    /// Not reverted, with the reason.
    pub(crate) skipped: Vec<(PathBuf, &'static str)>,
}

impl RevertPlan {
    pub(crate) fn chosen(&self, include_index: bool) -> Vec<RevertItem> {
        let mut items = self.items.clone();
        if include_index {
            items.extend(self.index_items.iter().cloned());
        }
        items
    }
}

pub(crate) fn plan(turn: &Turn) -> RevertPlan {
    let mut plan = RevertPlan::default();
    for file in &turn.files {
        let Some(base) = &file.base else {
            plan.skipped
                .push((file.path.clone(), "new in this turn or no snapshot"));
            continue;
        };
        if file.newest.as_deref() == Some(base.id.as_str()) {
            plan.skipped.push((file.path.clone(), "unchanged"));
            continue;
        }
        let item = RevertItem {
            path: file.path.clone(),
            base: base.id.clone(),
            newest: file.newest.clone(),
        };
        if file.base_is_git_index {
            plan.index_items.push(item);
        } else {
            plan.items.push(item);
        }
    }
    plan
}

/// Service route: reads replace each other; a revert is ordered.
pub(crate) fn route(op: &TurnOp) -> (&'static str, String, Policy) {
    match op {
        TurnOp::Load(session) => (
            "local-history",
            format!("turn:{session}"),
            Policy::ReplaceableRead,
        ),
        TurnOp::Diff { .. } => ("local-history", "turn-diff".into(), Policy::ReplaceableRead),
        TurnOp::Revert(..) => (
            "local-history",
            "turn-revert".into(),
            Policy::OrderedMutation,
        ),
    }
}

impl TurnOp {
    pub(crate) fn failed(&self, error: String) -> TurnReply {
        match self {
            Self::Load(session) => TurnReply::Loaded(session.clone(), Err(error)),
            Self::Diff { path, .. } => TurnReply::Diff(path.clone(), Err(error)),
            // The job may have stopped part way: never claim nothing changed.
            Self::Revert(session, _) => TurnReply::Reverted(
                session.clone(),
                RevertReport {
                    failed: Some((
                        PathBuf::from("the revert"),
                        format!(
                            "{error}; it stopped, so some files may be reverted already. Reload to see them"
                        ),
                    )),
                    ..RevertReport::default()
                },
            ),
        }
    }
}

/// Data of `CenterPage::LastTurn`.
#[derive(Default)]
pub(crate) struct LastTurn {
    pub(crate) session: String,
    pub(crate) label: String,
    pub(crate) turn: Option<Result<Option<Box<Turn>>, String>>,
    pub(crate) selected: Option<PathBuf>,
    pub(crate) doc: Option<Result<Arc<DiffDocument>, String>>,
    /// The confirmation shows.
    pub(crate) confirm: bool,
    /// Also restore files whose only base is the Git index version.
    pub(crate) include_index: bool,
    pub(crate) busy: bool,
}

impl App {
    pub(crate) fn last_turn_supported(&self) -> bool {
        self.state.capabilities.iter().any(|c| c == TURN_CAPABILITY)
    }

    /// The agent session whose turn to show: the active session when a
    /// hook reported an agent in it, else the newest agent of `project`.
    pub(crate) fn last_turn_session(&self, project: Option<&str>) -> Option<String> {
        let agent_of = |sid: &str| self.state.agents.iter().any(|a| a.session_id == sid);
        if let Some(active) = self.active_session.as_deref()
            && agent_of(active)
        {
            return Some(active.to_owned());
        }
        let project = project?;
        self.state
            .agents
            .iter()
            .filter(|a| {
                self.state
                    .sessions
                    .iter()
                    .any(|s| s.id == a.session_id && s.project_id == project)
            })
            .max_by_key(|a| a.updated)
            .map(|a| a.session_id.clone())
    }

    /// "Last turn changes" in an agent terminal's pane menu.
    pub(crate) fn last_turn_menu_item(&mut self, ui: &mut eframe::egui::Ui, sid: &str) {
        let agent = self.state.agents.iter().any(|a| a.session_id == sid);
        if !agent || !self.last_turn_supported() {
            return;
        }
        if crate::appearance::menu_action(ui, true, "Last turn changes", "History").clicked() {
            self.open_last_turn(sid.to_owned());
            ui.close();
        }
    }

    /// Review last agent turn (action and palette).
    pub(crate) fn review_last_turn(&mut self) {
        let project = self.selected_project().map(|p| p.id.clone());
        match self.last_turn_session(project.as_deref()) {
            Some(session) => self.open_last_turn(session),
            None => self.info = Some("No agent turn in this project yet.".into()),
        }
    }

    pub(crate) fn open_last_turn(&mut self, session: String) {
        if !self.last_turn_supported() {
            self.error
                .set("Last turn needs an updated session service.");
            return;
        }
        let label = self
            .state
            .sessions
            .iter()
            .find(|s| s.id == session)
            .map(|s| s.label.clone())
            .unwrap_or_default();
        self.show_center(CenterPage::LastTurn);
        let _ = self
            .jobs
            .send(Job::LocalHistory(super::Op::Turn(TurnOp::Load(
                session.clone(),
            ))));
        self.git_extras.last_turn = Some(LastTurn {
            session,
            label,
            ..LastTurn::default()
        });
    }

    pub(crate) fn reload_last_turn(&mut self) {
        if let Some(page) = self.git_extras.last_turn.take() {
            self.open_last_turn(page.session);
        }
    }

    pub(crate) fn select_turn_file(&mut self, path: PathBuf) {
        let Some(page) = self.git_extras.last_turn.as_mut() else {
            return;
        };
        let Some(Ok(Some(turn))) = &page.turn else {
            return;
        };
        let Some(file) = turn.files.iter().find(|f| f.path == path) else {
            return;
        };
        let base = file.base.as_ref().map(|b| b.id.clone());
        page.selected = Some(path.clone());
        page.doc = None;
        let _ = self
            .jobs
            .send(Job::LocalHistory(super::Op::Turn(TurnOp::Diff {
                path,
                base,
            })));
    }

    pub(crate) fn handle_turn_reply(&mut self, reply: TurnReply) {
        let Some(page) = self.git_extras.last_turn.as_mut() else {
            return;
        };
        match reply {
            TurnReply::Loaded(session, result) if session == page.session => {
                let first = result
                    .as_ref()
                    .ok()
                    .and_then(|turn| turn.as_ref()?.files.first().map(|f| f.path.clone()));
                page.turn = Some(result);
                page.selected = None;
                page.doc = None;
                if let Some(path) = first {
                    self.select_turn_file(path);
                }
            }
            TurnReply::Diff(path, result) if page.selected.as_ref() == Some(&path) => {
                page.doc = Some(result);
            }
            TurnReply::Reverted(session, report) if session == page.session => {
                page.busy = false;
                if report.complete() {
                    self.info = Some(report.summary());
                } else {
                    self.error.set(report.summary());
                }
                self.reload_last_turn();
            }
            _ => {}
        }
    }

    /// Why the revert cannot run now: an unsaved, loading or saving native
    /// buffer of a listed file, a live Neovim editor of one (its buffer
    /// state is unknown), or the agent still working.
    pub(crate) fn turn_revert_blockers(&self, session: &str, items: &[RevertItem]) -> Vec<String> {
        let mut blockers = Vec::new();
        let running = self
            .state
            .agents
            .iter()
            .any(|a| a.session_id == session && a.state == AgentState::Running);
        if running {
            blockers.push("The agent is still working: wait for the turn to end.".to_owned());
        }
        for item in items {
            if let Some(doc) = self.native_docs.get(&item.path)
                && (doc.dirty() || doc.busy())
            {
                blockers.push(format!("Save {} first.", item.path.display()));
            }
            let editor = self.state.sessions.iter().any(|s| {
                s.kind == SessionKind::Editor
                    && s.lifecycle.live()
                    && s.file.as_ref() == Some(&item.path)
            });
            if editor {
                blockers.push(format!(
                    "Close the editor of {} first.",
                    item.path.display()
                ));
            }
        }
        blockers
    }

    /// Sends the revert after the user confirmed it. Blockers refuse the
    /// whole revert before any write.
    pub(crate) fn revert_last_turn(&mut self) {
        let Some(page) = self.git_extras.last_turn.as_ref() else {
            return;
        };
        let Some(Ok(Some(turn))) = &page.turn else {
            return;
        };
        let items = plan(turn).chosen(page.include_index);
        let session = page.session.clone();
        if items.is_empty() {
            self.info = Some("Nothing to revert in this turn.".into());
            return;
        }
        let blockers = self.turn_revert_blockers(&session, &items);
        if !blockers.is_empty() {
            self.error.set(format!(
                "Revert refused; no file was changed. {}",
                blockers.join(" ")
            ));
            return;
        }
        if let Some(page) = self.git_extras.last_turn.as_mut() {
            page.busy = true;
            page.confirm = false;
        }
        let _ = self
            .jobs
            .send(Job::LocalHistory(super::Op::Turn(TurnOp::Revert(
                session, items,
            ))));
    }
}

#[cfg(test)]
#[path = "turn_tests.rs"]
mod tests;
