use anyhow::{Context, Result, ensure};
use std::path::PathBuf;
use terminator_core::*;

/// Inputs for creating a session in a new tab.
pub(super) struct NewSession<'a> {
    pub(super) project: &'a Project,
    pub(super) cwd: Option<PathBuf>,
    pub(super) file: Option<PathBuf>,
}
/// Workspace paths plus the daemon snapshot every control reads from.
pub(super) struct Ctl {
    pub(super) paths: Paths,
    pub(super) state: State,
}
impl Ctl {
    pub(super) fn connect(paths: &Paths) -> Result<Self> {
        let paths = generations::workspace_paths(paths)?;
        let state = Self::snapshot(&paths)?;
        Ok(Self { paths, state })
    }
    pub(super) fn snapshot(paths: &Paths) -> Result<State> {
        match rpc(paths, Request::Snapshot)?.checked()? {
            Response::State(state) => Ok(*state),
            _ => anyhow::bail!("Expected daemon snapshot"),
        }
    }
    pub(super) fn capability(&self, feature: &str) -> Result<()> {
        ensure!(
            self.state.capabilities.iter().any(|c| c == feature),
            "Running daemon lacks {feature}; existing live sessions were left running"
        );
        Ok(())
    }
    pub(super) fn project(state: &State, name: &str) -> Result<Project> {
        let mut matches = state
            .projects
            .iter()
            .filter(|p| p.id == name || p.path.to_string_lossy() == name || p.name == name)
            .cloned()
            .collect::<Vec<_>>();
        matches.extend(
            state
                .remote_projects
                .iter()
                .filter(|p| p.id == name || p.name == name || p.label() == name)
                .map(|p| Project {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    path: p.identity_path(),
                    layout: p.layout.clone(),
                }),
        );
        match matches.as_slice() {
            [project] => Ok(project.clone()),
            _ => anyhow::bail!("Project must match exactly one ID, location, or name"),
        }
    }
    pub(super) fn session(&self, sid: &str) -> Result<&Session> {
        self.state
            .sessions
            .iter()
            .find(|s| s.id == sid)
            .context("Unknown session ID")
    }
    pub(super) fn create_session(&self, new: NewSession<'_>) -> Result<Session> {
        let NewSession { project, cwd, file } = new;
        let file = if project.path.starts_with("/__terminator_remote__") {
            file
        } else {
            file.map(std::path::absolute).transpose()?
        };
        match rpc(
            &self.paths,
            Request::Create {
                project: project.id.clone(),
                cwd,
                file: file.clone(),
                line: None,
                column: None,
                editor: file.is_some(),
            },
        )?
        .checked()?
        {
            Response::Created(s) => Ok(s),
            _ => anyhow::bail!("Daemon did not create session"),
        }
    }
}
