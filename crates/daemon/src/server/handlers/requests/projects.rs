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

    pub(super) fn on_set_init_command(
        &self,
        project: &str,
        command: Option<&str>,
    ) -> Result<Outcome> {
        let mut s = relock(&self.state);
        if !set_init_command(&mut s, project, command)? {
            return Ok(Outcome::Reply(Response::Ok));
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

/// Stores (or clears, when blank) a local project's init command. True when
/// it changed.
fn set_init_command(s: &mut State, project: &str, command: Option<&str>) -> Result<bool> {
    let command = command.and_then(|text| remote::normalize_command(text, ""));
    if let Some(command) = &command {
        ensure!(
            command.len() <= remote::MAX_COMMAND_LEN && !command.chars().any(char::is_control),
            "Init command must be one line"
        );
    }
    ensure!(
        s.projects.iter().any(|p| p.id == project),
        "Init commands are for local projects"
    );
    Ok(match command {
        Some(command) => {
            s.init_commands.insert(project.to_owned(), command.clone()) != Some(command)
        }
        None => s.init_commands.remove(project).is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_command_is_one_line_for_a_local_project() {
        let mut s = State::default();
        s.projects.push(Project {
            id: "p".into(),
            name: "p".into(),
            path: "/p".into(),
            layout: serde_json::Value::Null,
        });
        assert!(set_init_command(&mut s, "p", Some(" make dev ")).unwrap());
        assert_eq!(s.init_commands["p"], "make dev");
        assert!(!set_init_command(&mut s, "p", Some("make dev")).unwrap());
        assert!(set_init_command(&mut s, "p", Some("a\nb")).is_err());
        assert!(set_init_command(&mut s, "ssh", Some("x")).is_err());
        assert!(set_init_command(&mut s, "p", Some("  ")).unwrap());
        assert!(s.init_commands.is_empty());
    }
}
