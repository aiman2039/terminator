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
        crate::perforce::context(service, &mut result, include_stats, cancel).await;
        return result;
    };
    let root = PathBuf::from(String::from_utf8_lossy(&bytes).trim());
    let root = service
        .fs()
        .run(cancel, move || anyhow::Ok(root.canonicalize()?))
        .await
        .unwrap_or_else(|_| cwd.clone());
    result.git_dirs = git.git_dirs(&root, &cwd).await;
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

    async fn git_dirs(&self, root: &Path, cwd: &Path) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        for arg in ["--absolute-git-dir", "--git-common-dir"] {
            if let Ok(bytes) = self.run(root, &["rev-parse", arg]).await {
                let path = PathBuf::from(String::from_utf8_lossy(&bytes).trim());
                dirs.push(if path.is_absolute() {
                    path
                } else {
                    cwd.join(path)
                });
            }
        }
        dirs
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
