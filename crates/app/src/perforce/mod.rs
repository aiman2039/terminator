//! Perforce CLI adapter. Reads and explicit mutations use the GUI process pool.
mod context;
mod document;
mod run;

use crate::gui_services::Services;
use anyhow::{Context, Result, bail, ensure};
pub use context::{ContextInput, context};
pub use document::{DocumentInput, document};
pub use run::{Progress, RUN_BUSY, RUN_CANCELLED, RUN_SUBSYSTEM, RUN_TIMEOUT, Run, RunSlot};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use terminator_core::{CommandOptions, async_service::CancellationToken};

#[derive(Clone, Debug)]
pub struct Workspace {
    pub client: String,
    pub command_root: PathBuf,
    pub stream: String,
    pub files: Vec<File>,
    pub pending: Vec<(String, String)>,
    pub history: Vec<(String, String)>,
}

#[derive(Clone, Debug)]
pub struct File {
    pub path: PathBuf,
    pub action: String,
    pub change: String,
    pub opened: bool,
}

#[derive(Clone, Debug)]
pub enum Op {
    Reconcile(PathBuf),
    Edit(PathBuf),
    Sync,
    Submit(String),
    Shelve(String),
    Unshelve(String),
}

pub fn records(raw: &[u8], allow_empty: bool) -> Result<Vec<Value>> {
    let mut records = Vec::new();
    for line in raw
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let record: Value = serde_json::from_slice(line).context("Invalid p4 JSON output")?;
        ensure!(record.is_object(), "Invalid p4 JSON record");
        let severity = number(&record, "severity");
        if severity >= 2 || field(&record, "code") == "error" {
            if allow_empty && severity == 2 && number(&record, "generic") == 17 {
                continue; // EV_EMPTY: a successful query with no matching files.
            }
            bail!("{}", field(&record, "data"));
        }
        records.push(record);
    }
    Ok(records)
}

fn number(record: &Value, key: &str) -> u64 {
    record
        .get(key)
        .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()))
        .unwrap_or(0)
}

fn field<'a>(record: &'a Value, key: &str) -> &'a str {
    record.get(key).and_then(Value::as_str).unwrap_or("")
}

struct RunInput<'a> {
    service: &'a Services,
    cwd: &'a Path,
    args: &'a [String],
    json: bool,
}

/// A long action's output limit; past it the start is dropped.
const LONG_STDOUT_LIMIT: usize = 64 * 1024 * 1024;

async fn run(input: RunInput<'_>) -> Result<Vec<u8>> {
    run_with(input, None).await
}

