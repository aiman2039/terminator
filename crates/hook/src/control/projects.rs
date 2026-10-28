use super::{AddRemoteProjectArgs, ShutdownArgs, Worktree, lookup::Ctl};
use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};
use terminator_core::*;

impl Ctl {
    pub(super) fn shutdown(self, args: ShutdownArgs) -> Result<Value> {
        let ShutdownArgs {
            stop_all,
            timeout,
            relaunch,
            exe,
            confirmed_inventory,
        } = args;
        let relaunch = if relaunch {
            Some(match exe {
                Some(path) => path,
                None => std::env::current_exe()?.with_file_name("terminator"),
            })
        } else {
            None
        };
        crate::shutdown::run(
            &self.paths,
            self.state,
            crate::shutdown::Options {
                stop_all,
                confirmed: confirmed_inventory
                    .map(|json| serde_json::from_str(&json))
                    .transpose()?,
                timeout: Duration::from_secs(timeout),
                relaunch,
            },
        )
    }
    pub(super) fn list(&self) -> Value {
        let state = &self.state;
        json!({"generation":state.generation,"projects":state.projects,"remote_projects":state.remote_projects,"sessions":state.sessions,"worktrees":state.worktrees,"agents":state.agents,"terminal_notices":state.terminal_notices})
    }
    pub(super) fn add_remote_project(&self, args: AddRemoteProjectArgs) -> Result<Value> {
        let AddRemoteProjectArgs {
            host,
            port,
            user,
            root,
            name,
            identity,
            known_hosts,
        } = args;
        self.capability(remote::CAPABILITY)?;
        let project = remote::RemoteProject {
            id: String::new(),
            name,
            root,
            ssh: ssh::SshOptions {
                host,
                port,
                username: user,
                identity_file: identity,
                known_hosts_file: known_hosts,
            },
            layout: serde_json::Value::Null,
            group: None,
            forwards: Vec::new(),
        };
        project.validate()?;
        rpc(
            &self.paths,
            Request::AddRemoteProject {
                project: project.clone(),
            },
        )?
        .checked()?;
        let state = Self::snapshot(&self.paths)?;
        Ok(serde_json::to_value(
            state
                .remote_projects
                .iter()
                .find(|p| p.ssh == project.ssh && p.root == project.root)
                .context("Registered SSH project missing")?,
        )?)
    }
    pub(super) fn add_project(&self, path: PathBuf) -> Result<Value> {
        let path = std::path::absolute(path)?;
        rpc(&self.paths, Request::AddProject { path: path.clone() })?.checked()?;
        let state = Self::snapshot(&self.paths)?;
        Ok(serde_json::to_value(Self::project(
            &state,
            &path.canonicalize()?.to_string_lossy(),
        )?)?)
    }
    pub(super) fn worktree(&self, command: Worktree) -> Result<Value> {
        self.capability(WORKTREES_CAPABILITY)?;
        let request = self.worktree_request(command)?;
        let response = rpc(&self.paths, request)?.checked()?;
        Ok(serde_json::to_value(response)?)
    }
    fn worktree_request(&self, command: Worktree) -> Result<Request> {
        Ok(match command {
            Worktree::List { project: name } => Request::WorktreeList {
                project: Self::project(&self.state, &name)?.id.clone(),
            },
            Worktree::Add {
                project: name,
                path,
                branch,
                start,
            } => Request::WorktreeAdd {
                project: Self::project(&self.state, &name)?.id.clone(),
                path: std::path::absolute(path)?,
                branch,
                start,
            },
            Worktree::Remove { project: name } => Request::WorktreeRemove {
                project: Self::project(&self.state, &name)?.id.clone(),
            },
        })
    }
}
