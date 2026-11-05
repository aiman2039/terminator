//! Async repository context for the visible project: root, Git dirs,
//! branch, status, and per-file line counts.
use super::{ContextData, MergeStatsCancelInput, merge_stats_cancel};
use crate::gui_services::Services;
use std::{ffi::OsString, path::Path, path::PathBuf};
use terminator_core::{CommandOptions, async_service::CancellationToken};

pub struct ContextAsyncInput<'a> {
    pub service: &'a Services,
    pub cwd: PathBuf,
    pub include_stats: bool,
    pub cancel: &'a CancellationToken,
}

pub async fn context_async(input: ContextAsyncInput<'_>) -> ContextData {
    let ContextAsyncInput {
        service,
        cwd,
        include_stats,
        cancel,
    } = input;
    let mut result = ContextData {
        cwd: cwd.clone(),
        root: None,
        perforce: None,
        git_dirs: vec![],
        branch: String::new(),
        changes: vec![],
        decorations: std::collections::HashMap::default(),
        stats: std::collections::HashMap::default(),
        error: None,
    };
    let git = GitRunner { service };
    let Ok(bytes) = git.run(&cwd, &["rev-parse", "--show-toplevel"]).await else {
        crate::perforce::context(crate::perforce::ContextInput {
            service,
            result: &mut result,
            include_status: include_stats,
            cancel,
        })
        .await;
        return result;
    };
    let root = PathBuf::from(String::from_utf8_lossy(&bytes).trim());
    let root = service
        .fs()
        .run(cancel, move || anyhow::Ok(root.canonicalize()?))
        .await
        .unwrap_or_else(|_| cwd.clone());
    result.git_dirs = git.git_dirs(&root, cancel).await;
    result.root = Some(root.clone());
    if !include_stats {
        return result;
    }
    result.branch = git.branch(&root).await;
    match git
        .run(
            &root,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )
        .await
    {
        Ok(raw) => {
            result.changes = terminator_git::parse_porcelain(&root, &raw);
        }
        Err(error) => result.error = Some(error.to_string()),
    }
    result.decorations = terminator_git::decorations(&root, &result.changes);
    if !result.changes.is_empty() {
        result.stats = git.line_stats(root, &result.changes, cancel).await;
    }
    result
}

/// Runs bounded Git commands through the GUI service pools.
struct GitRunner<'a> {
    service: &'a Services,
}

