use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};
use terminator_core::*;

use super::super::super::shared::{Shared, relock};
use super::Outcome;

impl Shared {
    pub(super) fn on_add_project(&self, path: &Path) -> Result<Outcome> {
        let path = path.canonicalize()?;
        ensure!(path.is_dir(), "Project must be a directory");
        let mut s = relock(&self.state);
        match s
            .projects
            .iter()
            .find(|p| p.path == path)
            .map(|p| p.id.clone())
        {
            Some(project) => Self::reselect_project(&mut s, project),
            None => Self::push_project(&mut s, path),
        }
        Ok(Outcome::Changed)
    }

    /// Re-adding a known path forgets its removed-worktree marker and selects it.
    fn reselect_project(s: &mut State, project: String) {
        let previous = s.worktrees.len();
        s.worktrees
            .retain(|w| w.project_id != project || !w.removed);
        if previous != s.worktrees.len() {
            s.revision = s.revision.saturating_add(1);
        }
        if s.selected_project.as_ref() != Some(&project) {
            s.selected_project = Some(project);
            s.revision = s.revision.saturating_add(1);
        }
    }

    fn push_project(s: &mut State, path: PathBuf) {
        let pid = id();
        s.projects.push(Project {
            id: pid.clone(),
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            path,
            layout: serde_json::Value::Null,
        });
        s.selected_project = Some(pid);
        s.revision = s.revision.saturating_add(1);
    }

    pub(super) fn on_save_layout(
        &self,
        project: &str,
        layout: serde_json::Value,
    ) -> Result<Outcome> {
        ensure!(
            serde_json::to_vec(&layout)?.len() < 512 * 1024,
            "Layout too large"
        );
        let mut s = relock(&self.state);
        if let Some(remote) = s.remote_projects.iter_mut().find(|p| p.id == project) {
            remote.layout = layout;
        } else {
            s.projects
                .iter_mut()
                .find(|p| p.id == project)
                .context("Unknown project")?
                .layout = layout;
        }
        s.revision = s.revision.saturating_add(1);
        Ok(Outcome::Changed)
    }

    pub(super) fn on_select_project(&self, project: String) -> Result<Outcome> {
        let mut s = relock(&self.state);
        ensure!(
            !s.worktrees
                .iter()
                .any(|w| w.project_id == project && w.removed),
            "Worktree was removed"
        );
        ensure!(
            s.projects.iter().any(|p| p.id == project)
                || s.remote_projects.iter().any(|p| p.id == project),
            "Unknown project"
        );
        s.selected_project = Some(project);
        s.revision = s.revision.saturating_add(1);
        Ok(Outcome::Changed)
    }
}
