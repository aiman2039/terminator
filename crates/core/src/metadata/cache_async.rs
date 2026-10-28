//! Metadata refreshed through the async process pool and the file pool.
use super::*;
use std::ffi::OsString;

pub struct CollectAsyncInput<'a> {
    pub cwd: &'a Path,
    pub identity: Option<(u32, u64)>,
    pub include_pr: bool,
}

impl Cache {
    pub async fn collect_async(
        &mut self,
        processes: &crate::async_process::Processes,
        files: &crate::async_service::NativePool,
        input: CollectAsyncInput<'_>,
    ) -> Metadata {
        let CollectAsyncInput {
            cwd,
            identity,
            include_pr,
        } = input;
        let changed = self.enter(cwd);
        if self.git_due(changed) {
            let previous = self.data.branch.clone();
            self.data.root = git_async(GitAsyncInput {
                processes,
                files,
                cwd,
                args: &["rev-parse", "--show-toplevel"],
            })
            .await
            .ok()
            .map(PathBuf::from);
            self.data.branch = git_async(GitAsyncInput {
                processes,
                files,
                cwd,
                args: &["branch", "--show-current"],
            })
            .await
            .ok()
            .filter(|s| !s.is_empty());
            let root = self.data.root.clone();
            self.data.worktree = files
                .run(&crate::async_service::CancellationToken::new(), move || {
                    Ok::<_, crate::async_service::Failure>(
                        root.as_ref().is_some_and(|p| p.join(".git").is_file()),
                    )
                })
                .await
                .unwrap_or(false);
            self.branch_checked(previous);
        }
        if self.pr_due(include_pr, changed) {
            let pr = pull_request_async(processes, cwd).await;
            self.store_pr(pr);
        }
        self.finish_pr(include_pr);
        let ports = ports_async(processes, files, identity).await;
        self.store_ports(ports)
    }
}

struct GitAsyncInput<'a> {
    processes: &'a crate::async_process::Processes,
    files: &'a crate::async_service::NativePool,
    cwd: &'a Path,
    args: &'a [&'a str],
}

async fn git_async(input: GitAsyncInput<'_>) -> Result<String> {
    let GitAsyncInput {
        processes,
        files,
        cwd,
        args,
    } = input;
    let args = args.iter().map(OsString::from).collect();
    let output = crate::git::run(
        processes,
        files,
        crate::git::RunInput {
            cwd,
            args,
            options: CommandOptions {
                timeout: Duration::from_secs(3),
                stdout_limit: 65536,
                ..Default::default()
            },
        },
    )
    .await?;
    Ok(String::from_utf8(output)?.trim_end().into())
}
async fn pull_request_async(
    processes: &crate::async_process::Processes,
    cwd: &Path,
) -> Result<Option<PullRequest>> {
    let mut command = Command::new("gh");
    command
        .current_dir(cwd)
        .args(["pr", "view", "--json", "number,title,url,state"])
        .env("GH_PROMPT_DISABLED", "1");
    let output = processes
        .run(
            command,
            CommandOptions {
                timeout: Duration::from_secs(4),
                stdout_limit: 65536,
                stderr_limit: 8192,
                ..Default::default()
            },
            None,
        )
        .await?;
    decode_pull_request(&output.stdout)
}
async fn ports_async(
    processes: &crate::async_process::Processes,
    files: &crate::async_service::NativePool,
    identity: Option<(u32, u64)>,
) -> Result<Option<Vec<ListeningPort>>> {
    let Some(identity) = identity else {
        return Ok(None);
    };
    let owned = files
        .run(&crate::async_service::CancellationToken::new(), move || {
            owned_processes(identity)
        })
        .await?;
    if owned.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let pids = owned
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let mut command = Command::new("lsof");
    command.args(["-nP", "-a", "-p", &pids, "-iTCP", "-sTCP:LISTEN", "-Fpn"]);
    let output = processes
        .run(
            command,
            CommandOptions {
                timeout: Duration::from_secs(3),
                stdout_limit: 256 * 1024,
                accepted_exit_codes: Some(vec![0, 1]),
                ..Default::default()
            },
            None,
        )
        .await?;
    Ok(Some(parse_ports(
        &String::from_utf8_lossy(&output.stdout),
        &owned,
    )))
}
