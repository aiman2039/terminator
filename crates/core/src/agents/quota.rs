//! Agent quota: how much of each usage window an agent account has used and
//! when it resets. Read only from data the agent CLI writes itself; never
//! from credentials, a provider API, or terminal text.
use serde::Deserialize;
use std::{
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

/// Bytes read from the end of a Codex session log. The newest
/// `rate_limits` record is near the end.
const TAIL: u64 = 512 * 1024;
/// Session logs looked at, newest first, before giving up.
const LOGS: usize = 8;

/// One usage window of an agent account.
#[derive(Clone, Debug, PartialEq)]
pub struct QuotaWindow {
    /// `5h`, `weekly`, or the window length in minutes.
    pub label: String,
    pub used_percent: f64,
    /// Unix seconds; `None` when the agent did not say.
    pub resets_at: Option<u64>,
}

/// What an agent CLI last reported for its account.
#[derive(Clone, Debug, PartialEq)]
pub struct Quota {
    pub agent: String,
    pub plan: Option<String>,
    pub windows: Vec<QuotaWindow>,
    /// Unix seconds of the record the values come from.
    pub observed_at: u64,
}

#[derive(Deserialize)]
struct Line {
    timestamp: Option<String>,
    payload: Option<Payload>,
}

#[derive(Deserialize)]
struct Payload {
    rate_limits: Option<RateLimits>,
}

#[derive(Deserialize)]
struct RateLimits {
    primary: Option<Window>,
    secondary: Option<Window>,
    plan_type: Option<String>,
}

#[derive(Deserialize)]
struct Window {
    used_percent: f64,
    window_minutes: Option<u64>,
    resets_at: Option<u64>,
    resets_in_seconds: Option<u64>,
}

fn window_label(minutes: Option<u64>) -> String {
    match minutes {
        Some(300) => "5h".into(),
        Some(10080) => "weekly".into(),
        Some(minutes) if minutes % 60 == 0 => format!("{}h", minutes / 60),
        Some(minutes) => format!("{minutes}m"),
        None => "window".into(),
    }
}

fn unix_seconds(timestamp: &str) -> Option<u64> {
    let time =
        time::OffsetDateTime::parse(timestamp, &time::format_description::well_known::Rfc3339)
            .ok()?;
    u64::try_from(time.unix_timestamp()).ok()
}

/// The newest `rate_limits` record in a Codex session log text that names
/// at least one window. Plans without windows report nothing.
#[must_use]
pub fn codex_quota_from_log(text: &str) -> Option<Quota> {
    text.lines().rev().find_map(|line| {
        if !line.contains("\"rate_limits\"") {
            return None;
        }
        let line: Line = serde_json::from_str(line).ok()?;
        let limits = line.payload?.rate_limits?;
        let observed_at = line
            .timestamp
            .as_deref()
            .and_then(unix_seconds)
            .unwrap_or(0);
        let windows = [limits.primary, limits.secondary]
            .into_iter()
            .flatten()
            .map(|w| QuotaWindow {
                label: window_label(w.window_minutes),
                used_percent: w.used_percent.clamp(0.0, 100.0),
                resets_at: w
                    .resets_at
                    .or_else(|| w.resets_in_seconds.map(|s| observed_at.saturating_add(s))),
            })
            .collect::<Vec<_>>();
        (!windows.is_empty()).then(|| Quota {
            agent: "codex".into(),
            plan: limits.plan_type,
            windows,
            observed_at,
        })
    })
}

/// `$CODEX_HOME/sessions`, else `~/.codex/sessions`.
#[must_use]
pub fn codex_sessions_dir(home: Option<&Path>) -> Option<PathBuf> {
    if let Some(codex) = std::env::var_os("CODEX_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(codex).join("sessions"));
    }
    Some(home?.join(".codex").join("sessions"))
}

/// The newest session log files under `dir`. Codex names its folders
/// `YYYY/MM/DD` and its logs `rollout-<time>-…`, so a walk in reverse name
/// order meets the newest first and stops after [`LOGS`] files. Only
/// `.jsonl` files are taken; links are not followed.
fn newest_logs(dir: &Path) -> Vec<PathBuf> {
    walkdir::WalkDir::new(dir)
        .max_depth(4)
        .sort_by(|a, b| b.file_name().cmp(a.file_name()))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .take(LOGS)
        .map(walkdir::DirEntry::into_path)
        .collect()
}

fn tail(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let length = file.metadata()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(TAIL)))?;
    let mut bytes = Vec::new();
    file.take(TAIL).read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The newest Codex quota in the session logs under `dir`. Blocking file
/// reads: call it off the GUI thread.
#[must_use]
pub fn codex_quota(dir: &Path) -> Option<Quota> {
    newest_logs(dir)
        .iter()
        .find_map(|path| codex_quota_from_log(&tail(path).ok()?))
}

/// Where `terminator-hook statusline` records Claude Code's `rate_limits`,
/// under the data folder shared by every generation.
#[must_use]
pub fn claude_quota_path(data: &Path) -> PathBuf {
    data.join("quota").join("claude.json")
}

#[derive(Deserialize)]
struct ClaudeWindow {
    used_percentage: f64,
    resets_at: Option<u64>,
}

#[derive(Deserialize)]
struct ClaudeRecord {
    five_hour: Option<ClaudeWindow>,
    seven_day: Option<ClaudeWindow>,
    observed_at: u64,
}

/// The Claude Code quota the status-line source last recorded.
#[must_use]
pub fn claude_quota_from_record(bytes: &[u8]) -> Option<Quota> {
    let record: ClaudeRecord = serde_json::from_slice(bytes).ok()?;
    let windows = [("5h", record.five_hour), ("weekly", record.seven_day)]
        .into_iter()
        .filter_map(|(label, window)| {
            let window = window?;
            Some(QuotaWindow {
                label: label.into(),
                used_percent: window.used_percentage.clamp(0.0, 100.0),
                resets_at: window.resets_at,
            })
        })
        .collect::<Vec<_>>();
    (!windows.is_empty()).then(|| Quota {
        agent: "claude".into(),
        plan: None,
        windows,
        observed_at: record.observed_at,
    })
}

/// Every agent quota the local agent CLIs report. `data` is the shared
/// data folder. Blocking file reads: call it off the GUI thread.
#[must_use]
pub fn read_all(data: &Path) -> Vec<Quota> {
    let home = directories::BaseDirs::new().map(|base| base.home_dir().to_path_buf());
    let claude = std::fs::read(claude_quota_path(data))
        .ok()
        .and_then(|bytes| claude_quota_from_record(&bytes));
    let codex = codex_sessions_dir(home.as_deref()).and_then(|dir| codex_quota(&dir));
    claude.into_iter().chain(codex).collect()
}

#[cfg(test)]
#[path = "quota_tests.rs"]
mod tests;
