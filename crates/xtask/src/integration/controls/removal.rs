use super::super::super::harness::{git, id, output, session, session_present};
use super::super::run::symlink;
use super::{CHILD_SIGNAL, CHILD_STOPPED_SIGNAL, CROSS_PROJECT_SIGNAL, Checkout, shell_quote};
use anyhow::{Context, Result, anyhow, ensure};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};
use terminator_core::transport;

impl Checkout {
    pub(super) fn dirty_refusal(&self) -> Result<()> {
        let Self {
            h, destination, p, ..
        } = self;
        fs::write(destination.join("dirty"), "keep")?;
        ensure!(
            h.rpc(json!({"WorktreeRemove":{"project":id(p)}})).is_err(),
            "Dirty checkout was removed"
        );
        fs::remove_file(destination.join("dirty"))?;
        Ok(())
    }
    /// A shell from the parent project enters the checkout through an alias;
    /// returns that shell, its stream and the alias path.
    pub(super) fn cross_project_refusal(&self) -> Result<(Value, transport::Stream, PathBuf)> {
        let Self {
            h,
            project,
            destination,
            p,
            ..
        } = self;
        let outside = h.shell(project)?;
        let mut outside_stream = h.attach(&outside)?;
        let alias = h.root.join("checkout-alias");
        symlink(destination, &alias, true)?;
        h.write(
            &mut outside_stream,
            &format!(
                "stty -echo; cd {} && {CROSS_PROJECT_SIGNAL}\n",
                shell_quote(&alias.to_string_lossy())
            ),
        )?;
        h.wait(
            |_| {
                h.history(id(&outside))
                    .is_ok_and(|s| s.contains("CROSS_PROJECT_READY"))
            },
            5,
        )?;
        ensure!(
            h.state().is_ok_and(|state| {
                project
                    .get("path")
                    .is_some_and(|path| &session(&state, id(&outside))["cwd"] == path)
            }),
            "Fixture unexpectedly reported cwd"
        );
        let error = h
            .rpc(json!({"WorktreeRemove":{"project":id(p)}}))
            .expect_err("Removed checkout with another project's process inside");
        ensure!(
            error.to_string().contains("Stop processes using"),
            "Wrong cwd refusal: {error:#}"
        );
        ensure!(
            destination.join("file").is_file() && Self::worktree_removed(&h.state()?, false),
            "Refusal changed checkout or registry"
        );
        Ok((outside, outside_stream, alias))
    }
    pub(super) fn descendant_refusal(
        &self,
        outside: &Value,
        outside_stream: &mut transport::Stream,
    ) -> Result<()> {
        let Self {
            h,
            repo,
            destination,
            p,
            ..
        } = self;
        h.write(
            outside_stream,
            &format!(
                "cd {} && {{ (cd {} && {CHILD_SIGNAL} && exec sleep 30) & }}\n",
                shell_quote(&repo.to_string_lossy()),
                shell_quote(&destination.to_string_lossy())
            ),
        )?;
        h.wait(
            |_| {
                h.history(id(outside))
                    .is_ok_and(|s| s.contains("CHILD_READY"))
            },
            5,
        )?;
        let error = h
            .rpc(json!({"WorktreeRemove":{"project":id(p)}}))
            .expect_err("Removed checkout with a live descendant inside");
        ensure!(
            error.to_string().contains("Stop processes using"),
            "Wrong descendant refusal: {error:#}"
        );
        h.write(
            outside_stream,
            &format!("kill %1; wait; {CHILD_STOPPED_SIGNAL}\n"),
        )?;
        h.wait(
            |_| {
                h.history(id(outside))
                    .is_ok_and(|s| s.contains("CHILD_STOPPED"))
            },
            5,
        )
        .with_context(|| format!("Child cleanup history: {:?}", h.history(id(outside))))?;
        h.assert_pids(std::slice::from_ref(outside))
    }
    pub(super) fn recorded_cwd_refusal(&self, alias: &PathBuf) -> Result<()> {
        let Self { h, project, p, .. } = self;
        let recorded = h
            .rpc(json!({"Create":{"project":id(project),"cwd":alias,"file":null,"line":null,"editor":false}}))?
            .get("Created")
            .cloned()
            .ok_or_else(|| anyhow!("missing Created"))?;
        ensure!(
            h.rpc(json!({"WorktreeRemove":{"project":id(p)}})).is_err(),
            "Removed checkout used by another project's recorded cwd"
        );
        h.rpc(json!({"Stop":{"session":id(&recorded)}}))?;
        h.wait(|st| !session_present(st, id(&recorded)), 5)?;
        Ok(())
    }
    /// A Git-locked checkout must be refused; returns the branch commit.
    pub(super) fn locked_refusal(&self, s: &Value) -> Result<Vec<u8>> {
        let Self {
            h,
            repo,
            destination,
            p,
            ..
        } = self;
        let branch = git(repo, &["rev-parse", "refs/heads/fixture-task"])?;
        ensure!(
            git(destination, &["status", "--porcelain"])?.is_empty(),
            "Lock fixture is dirty"
        );
        git(
            repo,
            &[
                "worktree",
                "lock",
                "--reason",
                "fixture lock",
                destination.to_str().unwrap(),
            ],
        )?;
        let mut locked_remove = h.command("terminator-hook");
        locked_remove.args(["ctl", "worktree", "remove", id(p)]);
        let error = output(locked_remove).expect_err("Locked checkout was removed");
        ensure!(
            format!("{error:#}").contains("locked"),
            "Wrong refusal: {error:#}"
        );
        ensure!(
            fs::read(destination.join("file"))? == b"base\n",
            "Locked tracked file changed"
        );
        let common = terminator_core::worktrees::common_dir(repo)?;
        ensure!(
            terminator_core::worktrees::list(&common)?
                .iter()
                .any(|w| w.path == *destination && w.locked),
            "Git registration or lock lost"
        );
        ensure!(
            git(repo, &["rev-parse", "refs/heads/fixture-task"])? == branch,
            "Locked branch reference changed"
        );
        let state = h.state()?;
        ensure!(
            Self::worktree_removed(&state, false),
            "Refusal marked checkout removed"
        );
        ensure!(
            !session_present(&state, id(s)),
            "Lock refusal depended on a live session"
        );
        Ok(branch)
    }
    pub(super) fn remove(&self, outside: &Value, branch: &[u8]) -> Result<()> {
        let Self {
            h,
            repo,
            destination,
            p,
            ..
        } = self;
        git(repo, &["worktree", "unlock", destination.to_str().unwrap()])?;
        let mut remove = h.command("terminator-hook");
        remove.args(["ctl", "worktree", "remove", id(p)]);
        output(remove)?;
        h.assert_pids(std::slice::from_ref(outside))?;
        ensure!(
            !destination.exists() && Self::worktree_removed(&h.state()?, true),
            "Worktree removal not recorded"
        );
        ensure!(
            git(repo, &["rev-parse", "refs/heads/fixture-task"])? == branch,
            "Removal changed branch reference"
        );
        ensure!(
            h.state()?
                .get("selected_project")
                .is_some_and(|selected| selected != id(p)),
            "Removed worktree remains selected"
        );
        ensure!(
            h.rpc(json!({"SelectProject":{"project":id(p)}})).is_err(),
            "Removed worktree can be selected"
        );
        ensure!(
            h.rpc(json!({"Create":{"project":id(p),"cwd":repo,"editor":false}}))
                .is_err(),
            "Removed worktree can create a session in another directory"
        );
        Ok(())
    }
    // Explicitly opening a restored folder must not inherit the removal marker.
    pub(super) fn restore(&self) -> Result<()> {
        let Self {
            h, destination, p, ..
        } = self;
        fs::create_dir(destination)?;
        h.rpc(json!({"AddProject":{"path":destination}}))?;
        ensure!(
            h.state()?
                .get("selected_project")
                .is_some_and(|selected| selected == id(p)),
            "Restored project was not selected"
        );
        let restored = h.shell(p)?;
        ensure!(
            restored
                .get("cwd")
                .is_some_and(|cwd| cwd == destination.to_string_lossy().as_ref()),
            "Restored shell used a stale folder"
        );
        Ok(())
    }
}
