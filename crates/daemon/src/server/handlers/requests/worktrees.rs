use anyhow::{Context, Result, bail, ensure};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use terminator_core::*;

use super::super::super::shared::{Shared, relock};
use super::Outcome;

/// Tries for the coordination lock after `git worktree add` (2 s each).
const LOCK_ATTEMPTS: u32 = 5;

pub(super) struct WorktreeAddInput {
    pub project: String,
    pub path: PathBuf,
    pub branch: Option<String>,
    pub start: String,
}

impl Shared {
    pub(super) fn on_worktree_list(&self, project: &str) -> Result<Outcome> {
        let path = self.project_path(project)?;
        Ok(Outcome::Reply(Response::Worktrees(worktrees::list(
            &worktrees::common_dir(&path)?,
        )?)))
    }

    /// `coordination` holds the cross-generation lock on entry; it is held
    /// again on success so the caller saves the workspace under it.
    pub(super) fn on_worktree_add(
        self: &Arc<Self>,
        input: WorktreeAddInput,
        coordination: &mut Option<std::fs::File>,
    ) -> Result<Outcome> {
        let WorktreeAddInput {
            project,
            path,
            branch,
            start,
        } = input;
        // No worktree or coordination lock here: a long `git worktree add`
        // must not block Create and Cwd (#190), which wait only 2 s for the
        // coordination lock. Removal only acts on registered checkouts, and
        // this one registers after git finishes.
        drop(coordination.take());
        let source = self.project_path(&project)?;
        let common_dir = worktrees::common_dir(&source)?;
        let revision = worktrees::resolve_start(&source, &start)?;
        // A retry after an add that created the checkout but could not
        // register it adopts that checkout instead of failing on the path.
        let checkout = match self.unregistered_checkout(&common_dir, &path, branch.as_deref())? {
            Some(checkout) => checkout,
            None => worktrees::add(worktrees::AddInput {
                common: &common_dir,
                path: &path,
                branch: branch.as_deref(),
                start: &revision,
            })?,
        };
        let unregistered = || {
            format!(
                "Worktree {} was created but not added as a project; try again to add it",
                checkout.path.display()
            )
        };
        let prepared = self.prepare_after_git().with_context(unregistered)?;
        if prepared.early.is_some() {
            bail!(
                "{}: this session service is no longer active",
                unregistered()
            );
        }
        *coordination = prepared.coordination;
        self.register_worktree(common_dir, checkout.path);
        Ok(Outcome::Changed)
    }

    /// Take the coordination lock again after the Git run. Other requests
    /// hold it only briefly, so wait through a few of their 2 s attempts.
    fn prepare_after_git(self: &Arc<Self>) -> Result<super::prepare::PreparedRequest> {
        for _ in 1..LOCK_ATTEMPTS {
            if let Ok(prepared) = self.prepare_shared_write() {
                return Ok(prepared);
            }
        }
        self.prepare_shared_write()
    }

    /// The checkout Git has at `path` on `branch` that no project uses.
    fn unregistered_checkout(
        &self,
        common: &Path,
        path: &Path,
        branch: Option<&str>,
    ) -> Result<Option<worktrees::GitWorktree>> {
        let Some(checkout) = worktrees::existing(common, path, branch)? else {
            return Ok(None);
        };
        let state = relock(&self.state);
        let known: Vec<PathBuf> = state
            .projects
            .iter()
            .map(|p| p.path.clone())
            .chain(
                state
                    .worktrees
                    .iter()
                    .filter(|w| !w.removed)
                    .map(|w| w.path.clone()),
            )
            .collect();
        drop(state);
        let used = known.iter().any(|known| {
            *known == checkout.path
                || std::fs::canonicalize(known).is_ok_and(|real| real == checkout.path)
        });
        Ok((!used).then_some(checkout))
    }

    pub(super) fn on_worktree_remove(&self, project: &str) -> Result<Outcome> {
        let _worktree_guard = relock(&self.worktree_operations);
        let (record, sessions) = self.worktree_removal_inputs(project)?;
        worktrees::ensure_unused(&record.path, project, &sessions)?;
        worktrees::remove(&record.common_dir, &record.path)?;
        self.mark_worktree_removed(project);
        Ok(Outcome::Changed)
    }

    fn project_path(&self, project: &str) -> Result<PathBuf> {
        Ok(relock(&self.state)
            .projects
            .iter()
            .find(|p| p.id == project)
            .context("Unknown project")?
            .path
            .clone())
    }

    fn register_worktree(&self, common_dir: PathBuf, path: PathBuf) {
        let mut state = relock(&self.state);
        let project_id = id();
        let project = Project {
            id: project_id.clone(),
            name: Self::checkout_name(&path),
            path: path.clone(),
            layout: serde_json::Value::Null,
        };
        state.projects.push(project);
        state.worktrees.push(worktrees::Registration {
            project_id,
            common_dir,
            path,
            created: now(),
            removed: false,
        });
        state.revision = state.revision.saturating_add(1);
    }

    fn checkout_name(path: &Path) -> String {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    }

    /// The managed worktree record and the sessions that must not use it,
    /// taken from every owner when a generation catalog exists.
    fn worktree_removal_inputs(
        &self,
        project: &str,
    ) -> Result<(worktrees::Registration, Vec<Session>)> {
        let state = relock(&self.state);
        let record = state
            .worktrees
            .iter()
            .find(|w| w.project_id == project && !w.removed)
            .context("Project is not an active managed worktree")?
            .clone();
        let mut sessions = state.sessions.clone();
        drop(state);
        if let Some(paths) = &self.catalog_paths {
            let inventory = generations::snapshot(paths)?;
            ensure!(
                inventory.generations.iter().all(|g| g.error.is_none()),
                "Cannot establish worktree safety while an owner is unavailable"
            );
            sessions = inventory.sessions;
        }
        Ok((record, sessions))
    }

    fn mark_worktree_removed(&self, project: &str) {
        let mut state = relock(&self.state);
        if let Some(record) = state.worktrees.iter_mut().find(|w| w.project_id == project) {
            record.removed = true;
        }
        if state.selected_project.as_deref() == Some(project) {
            state.selected_project = state
                .projects
                .iter()
                .find(|candidate| {
                    candidate.id != project
                        && !state
                            .worktrees
                            .iter()
                            .any(|w| w.project_id == candidate.id && w.removed)
                })
                .map(|candidate| candidate.id.clone());
        }
        state.revision = state.revision.saturating_add(1);
    }
}