/// Runs `p4`. With `long`, the call is a cancellable network transfer:
/// [`RUN_TIMEOUT`] cap, and `long` (or the cap) sends Ctrl-C first.
async fn run_with(input: RunInput<'_>, long: Option<&CancellationToken>) -> Result<Vec<u8>> {
    let RunInput {
        service,
        cwd,
        args,
        json,
    } = input;
    let mut command = Command::new("p4");
    command.current_dir(cwd).arg("-d").arg(cwd);
    if json {
        command.args(["-ztag", "-Mj"]);
    }
    command.args(args);
    let options = CommandOptions {
        timeout: if long.is_some() {
            RUN_TIMEOUT
        } else if args.first().is_some_and(|arg| arg == "info") {
            Duration::from_secs(3)
        } else {
            Duration::from_secs(30)
        },
        stdout_limit: if long.is_some() {
            LONG_STDOUT_LIMIT
        } else {
            4 * 1024 * 1024
        },
        // Perforce reports structured errors on stdout, including empty lists.
        accepted_exit_codes: Some(if json { vec![0, 1] } else { vec![0] }),
        ..Default::default()
    };
    let processes = service.processes();
    let output = match long {
        Some(cancel) => processes.run_cancellable(command, options, cancel).await,
        None => {
            processes
                .run(command, options, Some("perforce".into()))
                .await
        }
    };
    let mut output = output?;
    if long.is_some() && output.stdout.len() >= LONG_STDOUT_LIMIT {
        // The kept tail may start inside a record.
        let start = output
            .stdout
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(output.stdout.len(), |at| at.saturating_add(1));
        output.stdout.drain(..start);
    }
    // Raw print can report a missing revision as a warning on stderr and
    // still exit successfully. Never turn that failed read into an empty side.
    ensure!(
        json || output.stderr.iter().all(u8::is_ascii_whitespace),
        "Perforce file read failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    // Exit 1 is accepted only for structured query errors such as EV_EMPTY.
    // Startup/configuration errors can instead arrive solely on stderr.
    if !output.status.success() {
        let structured_error = output.stdout.split(|byte| *byte == b'\n').any(|line| {
            serde_json::from_slice::<Value>(line)
                .is_ok_and(|record| number(&record, "severity") >= 2)
        });
        ensure!(
            json && structured_error,
            "Perforce command failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

struct QueryInput<'a> {
    service: &'a Services,
    cwd: &'a Path,
    args: &'a [&'a str],
    allow_empty: bool,
}

async fn query(input: QueryInput<'_>) -> Result<Vec<Value>> {
    let QueryInput {
        service,
        cwd,
        args,
        allow_empty,
    } = input;
    records(
        &run(RunInput {
            service,
            cwd,
            args: &args.iter().map(ToString::to_string).collect::<Vec<_>>(),
            json: true,
        })
        .await?,
        allow_empty,
    )
}

/// Escape literal file names before passing them as Perforce file specs.
fn filespec(path: &Path) -> Result<String> {
    let text = path
        .to_str()
        .context("Perforce requires a UTF-8 file path")?;
    ensure!(
        !text.contains('\u{fffd}') && !text.contains("..."),
        "Unsupported Perforce file name"
    );
    Ok(text
        .replace('%', "%25")
        .replace('#', "%23")
        .replace('@', "%40")
        .replace('*', "%2A"))
}

fn local_path(root: &Path, record: &Value) -> Option<PathBuf> {
    let text = ["clientFile", "path", "depotFile"]
        .into_iter()
        .map(|key| field(record, key))
        .find(|text| !text.is_empty() && !text.starts_with("//"))?;
    if text.contains('\u{fffd}') {
        return None;
    }
    let path = PathBuf::from(text);
    let path = if path.is_absolute() {
        path
    } else {
        root.join(path)
    };
    // Canonicalize the parent only: deleted files need not exist, and the
    // final component must retain its identity for symlink review checks.
    let path = path
        .parent()
        .and_then(|parent| parent.canonicalize().ok())
        .and_then(|parent| path.file_name().map(|name| parent.join(name)))
        .unwrap_or(path);
    if path
        .strip_prefix(root)
        .ok()?
        .components()
        .all(|part| matches!(part, std::path::Component::Normal(_)))
    {
        Some(path)
    } else {
        None
    }
}

pub fn parse_files(root: &Path, opened: &[Value], offline: &[Value]) -> Vec<File> {
    let mut files = BTreeMap::new();
    // Opened records win over the reconciliation preview.
    for (records, is_open) in [(offline, false), (opened, true)] {
        for record in records {
            let Some(path) = local_path(root, record) else {
                continue;
            };
            let action = field(record, "action");
            if action.is_empty() {
                continue;
            }
            let change = field(record, "change");
            files.insert(
                path.clone(),
                File {
                    path,
                    action: action.into(),
                    change: if change.is_empty() {
                        "default".into()
                    } else {
                        change.into()
                    },
                    opened: is_open,
                },
            );
        }
    }
    files.into_values().collect()
}

pub struct PerformInput<'a> {
    pub service: &'a Services,
    pub cwd: &'a Path,
    pub op: Op,
    pub cancel: &'a CancellationToken,
    /// A long action ([`Op::is_long`]) that `cancel` stops with Ctrl-C.
    pub long: bool,
}

/// Status lines kept from a long action's output.
const REPORT_LINES: usize = 10;

pub async fn perform(input: PerformInput<'_>) -> Result<crate::workspace_ops::Report> {
    let PerformInput {
        service,
        cwd,
        op,
        cancel,
        long,
    } = input;
    let verb = op.verb();
    let args = match op {
        Op::Edit(path) | Op::Reconcile(path) => {
            let root = cwd.to_owned();
            let path = service
                .fs()
                .run(cancel, move || {
                    let relative = terminator_git::relative_path(&root.canonicalize()?, &path)?;
                    filespec(&root.join(relative))
                })
                .await?;
            vec![verb.into(), path]
        }
        Op::Sync => vec!["sync".into(), format!("{}/...", filespec(cwd)?)],
        Op::Submit(change) | Op::Shelve(change) | Op::Unshelve(change) => {
            ensure!(
                !change.is_empty() && change.bytes().all(|byte| byte.is_ascii_digit()),
                "Enter a numbered pending changelist"
            );
            vec![
                verb.into(),
                if verb == "unshelve" { "-s" } else { "-c" }.into(),
                change,
            ]
        }
    };
    let raw = run_with(
        RunInput {
            service,
            cwd,
            args: &args,
            json: true,
        },
        long.then_some(cancel),
    )
    .await;
    if long && cancel.is_cancelled() {
        bail!("{RUN_CANCELLED}");
    }
    let raw = raw.context("Perforce action failed; check the workspace before retrying")?;
    let result = records(&raw, verb == "sync")?;
    let messages: Vec<_> = result
        .iter()
        .map(|record| field(record, "data"))
        .filter(|text| !text.is_empty())
        .collect();
    Ok(crate::workspace_ops::Report::Message(report(&messages)))
}

/// The status text of an action: its messages, the last [`REPORT_LINES`]
/// of a long list (a large sync names every file).
fn report(messages: &[&str]) -> String {
    match messages.len() {
        0 => "Perforce action complete".into(),
        n if n <= REPORT_LINES => messages.join("\n"),
        n => format!(
            "{} more lines\n{}",
            n.saturating_sub(REPORT_LINES),
            messages
                .get(n.saturating_sub(REPORT_LINES)..)
                .unwrap_or_default()
                .join("\n")
        ),
    }
}

#[cfg(test)]
mod tests;
