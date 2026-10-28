use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};
use terminator_core::*;

use super::super::super::shared::{Shared, relock};
use super::Outcome;

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

    pub(super) fn on_worktree_add(&self, input: WorktreeAddInput) -> Result<Outcome> {
        let WorktreeAddInput {
            project,
            path,
            branch,
            start,
        } = input;
        let _worktree_guard = relock(&self.worktree_operations);
        let source = self.project_path(&project)?;
        let common_dir = worktrees::common_dir(&source)?;
        let revision = worktrees::resolve_start(&source, &start)?;
        let checkout = worktrees::add(worktrees::AddInput {
            common: &common_dir,
            path: &path,
            branch: branch.as_deref(),
            start: &revision,
        })?;
        self.register_worktree(common_dir, checkout.path);
        Ok(Outcome::Changed)
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
