//! Perforce CLI adapter. Reads and explicit mutations use the GUI process pool.
use crate::{gui_services::Services, services::ContextData};
use anyhow::{Context, Result, bail, ensure};
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

async fn run(service: &Services, cwd: &Path, args: &[String], json: bool) -> Result<Vec<u8>> {
    let mut command = Command::new("p4");
    command.current_dir(cwd).arg("-d").arg(cwd);
    if json {
        command.args(["-ztag", "-Mj"]);
    }
    command.args(args);
    let output = service
        .processes()
        .run(
            command,
            CommandOptions {
                timeout: Duration::from_secs(if args.first().is_some_and(|arg| arg == "info") {
                    3
                } else {
                    30
                }),
                stdout_limit: 4 * 1024 * 1024,
                // Perforce reports structured errors on stdout, including empty lists.
                accepted_exit_codes: Some(if json { vec![0, 1] } else { vec![0] }),
                ..Default::default()
            },
            Some("perforce".into()),
        )
        .await?;
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

async fn query(
    service: &Services,
    cwd: &Path,
    args: &[&str],
    allow_empty: bool,
) -> Result<Vec<Value>> {
    records(
        &run(
            service,
            cwd,
            &args.iter().map(ToString::to_string).collect::<Vec<_>>(),
            true,
        )
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

pub async fn context(
    service: &Services,
    result: &mut ContextData,
    include_status: bool,
    cancel: &CancellationToken,
) {
    let info = match query(service, &result.cwd, &["info"], false).await {
        Ok(info) => info,
        Err(error) => {
            if ["P4PORT", "P4CLIENT", "P4CONFIG"]
                .iter()
                .any(|key| std::env::var_os(key).is_some())
            {
                result.error = Some(format!("Perforce: {error:#}"));
            }
            return;
        }
    };
    let Some(info) = info
        .iter()
        .find(|record| !field(record, "clientRoot").is_empty())
    else {
        return;
    };
    let root = PathBuf::from(field(info, "clientRoot"));
    let cwd = result.cwd.clone();
    let Ok((root, command_root)) = service
        .fs()
        .run(cancel, move || {
            let canonical = root.canonicalize()?;
            let cwd = cwd.canonicalize()?;
            let relative = cwd
                .strip_prefix(&canonical)
                .context("Folder is outside the Perforce workspace")?;
            let command_root = root.join(relative);
            Ok((cwd, command_root))
        })
        .await
    else {
        return;
    };
    let client = field(info, "clientName").to_string();
    result.root = Some(root.clone());
    result.branch.clone_from(&client);
    result.perforce = Some(Workspace {
        client,
        command_root: command_root.clone(),
        stream: field(info, "clientStream").into(),
        files: Vec::new(),
        pending: Vec::new(),
        history: Vec::new(),
    });
    if !include_status {
        return;
    }
    let status = async {
        let scope = format!("{}/...", filespec(&command_root)?);
        let opened = query(
            service,
            &command_root,
            &["fstat", "-Ro", "-T", "clientFile,action,change", &scope],
            true,
        )
        .await?;
        let offline = query(
            service,
            &command_root,
            &["reconcile", "-n", "-l", &scope],
            true,
        )
        .await?;
        let root = root.clone();
        service
            .fs()
            .run(cancel, move || Ok(parse_files(&root, &opened, &offline)))
            .await
    }
    .await;
    if let Some(workspace) = &mut result.perforce {
        for (state, destination) in [
            ("pending", &mut workspace.pending),
            ("submitted", &mut workspace.history),
        ] {
            match query(
                service,
                &result.cwd,
                &["changes", "-m", "30", "-s", state, "-c", &workspace.client],
                true,
            )
            .await
            {
                Ok(records) => {
                    *destination = records
                        .iter()
                        .filter_map(|record| {
                            let change = field(record, "change");
                            (!change.is_empty())
                                .then(|| (change.into(), field(record, "desc").trim().into()))
                        })
                        .collect()
                }
                Err(error) => result.error = Some(format!("Perforce changelists: {error:#}")),
            }
        }
    }
    match status {
        Ok(files) => {
            result.changes = files
                .iter()
                .map(|file| terminator_git::Change {
                    path: file.path.clone(),
                    status: match file.action.as_str() {
                        "add" | "branch" | "move/add" => " A",
                        "delete" | "move/delete" => " D",
                        _ => " M",
                    }
                    .into(),
                })
                .collect();
            result.decorations = terminator_git::decorations(&root, &result.changes);
            if let Some(workspace) = &mut result.perforce {
                workspace.files = files;
            }
        }
        Err(error) => result.error = Some(format!("Perforce: {error:#}")),
    }
}

pub async fn perform(
    service: &Services,
    cwd: &Path,
    op: Op,
    cancel: &CancellationToken,
) -> Result<crate::workspace_ops::Report> {
    let verb = match &op {
        Op::Edit(_) => "edit",
        Op::Reconcile(_) => "reconcile",
        Op::Sync => "sync",
        Op::Submit(_) => "submit",
        Op::Shelve(_) => "shelve",
        Op::Unshelve(_) => "unshelve",
    };
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
    let raw = run(service, cwd, &args, true)
        .await
        .context("Perforce action failed; check the workspace before retrying")?;
    let result = records(&raw, verb == "sync")?;
    let messages: Vec<_> = result
        .iter()
        .map(|record| field(record, "data"))
        .filter(|text| !text.is_empty())
        .collect();
    Ok(crate::workspace_ops::Report::Message(
        if messages.is_empty() {
            "Perforce action complete".into()
        } else {
            messages.join("\n")
        },
    ))
}

pub struct DocumentInput<'a> {
    pub service: &'a Services,
    pub cwd: PathBuf,
    pub path: PathBuf,
    pub context: usize,
    pub cancel: &'a CancellationToken,
}

pub async fn document(input: DocumentInput<'_>) -> Result<crate::diff::DiffDocument> {
    let DocumentInput {
        service,
        cwd,
        path,
        context,
        cancel,
    } = input;
    let root = cwd.clone();
    let (canonical, relative, spec) = service
        .fs()
        .run(cancel, move || {
            let canonical = root.canonicalize()?;
            let relative = terminator_git::relative_path(&canonical, &path)?;
            let spec = filespec(&root.join(&relative))?;
            Ok((canonical, relative, spec))
        })
        .await?;
    // fstat reports EV_EMPTY for both new files and paths outside the client
    // view. Verify the mapping before treating an empty result as an addition.
    let mappings = query(service, &cwd, &["where", &spec], false).await?;
    let expected = canonical.join(&relative);
    service
        .fs()
        .run(cancel, move || {
            let mapping = mappings
                .iter()
                .rev()
                .find(|record| local_path(&canonical, record).is_some_and(|path| path == expected));
            ensure!(
                mapping.is_some_and(|record| {
                    record.get("unmap").is_none() && !field(record, "depotFile").starts_with('-')
                }),
                "File is not mapped in the Perforce workspace"
            );
            Ok(())
        })
        .await?;
    let files = query(
        service,
        &cwd,
        &["fstat", "-T", "clientFile,depotFile,haveRev,action", &spec],
        true,
    )
    .await?;
    let record = files
        .iter()
        .find(|record| !field(record, "depotFile").is_empty());
    let left = if let Some(record) = record.filter(|record| number(record, "haveRev") > 0) {
        let depot = field(record, "depotFile");
        run(
            service,
            &cwd,
            &[
                "print".into(),
                "-q".into(),
                format!("{depot}#{}", number(record, "haveRev")),
            ],
            false,
        )
        .await?
    } else {
        Vec::new()
    };
    ensure!(
        left.len() <= terminator_git::SNAPSHOT_LIMIT,
        "Diff file exceeds 1 MiB"
    );
    let path = cwd.join(&relative);
    let deleted =
        record.is_some_and(|record| matches!(field(record, "action"), "delete" | "move/delete"));
    let right = if deleted {
        Vec::new()
    } else {
        service
            .fs()
            .run(cancel, move || {
                terminator_git::worktree_file(&path).map_err(anyhow::Error::from)
            })
            .await?
    };
    service
        .cpu()
        .run(cancel, move || {
            ensure!(
                terminator_git::check_text(&left) && terminator_git::check_text(&right),
                "Binary or non-UTF-8 files cannot be reviewed natively"
            );
            let mut doc = crate::diff::build_with_context(crate::diff::BuildWithContextInput {
                path: &relative,
                left: std::str::from_utf8(&left)?,
                right: std::str::from_utf8(&right)?,
                staged: false,
                context,
            });
            doc.read_only = true;
            doc.left_label = "Have revision".into();
            doc.right_label = "Workspace".into();
            Ok(doc)
        })
        .await
}

#[cfg(test)]
mod tests;
