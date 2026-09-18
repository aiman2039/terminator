//! Read-only context discovery. Git and gh own repository/provider semantics;
//! sysinfo scopes lsof results to descendants of the authenticated session PID.
use crate::{CommandOptions, find_executable, run_command};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ListeningPort {
    pub pid: u32,
    pub address: String,
    pub port: u16,
}
impl ListeningPort {
    pub fn url(&self) -> String {
        let host = self
            .address
            .rsplit_once(':')
            .map(|(s, _)| s)
            .unwrap_or("127.0.0.1");
        let host = if matches!(host, "*" | "0.0.0.0" | "[::]" | "::") {
            "127.0.0.1"
        } else {
            host
        };
        format!("http://{host}:{}/", self.port)
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Metadata {
    pub cwd: PathBuf,
    pub root: Option<PathBuf>,
    pub branch: Option<String>,
    pub worktree: bool,
    pub pull_request: Option<PullRequest>,
    pub ports: Vec<ListeningPort>,
    pub pr_error: Option<String>,
    pub ports_error: Option<String>,
}
fn git(cwd: &Path, args: &[&str]) -> Result<String> {
    let mut c = Command::new(find_executable("git").context("Git not found")?);
    c.arg("-C")
        .arg(cwd)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0");
    Ok(String::from_utf8(
        run_command(
            c,
            CommandOptions {
                timeout: Duration::from_secs(3),
                stdout_limit: 65536,
                ..Default::default()
            },
        )?
        .stdout,
    )?
    .trim_end()
    .into())
}
pub fn http_url(value: &str) -> Result<String> {
    let url = url::Url::parse(value).context("Invalid URL")?;
    ensure!(
        matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
        "Only HTTP(S) URLs are supported"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "URLs containing credentials are not supported"
    );
    Ok(url.into())
}
fn pull_request(cwd: &Path) -> Result<Option<PullRequest>> {
    let mut c = Command::new(find_executable("gh").context("GitHub CLI (gh) is not installed")?);
    c.current_dir(cwd)
        .args(["pr", "view", "--json", "number,title,url,state"])
        .env("GH_PROMPT_DISABLED", "1");
    let out = run_command(
        c,
        CommandOptions {
            timeout: Duration::from_secs(4),
            stdout_limit: 65536,
            stderr_limit: 8192,
            ..Default::default()
        },
    )?;
    decode_pull_request(&out.stdout)
}
fn decode_pull_request(bytes: &[u8]) -> Result<Option<PullRequest>> {
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    if value.is_null() {
        return Ok(None);
    }
    Ok(Some(PullRequest {
        number: value["number"].as_u64().context("Missing PR number")?,
        title: value["title"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .take(256)
            .collect(),
        url: http_url(value["url"].as_str().context("Missing PR URL")?)?,
        state: value["state"]
            .as_str()
            .unwrap_or_default()
            .chars()
            .take(32)
            .collect(),
    }))
}
pub fn listening_ports(identity: (u32, u64)) -> Result<Vec<ListeningPort>> {
    let owned = owned_processes(identity)?;
    let pids = owned
        .iter()
        .map(|p| p.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let mut c = Command::new(find_executable("lsof").context("lsof is not installed")?);
    c.args(["-nP", "-a", "-p", &pids, "-iTCP", "-sTCP:LISTEN", "-Fpn"]);
    let out = run_command(
        c,
        CommandOptions {
            timeout: Duration::from_secs(3),
            stdout_limit: 256 * 1024,
            accepted_exit_codes: Some(vec![0, 1]),
            ..Default::default()
        },
    )?;
    Ok(parse_ports(&String::from_utf8_lossy(&out.stdout), &owned))
}
fn owned_processes(identity: (u32, u64)) -> Result<HashSet<u32>> {
    use sysinfo::{Pid, ProcessRefreshKind, RefreshKind, System};
    let system = System::new_with_specifics(
        RefreshKind::nothing().with_processes(ProcessRefreshKind::nothing()),
    );
    let root = Pid::from_u32(identity.0);
    let Some(process) = system.process(root) else {
        return Ok(HashSet::new());
    };
    ensure!(
        process.start_time().abs_diff(identity.1) <= 2,
        "Session process identity changed"
    );
    let parents = system
        .processes()
        .iter()
        .map(|(pid, p)| (*pid, p.parent()))
        .collect::<Vec<_>>();
    let mut owned = HashSet::from([root]);
    for _ in 0..64 {
        let old = owned.len();
        for (pid, parent) in &parents {
            if parent.is_some_and(|p| owned.contains(&p)) {
                owned.insert(*pid);
            }
        }
        ensure!(
            owned.len() <= 256,
            "Too many session descendants for port discovery"
        );
        if owned.len() == old {
            break;
        }
    }
    Ok(owned.iter().map(|p| p.as_u32()).collect())
}

fn parse_ports(text: &str, owned: &HashSet<u32>) -> Vec<ListeningPort> {
    let mut pid = 0;
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for line in text.lines() {
        if let Some(value) = line.strip_prefix('p') {
            pid = value.parse().unwrap_or(0);
        }
        if let Some(address) = line.strip_prefix('n')
            && owned.contains(&pid)
            && let Some(port) = address
                .rsplit(':')
                .next()
                .and_then(|p| p.parse::<u16>().ok())
            && port > 0
            && seen.insert((pid, address.to_owned()))
            && result.len() < 64
        {
            result.push(ListeningPort {
                pid,
                address: address.chars().take(128).collect(),
                port,
            });
        }
    }
    result.sort_by_key(|p| (p.port, p.pid));
    result
}
#[derive(Clone)]
pub struct Cache {
    cwd: Option<PathBuf>,
    data: Metadata,
    git_at: Instant,
    pr_at: Instant,
    pr_enabled: bool,
}
impl Default for Cache {
    fn default() -> Self {
        Self {
            cwd: None,
            data: Metadata::default(),
            git_at: Instant::now() - Duration::from_secs(60),
            pr_at: Instant::now() - Duration::from_secs(60),
            pr_enabled: false,
        }
    }
}
impl Cache {
    pub fn collect(
        &mut self,
        cwd: &Path,
        identity: Option<(u32, u64)>,
        include_pr: bool,
    ) -> Metadata {
        let changed = self.cwd.as_deref() != Some(cwd);
        if changed {
            self.cwd = Some(cwd.into());
            self.data = Metadata {
                cwd: cwd.into(),
                ..Default::default()
            };
        }
        if changed || self.git_at.elapsed() >= Duration::from_secs(30) {
            self.git_at = Instant::now();
            let previous = self.data.branch.clone();
            self.data.root = git(cwd, &["rev-parse", "--show-toplevel"])
                .ok()
                .map(PathBuf::from);
            self.data.branch = git(cwd, &["branch", "--show-current"])
                .ok()
                .filter(|s| !s.is_empty());
            self.data.worktree = self
                .data
                .root
                .as_ref()
                .is_some_and(|p| p.join(".git").is_file());
            if self.data.branch != previous {
                self.pr_at = Instant::now() - Duration::from_secs(60);
            }
        }
        if include_pr
            && self.data.root.is_some()
            && (changed || !self.pr_enabled || self.pr_at.elapsed() >= Duration::from_secs(60))
        {
            self.pr_at = Instant::now();
            match pull_request(cwd) {
                Ok(pr) => {
                    self.data.pull_request = pr;
                    self.data.pr_error = None;
                }
                Err(error) => {
                    self.data.pull_request = None;
                    self.data.pr_error = Some(error.to_string().chars().take(256).collect());
                }
            }
        }
        if !include_pr {
            self.data.pull_request = None;
            self.data.pr_error = None;
        }
        self.pr_enabled = include_pr;
        match identity.map(listening_ports).transpose() {
            Ok(ports) => {
                self.data.ports = ports.unwrap_or_default();
                self.data.ports_error = None;
            }
            Err(error) => {
                self.data.ports.clear();
                self.data.ports_error = Some(error.to_string());
            }
        }
        self.data.clone()
    }
}
#[cfg(feature = "async-client")]
impl Cache {
    pub async fn collect_async(
        &mut self,
        processes: &crate::async_process::Processes,
        files: &crate::async_service::NativePool,
        cwd: &Path,
        identity: Option<(u32, u64)>,
        include_pr: bool,
    ) -> Metadata {
        let changed = self.cwd.as_deref() != Some(cwd);
        if changed {
            self.cwd = Some(cwd.into());
            self.data = Metadata {
                cwd: cwd.into(),
                ..Default::default()
            };
        }
        if changed || self.git_at.elapsed() >= Duration::from_secs(30) {
            self.git_at = Instant::now();
            let previous = self.data.branch.clone();
            self.data.root = git_async(processes, files, cwd, &["rev-parse", "--show-toplevel"])
                .await
                .ok()
                .map(PathBuf::from);
            self.data.branch = git_async(processes, files, cwd, &["branch", "--show-current"])
                .await
                .ok()
                .filter(|s| !s.is_empty());
            let root = self.data.root.clone();
            self.data.worktree = files
                .run(&crate::async_service::CancellationToken::new(), move || {
                    Ok(root.as_ref().is_some_and(|p| p.join(".git").is_file()))
                })
                .await
                .unwrap_or(false);
            if self.data.branch != previous {
                self.pr_at = Instant::now() - Duration::from_secs(60);
            }
        }
        if include_pr
            && self.data.root.is_some()
            && (changed || !self.pr_enabled || self.pr_at.elapsed() >= Duration::from_secs(60))
        {
            self.pr_at = Instant::now();
            match pull_request_async(processes, cwd).await {
                Ok(pr) => {
                    self.data.pull_request = pr;
                    self.data.pr_error = None;
                }
                Err(error) => {
                    self.data.pull_request = None;
                    self.data.pr_error = Some(error.to_string().chars().take(256).collect());
                }
            }
        }
        if !include_pr {
            self.data.pull_request = None;
            self.data.pr_error = None;
        }
        self.pr_enabled = include_pr;
        match ports_async(processes, files, identity).await {
            Ok(ports) => {
                self.data.ports = ports.unwrap_or_default();
                self.data.ports_error = None;
            }
            Err(error) => {
                self.data.ports.clear();
                self.data.ports_error = Some(error.to_string());
            }
        }
        self.data.clone()
    }
}

#[cfg(feature = "async-client")]
async fn git_async(
    processes: &crate::async_process::Processes,
    files: &crate::async_service::NativePool,
    cwd: &Path,
    args: &[&str],
) -> Result<String> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(cwd)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0");
    let directory = cwd.to_owned();
    let key = files
        .run(&crate::async_service::CancellationToken::new(), move || {
            Ok(crate::async_process::git_key(&directory))
        })
        .await?;
    let output = processes
        .run(
            command,
            CommandOptions {
                timeout: Duration::from_secs(3),
                stdout_limit: 65536,
                ..Default::default()
            },
            Some(key),
        )
        .await?;
    Ok(String::from_utf8(output.stdout)?.trim_end().into())
}
#[cfg(feature = "async-client")]
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
#[cfg(feature = "async-client")]
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ports_are_scoped_sorted_and_ipv6_is_preserved() {
        let ports = parse_ports(
            "p7\nn127.0.0.1:9000\nn[::1]:8080\np99\nn*:22\np7\nn[::1]:8080\n",
            &HashSet::from([7]),
        );
        assert_eq!(ports.len(), 2);
        assert_eq!(ports[0].url(), "http://[::1]:8080/");
        assert_eq!(ports[1].port, 9000);
    }
    #[test]
    fn external_links_allow_web_urls_only() {
        assert!(http_url("https://github.com/a/b/pull/1").is_ok());
        assert!(http_url("file:///etc/passwd").is_err());
        assert!(http_url("javascript:alert(1)").is_err());
        assert!(http_url("https://user:pass@example.com").is_err());
    }
}
