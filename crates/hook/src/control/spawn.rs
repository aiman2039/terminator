//! `ctl spawn`: a child terminal linked to a parent session, optionally in
//! a new worktree. The agent command is typed without Enter; the caller
//! starts it with an explicit `ctl send SESSION '' --enter`.
use super::{
    SpawnArgs,
    lookup::{Ctl, NewSession},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use terminator_core::*;

/// The bytes typed for `--agent`: the command only, never Enter. A command
/// with a line break is refused, since the break would run it.
pub(super) fn agent_input(agent: Option<&str>) -> Result<Option<Vec<u8>>> {
    ensure!(
        !agent.is_some_and(|a| a.chars().any(char::is_control)),
        "--agent must be one line without control characters"
    );
    let Some(command) = agent.map(str::trim).filter(|c| !c.is_empty()) else {
        return Ok(None);
    };
    ensure!(command.len() <= 4096, "--agent is longer than 4096 bytes");
    Ok(Some(command.as_bytes().to_vec()))
}

/// Branch and folder for `--worktree NAME`, as New task names them.
pub(super) fn worktree_target(source: &Path, name: &str) -> Result<(String, PathBuf)> {
    ensure!(
        !name.is_empty()
            && name.len() <= 64
            && !name.starts_with(['-', '.'])
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')),
        "--worktree must be letters, digits, '-', '_' or '.'"
    );
    let parent = source.parent().unwrap_or(source);
    Ok((
        format!("terminator/{name}"),
        parent.join(format!("terminator-{name}")),
    ))
}

impl Ctl {
    pub(super) fn spawn(&self, args: SpawnArgs) -> Result<Value> {
        let input = agent_input(args.agent.as_deref())?;
        let parent = self.session(&args.parent)?.clone();
        ensure!(parent.lifecycle.live(), "Parent session has ended");
        let mut project = Self::project(&self.state, &parent.project_id)?;
        let mut cwd = Some(parent.cwd.clone());
        if let Some(name) = &args.worktree {
            project = self.spawn_worktree(&project, name)?;
            cwd = None;
        }
        if !args.background {
            ui_control::rpc(&self.paths, ui_control::Request::Ping)?;
        }
        let child = self.create_session(NewSession {
            project: &project,
            cwd,
            file: None,
        })?;
        let linked = self.link_parent(&child.id, &parent.id)?;
        let typed = input.is_some();
        if let Some(bytes) = input {
            // The child is newer than this control's snapshot.
            let fresh = Ctl {
                paths: self.paths.clone(),
                state: Self::snapshot(&self.paths)?,
            };
            fresh.attach_and_input(child.id.clone(), bytes)?;
        }
        if !args.background {
            ui_control::rpc(
                &self.paths,
                ui_control::Request::ShowSession {
                    session: child.id.clone(),
                    anchor: None,
                    split: None,
                },
            )
            .with_context(|| format!("Session {} is running, but not shown", child.id))?;
        }
        Ok(json!({
            "session": child.id,
            "parent": parent.id,
            "project": project.id,
            "parent_linked": linked,
            "typed": typed,
            "enter": false,
        }))
    }

    /// Links the child when the daemon supports it; older daemons keep no
    /// link and the child still runs.
    fn link_parent(&self, child: &str, parent: &str) -> Result<bool> {
        if self.capability(session_parents::CAPABILITY).is_err() {
            return Ok(false);
        }
        rpc(
            &self.paths,
            Request::SetSessionParent {
                session: child.to_owned(),
                parent: parent.to_owned(),
            },
        )?
        .checked()?;
        Ok(true)
    }

    /// Adds `terminator/NAME` beside the project and returns its project.
    fn spawn_worktree(&self, source: &Project, name: &str) -> Result<Project> {
        self.capability(WORKTREES_CAPABILITY)?;
        ensure!(
            !source.path.starts_with("/__terminator_remote__"),
            "--worktree needs a local project"
        );
        let (branch, path) = worktree_target(&source.path, name)?;
        rpc(
            &self.paths,
            Request::WorktreeAdd {
                project: source.id.clone(),
                path: path.clone(),
                branch: Some(branch),
                start: "HEAD".to_owned(),
            },
        )?
        .checked()?;
        let canonical = path.canonicalize()?;
        let state = Self::snapshot(&self.paths)?;
        state
            .projects
            .iter()
            .find(|p| p.path == canonical)
            .cloned()
            .context("Created worktree missing from inventory")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctl_spawn_never_presses_enter() {
        assert_eq!(
            agent_input(Some("  claude --model x ")).unwrap(),
            Some(b"claude --model x".to_vec())
        );
        for bad in ["claude\n", "a\rb", "x\u{3}"] {
            assert!(agent_input(Some(bad)).is_err(), "{bad:?}");
        }
        assert_eq!(agent_input(None).unwrap(), None);
        assert_eq!(agent_input(Some("  ")).unwrap(), None);
    }

    #[test]
    fn ctl_spawn_has_no_enter_flag() {
        let argv: Vec<String> = ["spawn", "--parent", "s", "--agent", "claude", "--enter"]
            .iter()
            .map(|w| (*w).to_owned())
            .collect();
        assert!(super::super::parse(&argv).is_err());
    }

    #[test]
    fn spawn_worktree_names_follow_new_task() {
        let (branch, path) = worktree_target(Path::new("/src/app"), "fix-1").unwrap();
        assert_eq!(branch, "terminator/fix-1");
        assert_eq!(path, PathBuf::from("/src/terminator-fix-1"));
        for bad in ["", "-x", ".x", "a/b", "a b", "../x"] {
            assert!(
                worktree_target(Path::new("/src/app"), bad).is_err(),
                "{bad}"
            );
        }
    }
}