impl GitRunner<'_> {
    async fn run(
        &self,
        root: &Path,
        args: &[&str],
    ) -> Result<Vec<u8>, terminator_core::async_process::AsyncProcessError> {
        terminator_git::run(
            self.service.processes(),
            self.service.fs(),
            terminator_git::RunInput {
                cwd: root,
                args: args.iter().map(OsString::from).collect(),
                options: CommandOptions {
                    stdout_limit: 4 * 1024 * 1024,
                    ..Default::default()
                },
            },
        )
        .await
    }

    /// The Git dir and the common dir, canonical: a main worktree prints
    /// `.git` (a symlink in some layouts) where a linked worktree prints
    /// the resolved path, and a running commit compares the two.
    async fn git_dirs(&self, root: &Path, cancel: &CancellationToken) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        for arg in ["--absolute-git-dir", "--git-common-dir"] {
            if let Ok(bytes) = self.run(root, &["rev-parse", arg]).await {
                let path = PathBuf::from(String::from_utf8_lossy(&bytes).trim());
                dirs.push(if path.is_absolute() {
                    path
                } else {
                    root.join(path)
                });
            }
        }
        let printed = dirs.clone();
        self.service
            .fs()
            .run(cancel, move || {
                anyhow::Ok(
                    dirs.into_iter()
                        .map(|dir| dir.canonicalize().unwrap_or(dir))
                        .collect(),
                )
            })
            .await
            .unwrap_or(printed)
    }

    async fn branch(&self, root: &Path) -> String {
        let branch = self
            .run(root, &["branch", "--show-current"])
            .await
            .map(|b| String::from_utf8_lossy(&b).trim().to_owned())
            .unwrap_or_default();
        if branch.is_empty() {
            "Detached HEAD".into()
        } else {
            branch
        }
    }

    async fn line_stats(
        &self,
        root: PathBuf,
        changes: &[terminator_git::Change],
        cancel: &CancellationToken,
    ) -> std::collections::HashMap<PathBuf, (u32, u32)> {
        let working = self
            .run(&root, &["diff", "--numstat", "-z"])
            .await
            .unwrap_or_default();
        let staged = self
            .run(&root, &["diff", "--cached", "--numstat", "-z"])
            .await
            .unwrap_or_default();
        let changes = changes.to_vec();
        let operation = cancel.clone();
        self.service
            .fs()
            .run(cancel, move || {
                anyhow::Ok(merge_stats_cancel(MergeStatsCancelInput {
                    root: &root,
                    working: &working,
                    staged: &staged,
                    changes: &changes,
                    cancel: Some(&operation),
                }))
            })
            .await
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui_services::Services;

    #[tokio::test]
    async fn git_dirs_resolve_against_repo_root_from_subfolder() {
        let repo = tempfile::tempdir().unwrap();
        let root = repo.path().canonicalize().unwrap();
        let status = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&root)
            .status()
            .unwrap();
        assert!(status.success());
        let sub = root.join("src");
        std::fs::create_dir(&sub).unwrap();
        let state = tempfile::tempdir().unwrap();
        let (tx, _) = std::sync::mpsc::channel();
        let (service, _owner) = Services::new(
            terminator_core::Paths::at(state.path().into()),
            eframe::egui::Context::default(),
            tx,
        )
        .unwrap();
        let cancel = CancellationToken::new();
        let context = context_async(ContextAsyncInput {
            service: &service,
            cwd: sub,
            include_stats: false,
            cancel: &cancel,
        })
        .await;
        assert!(!context.git_dirs.is_empty());
        for dir in &context.git_dirs {
            assert_eq!(dir.canonicalize().unwrap(), root.join(".git"));
        }
    }

    /// Runs `git` in `cwd` with no hooks.
    #[cfg(unix)]
    fn git(cwd: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(["-c", "core.hooksPath=/dev/null"])
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    /// Every worktree of a repository shares the stash; a running commit
    /// matches their common dir, so both name it the same way.
    #[cfg(unix)]
    #[tokio::test]
    async fn every_worktree_names_its_common_dir_the_same_way() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let (main, linked) = (base.join("main"), base.join("linked"));
        std::fs::create_dir(&main).unwrap();
        git(&main, &["init", "-q", "-b", "main"]);
        // Some tools keep `.git` elsewhere and link it.
        let store = base.join("store.git");
        std::fs::rename(main.join(".git"), &store).unwrap();
        std::os::unix::fs::symlink(&store, main.join(".git")).unwrap();
        let author = ["-c", "user.email=w@example.invalid", "-c", "user.name=W"];
        let commit = ["commit", "-q", "--allow-empty", "-m", "base"];
        git(&main, &[author.as_slice(), commit.as_slice()].concat());
        let target = linked.to_string_lossy().into_owned();
        git(&main, &["worktree", "add", "-q", "-b", "side", &target]);
        let state = tempfile::tempdir().unwrap();
        let (tx, _) = std::sync::mpsc::channel();
        let (service, _owner) = Services::new(
            terminator_core::Paths::at(state.path().into()),
            eframe::egui::Context::default(),
            tx,
        )
        .unwrap();
        async fn common_dir(service: &Services, cwd: PathBuf) -> Option<PathBuf> {
            let input = ContextAsyncInput {
                service,
                cwd,
                include_stats: false,
                cancel: &CancellationToken::new(),
            };
            context_async(input).await.git_dirs.get(1).cloned()
        }
        assert_eq!(common_dir(&service, main).await, Some(store.clone()));
        assert_eq!(common_dir(&service, linked).await, Some(store));
    }
}
