use anyhow::{Context, Result, ensure};
use std::sync::Arc;
use terminator_core::*;

use super::super::super::shared::{Shared, relock};
use super::Outcome;

impl Shared {
    pub(super) fn on_probe_remote_project(
        &self,
        project: &remote::RemoteProject,
    ) -> Result<Outcome> {
        Ok(Outcome::Reply(Response::RemoteProject(
            self.probe_remote_project(project)?,
        )))
    }

    pub(super) fn on_scan_host_key(&self, ssh: &ssh::SshOptions) -> Result<Outcome> {
        Ok(Outcome::Reply(Response::HostKeys(self.scan_host_key(ssh)?)))
    }

    pub(super) fn on_trust_host_key(
        &self,
        ssh: &ssh::SshOptions,
        keys: &[ssh::HostKey],
    ) -> Result<Outcome> {
        self.trust_host_key(ssh, keys)?;
        Ok(Outcome::Reply(Response::Ok))
    }

    pub(super) fn on_remote_project(
        self: &Arc<Self>,
        project: &str,
        operation: remote::Operation,
    ) -> Result<Outcome> {
        Ok(Outcome::Reply(Response::RemoteProject(
            self.remote_project_operation(project, operation)?,
        )))
    }

    pub(super) fn on_add_remote_project(
        &self,
        mut project: remote::RemoteProject,
    ) -> Result<Outcome> {
        project.validate()?;
        let mut state = relock(&self.state);
        ensure!(state.remote_projects.len() < 256, "Too many SSH projects");
        if let Some(existing) = state
            .remote_projects
            .iter()
            .find(|p| p.ssh == project.ssh && p.root == project.root)
        {
            state.selected_project = Some(existing.id.clone());
        } else {
            project.id = id();
            project.layout = serde_json::Value::Null;
            state.selected_project = Some(project.id.clone());
            state.remote_projects.push(project);
        }
        state.revision = state.revision.saturating_add(1);
        Ok(Outcome::Changed)
    }

    pub(super) fn on_remove_remote_project(&self, project: &str) -> Result<Outcome> {
        self.remove_remote_project(project)?;
        Ok(Outcome::Changed)
    }

    pub(super) fn on_set_remote_project_group(
        &self,
        project: &str,
        group: Option<&str>,
    ) -> Result<Outcome> {
        let group = remote::normalize_group(group)?;
        let mut state = relock(&self.state);
        state
            .remote_projects
            .iter_mut()
            .find(|p| p.id == project)
            .context("Unknown SSH project")?
            .group = group;
        state.revision = state.revision.saturating_add(1);
        Ok(Outcome::Changed)
    }

    pub(super) fn on_set_remote_project_forwards(
        &self,
        project: &str,
        forwards: Vec<remote::Forward>,
    ) -> Result<Outcome> {
        remote::validate_forwards(&forwards)?;
        let mut state = relock(&self.state);
        let target = state
            .remote_projects
            .iter_mut()
            .find(|p| p.id == project)
            .context("Unknown SSH project")?;
        relock(&self.forwards).redefine(project, &forwards)?;
        target.forwards = forwards;
        state.revision = state.revision.saturating_add(1);
        Ok(Outcome::Changed)
    }

    pub(super) fn on_start_forward(&self, project: &str, local_port: u16) -> Result<Outcome> {
        Ok(Outcome::Reply(Response::Forwards(
            self.start_forward(project, local_port)?,
        )))
    }

    pub(super) fn on_stop_forward(&self, project: &str, local_port: u16) -> Result<Outcome> {
        Ok(Outcome::Reply(Response::Forwards(
            self.stop_forward(project, local_port)?,
        )))
    }

    pub(super) fn on_forward_status(&self) -> Outcome {
        Outcome::Reply(Response::Forwards(self.forward_status()))
    }
}
