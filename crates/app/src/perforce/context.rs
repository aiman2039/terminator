//! Perforce workspace context: client root, opened files and changelists.
use super::{File, QueryInput, Workspace, field, filespec, parse_files, query};
use crate::{gui_services::Services, services::ContextData};
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use terminator_core::async_service::CancellationToken;

pub struct ContextInput<'a> {
    pub service: &'a Services,
    pub result: &'a mut ContextData,
    pub include_status: bool,
    pub cancel: &'a CancellationToken,
}

pub async fn context(input: ContextInput<'_>) {
    let ContextInput {
        service,
        result,
        include_status,
        cancel,
    } = input;
    let Some(info) = client_info(service, result).await else {
        return;
    };
    let Some((root, command_root)) = client_root(ClientRootInput {
        service,
        cwd: &result.cwd,
        info: &info,
        cancel,
    })
    .await
    else {
        return;
    };
    let client = field(&info, "clientName").to_string();
    result.root = Some(root.clone());
    result.branch.clone_from(&client);
    result.perforce = Some(Workspace {
        client,
        command_root: command_root.clone(),
        stream: field(&info, "clientStream").into(),
        files: Vec::new(),
        pending: Vec::new(),
        history: Vec::new(),
    });
    if !include_status {
        return;
    }
    let status = workspace_files(WorkspaceFilesInput {
        service,
        command_root: &command_root,
        root: &root,
        cancel,
    })
    .await;
    if let Some(workspace) = &mut result.perforce
        && let Err(error) = load_changelists(service, &result.cwd, workspace).await
    {
        result.error = Some(error);
    }
    apply_status(result, &root, status);
}

/// The `p4 info` record that names a client root. Errors surface only when
/// the environment configures Perforce.
async fn client_info(service: &Services, result: &mut ContextData) -> Option<Value> {
    let info = match query(QueryInput {
        service,
        cwd: &result.cwd,
        args: &["info"],
        allow_empty: false,
    })
    .await
    {
        Ok(info) => info,
        Err(error) => {
            if ["P4PORT", "P4CLIENT", "P4CONFIG"]
                .iter()
                .any(|key| std::env::var_os(key).is_some())
            {
                result.error = Some(format!("Perforce: {error:#}"));
            }
            return None;
        }
    };
    info.into_iter()
        .find(|record| !field(record, "clientRoot").is_empty())
}

struct ClientRootInput<'a> {
    service: &'a Services,
    cwd: &'a Path,
    info: &'a Value,
    cancel: &'a CancellationToken,
}

/// The canonical folder and the same folder under the client root as
/// Perforce spells it, or `None` outside the workspace.
async fn client_root(input: ClientRootInput<'_>) -> Option<(PathBuf, PathBuf)> {
    let ClientRootInput {
        service,
        cwd,
        info,
        cancel,
    } = input;
    let root = PathBuf::from(field(info, "clientRoot"));
    let cwd = cwd.to_owned();
    service
        .fs()
        .run(cancel, move || {
            let canonical = root.canonicalize()?;
            let cwd = cwd.canonicalize()?;
            let relative = cwd
                .strip_prefix(&canonical)
                .context("Folder is outside the Perforce workspace")?;
            let command_root = root.join(relative);
            anyhow::Ok((cwd, command_root))
        })
        .await
        .ok()
}

struct WorkspaceFilesInput<'a> {
    service: &'a Services,
    command_root: &'a Path,
    root: &'a Path,
    cancel: &'a CancellationToken,
}

async fn workspace_files(input: WorkspaceFilesInput<'_>) -> Result<Vec<File>> {
    let WorkspaceFilesInput {
        service,
        command_root,
        root,
        cancel,
    } = input;
    let scope = format!("{}/...", filespec(command_root)?);
    let opened = query(QueryInput {
        service,
        cwd: command_root,
        args: &["fstat", "-Ro", "-T", "clientFile,action,change", &scope],
        allow_empty: true,
    })
    .await?;
    let offline = query(QueryInput {
        service,
        cwd: command_root,
        args: &["reconcile", "-n", "-l", &scope],
        allow_empty: true,
    })
    .await?;
    let root = root.to_owned();
    service
        .fs()
        .run(cancel, move || {
            anyhow::Ok(parse_files(&root, &opened, &offline))
        })
        .await
}

/// Fills pending and submitted changelists. Returns the last failure.
async fn load_changelists(
    service: &Services,
    cwd: &Path,
    workspace: &mut Workspace,
) -> std::result::Result<(), String> {
    let mut failure = Ok(());
    for (state, destination) in [
        ("pending", &mut workspace.pending),
        ("submitted", &mut workspace.history),
    ] {
        match query(QueryInput {
            service,
            cwd,
            args: &["changes", "-m", "30", "-s", state, "-c", &workspace.client],
            allow_empty: true,
        })
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
                    .collect();
            }
            Err(error) => failure = Err(format!("Perforce changelists: {error:#}")),
        }
    }
    failure
}

fn apply_status(result: &mut ContextData, root: &Path, status: Result<Vec<File>>) {
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
            result.decorations = terminator_git::decorations(root, &result.changes);
            if let Some(workspace) = &mut result.perforce {
                workspace.files = files;
            }
        }
        Err(error) => result.error = Some(format!("Perforce: {error:#}")),
    }
}
