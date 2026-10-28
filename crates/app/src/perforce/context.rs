//! Perforce workspace context: client root, opened files and changelists.
use super::{File, Workspace, field, filespec, parse_files, query};
use crate::{gui_services::Services, services::ContextData};
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};
use terminator_core::async_service::CancellationToken;

pub async fn context(
    service: &Services,
    result: &mut ContextData,
    include_status: bool,
    cancel: &CancellationToken,
) {
    let Some(info) = client_info(service, result).await else {
        return;
    };
    let Some((root, command_root)) = client_root(service, &result.cwd, &info, cancel).await else {
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
    let status = workspace_files(service, &command_root, &root, cancel).await;
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
    let info = match query(service, &result.cwd, &["info"], false).await {
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

/// The canonical folder and the same folder under the client root as
/// Perforce spells it, or `None` outside the workspace.
async fn client_root(
    service: &Services,
    cwd: &Path,
    info: &Value,
    cancel: &CancellationToken,
) -> Option<(PathBuf, PathBuf)> {
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

async fn workspace_files(
    service: &Services,
    command_root: &Path,
    root: &Path,
    cancel: &CancellationToken,
) -> Result<Vec<File>> {
    let scope = format!("{}/...", filespec(command_root)?);
    let opened = query(
        service,
        command_root,
        &["fstat", "-Ro", "-T", "clientFile,action,change", &scope],
        true,
    )
    .await?;
    let offline = query(
        service,
        command_root,
        &["reconcile", "-n", "-l", &scope],
        true,
    )
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
        match query(
            service,
            cwd,
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
