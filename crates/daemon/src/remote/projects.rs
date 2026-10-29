//! Editing saved SSH projects and recording when they were last used.
use super::{control_in_use, stop_control};
use crate::server::{Shared, relock};
use anyhow::{Context, Result, ensure};
use terminator_core::{State, remote::RemoteProject, ssh::SshOptions};

/// Replaces the saved project with `project.id`, keeping its layout,
/// forwards and last-connected time. `idle` runs only when the connection
/// or root changes. Returns the old SSH options when the connection moved,
/// so the caller can stop the old master.
fn replace_project(
    state: &mut State,
    mut project: RemoteProject,
    idle: impl FnOnce(&State) -> Result<()>,
) -> Result<Option<SshOptions>> {
    let old = state
        .remote_projects
        .iter()
        .find(|p| p.id == project.id)
        .cloned()
        .context("Unknown SSH project")?;
    ensure!(
        !state
            .remote_projects
            .iter()
            .any(|p| p.id != project.id && p.same_connection(&project)),
        "Another SSH project already uses this host and path"
    );
    let moved = !old.same_connection(&project);
    if moved {
        idle(state)?;
    }
    project.layout = old.layout;
    project.forwards = old.forwards;
    project.last_connected = old.last_connected;
    if let Some(slot) = state
        .remote_projects
        .iter_mut()
        .find(|p| p.id == project.id)
    {
        *slot = project;
    }
    state.revision = state.revision.saturating_add(1);
    Ok(moved.then_some(old.ssh))
}

impl Shared {
    /// Edits a saved project. A new connection or root is refused while
    /// anything still uses the old one; afterwards the old OpenSSH master
    /// stops unless something else shares it.
    pub(crate) fn update_remote_project(&self, project: RemoteProject) -> Result<()> {
        project.validate()?;
        let control = {
            let mut state = relock(&self.state);
            let id = project.id.clone();
            let old = replace_project(&mut state, project, |state| {
                self.ensure_project_idle(state, &id, "changing its connection")
            })?;
            old.and_then(|old| self.ssh_control(&old))
                .filter(|control| !control_in_use(&state, control, |ssh| self.ssh_control(ssh)))
        };
        if let Some(control) = control {
            stop_control(&control);
        }
        Ok(())
    }

    /// Refuses while a terminal, editor, VNC session, transfer or active
    /// forward still belongs to `id`. Takes the forwards lock after
    /// `state`, the established order.
    pub(crate) fn ensure_project_idle(&self, state: &State, id: &str, action: &str) -> Result<()> {
        ensure!(
            !state
                .sessions
                .iter()
                .any(|s| s.project_id == id && s.lifecycle.live())
                && !state
                    .vnc_sessions
                    .iter()
                    .any(|s| s.project_id == id && s.status.live()),
            "Close this SSH project's terminals and VNC sessions before {action}"
        );
        ensure!(
            relock(&self.remote_transfers).active(id) == 0,
            "Wait for this SSH project's file transfers to finish before {action}"
        );
        ensure!(
            !relock(&self.forwards)
                .status()
                .active
                .iter()
                .any(|a| a.project == id),
            "Stop this SSH project's port forwards before {action}"
        );
        Ok(())
    }

    /// Records that an SSH terminal opened for `id`.
    pub(crate) fn touch_remote_project(&self, id: &str) {
        let mut state = relock(&self.state);
        if let Some(project) = state.remote_projects.iter_mut().find(|p| p.id == id) {
            project.last_connected = terminator_core::now();
            state.revision = state.revision.saturating_add(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::bail;

    fn project(id: &str, host: &str, root: &str) -> RemoteProject {
        RemoteProject {
            id: id.into(),
            name: id.into(),
            ssh: SshOptions {
                host: host.into(),
                ..SshOptions::default()
            },
            root: root.into(),
            layout: serde_json::json!({"version": 6}),
            group: None,
            forwards: Vec::new(),
            last_connected: 42,
        }
    }

    fn state() -> State {
        State {
            remote_projects: vec![project("a", "alpha", "/a"), project("b", "beta", "/b")],
            ..State::default()
        }
    }

    #[test]
    fn renaming_keeps_layout_and_last_connected_without_an_idle_check() {
        let mut state = state();
        let mut edit = project("a", "alpha", "/a");
        edit.name = "Renamed".into();
        edit.group = Some("Lab".into());
        edit.layout = serde_json::Value::Null;
        edit.last_connected = 0;
        let moved = replace_project(&mut state, edit, |_| bail!("busy")).unwrap();
        assert!(moved.is_none());
        let saved = &state.remote_projects[0];
        assert_eq!(saved.name, "Renamed");
        assert_eq!(saved.group.as_deref(), Some("Lab"));
        assert_eq!(saved.layout, serde_json::json!({"version": 6}));
        assert_eq!(saved.last_connected, 42);
    }

    #[test]
    fn changing_the_host_requires_an_idle_project_and_returns_the_old_ssh() {
        let mut state = state();
        let edit = project("a", "gamma", "/a");
        assert!(replace_project(&mut state, edit.clone(), |_| bail!("busy")).is_err());
        assert_eq!(state.remote_projects[0].ssh.host, "alpha");
        let old = replace_project(&mut state, edit, |_| Ok(())).unwrap();
        assert_eq!(old.unwrap().host, "alpha");
        assert_eq!(state.remote_projects[0].ssh.host, "gamma");
    }

    #[test]
    fn an_edit_may_not_duplicate_another_project_or_name_an_unknown_one() {
        let mut state = state();
        assert!(replace_project(&mut state, project("a", "beta", "/b"), |_| Ok(())).is_err());
        assert!(replace_project(&mut state, project("z", "zeta", "/z"), |_| Ok(())).is_err());
    }
}
