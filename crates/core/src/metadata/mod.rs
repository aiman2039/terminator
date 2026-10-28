//! Read-only context discovery. Git and gh own repository/provider semantics;
//! sysinfo scopes lsof results to descendants of the authenticated session PID.
use crate::{CommandOptions, find_executable, run_command};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

mod cache;
#[cfg(feature = "async-client")]
mod cache_async;
pub use cache::Cache;
#[cfg(feature = "async-client")]
pub use cache_async::CollectAsyncInput;

/// Why a URL was rejected, or Git, pull request, or port discovery failed.
#[derive(Debug, thiserror::Error)]
pub enum MetadataError {
    #[error("Git not found")]
    GitNotFound,
    #[error("Invalid URL")]
    InvalidUrl {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("Only HTTP(S) URLs are supported")]
    UnsupportedUrl { scheme: String },
    #[error("URLs containing credentials are not supported")]
    UrlCredentials { host: Option<String> },
    #[error("GitHub CLI (gh) is not installed")]
    GhNotInstalled,
    #[error("Missing PR number")]
    MissingPrNumber,
    #[error("Missing PR URL")]
    MissingPrUrl,
    #[error("lsof is not installed")]
    LsofNotInstalled,
    #[error("Session process identity changed")]
    IdentityChanged { pid: u32 },
    #[error("Too many session descendants for port discovery")]
    TooManyDescendants { pid: u32 },
    #[error(transparent)]
    Process(#[from] crate::ProcessError),
    #[cfg(feature = "async-client")]
    #[error(transparent)]
    AsyncProcess(#[from] crate::async_process::AsyncProcessError),
    #[cfg(feature = "async-client")]
    #[error(transparent)]
    Stopped(#[from] crate::async_service::Failure),
    #[error(transparent)]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
type Result<T, E = MetadataError> = std::result::Result<T, E>;
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
    #[must_use]
    pub fn url(&self) -> String {
        let host = self
            .address
            .rsplit_once(':')
            .map_or("127.0.0.1", |(s, _)| s);
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
    let program = find_executable("git").ok_or(MetadataError::GitNotFound)?;
    let command = crate::git::command_with(program.as_os_str(), cwd, args);
    Ok(String::from_utf8(
        run_command(
            command,
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
    let url = url::Url::parse(value).map_err(|source| MetadataError::InvalidUrl {
        url: value.chars().take(256).collect(),
        source,
    })?;
    if !(matches!(url.scheme(), "http" | "https") && url.host_str().is_some()) {
        return Err(MetadataError::UnsupportedUrl {
            scheme: url.scheme().to_owned(),
        });
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(MetadataError::UrlCredentials {
            host: url.host_str().map(str::to_owned),
        });
    }
    Ok(url.into())
}
fn pull_request(cwd: &Path) -> Result<Option<PullRequest>> {
    let mut c = Command::new(find_executable("gh").ok_or(MetadataError::GhNotInstalled)?);
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
        number: value
            .get("number")
            .and_then(serde_json::Value::as_u64)
            .ok_or(MetadataError::MissingPrNumber)?,
        title: value
            .get("title")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .chars()
            .take(256)
            .collect(),
        url: http_url(
            value
                .get("url")
                .and_then(serde_json::Value::as_str)
                .ok_or(MetadataError::MissingPrUrl)?,
        )?,
        state: value
            .get("state")
            .and_then(serde_json::Value::as_str)
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
        .map(std::string::ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let mut c = Command::new(find_executable("lsof").ok_or(MetadataError::LsofNotInstalled)?);
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
    if process.start_time().abs_diff(identity.1) > 2 {
        return Err(MetadataError::IdentityChanged { pid: identity.0 });
    }
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
        if owned.len() > 256 {
            return Err(MetadataError::TooManyDescendants { pid: identity.0 });
        }
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

#[cfg(test)]
mod tests;
