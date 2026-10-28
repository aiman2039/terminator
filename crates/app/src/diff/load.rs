//! Load diff sides from Git, Perforce, or an SSH project.
use super::*;

pub struct DocumentAsyncInput<'a> {
    pub service: &'a crate::gui_services::Services,
    pub cwd: PathBuf,
    pub path: PathBuf,
    pub staged: bool,
    pub cancel: &'a terminator_core::async_service::CancellationToken,
}

pub async fn document_async(input: DocumentAsyncInput<'_>) -> Result<DiffDocument> {
    let DocumentAsyncInput {
        service,
        cwd,
        path,
        staged,
        cancel,
    } = input;
    document_with_context(DocumentWithContextInput {
        service,
        cwd,
        path,
        staged,
        context: CONTEXT,
        cancel,
    })
    .await
}

pub(crate) struct DocumentWithContextInput<'a> {
    pub(crate) service: &'a crate::gui_services::Services,
    pub(crate) cwd: PathBuf,
    pub(crate) path: PathBuf,
    pub(crate) staged: bool,
    pub(crate) context: usize,
    pub(crate) cancel: &'a terminator_core::async_service::CancellationToken,
}

pub(crate) async fn document_with_context(
    input: DocumentWithContextInput<'_>,
) -> Result<DiffDocument> {
    let DocumentWithContextInput {
        service,
        cwd,
        path,
        staged,
        context,
        cancel,
    } = input;
    let options = || CommandOptions {
        stdout_limit: 4 * 1024 * 1024,
        ..Default::default()
    };
    let root_bytes = terminator_git::run(
        service.processes(),
        service.fs(),
        terminator_git::RunInput {
            cwd: &cwd,
            args: vec!["rev-parse".into(), "--show-toplevel".into()],
            options: options(),
        },
    )
    .await;
    let root_bytes = match root_bytes {
        Ok(bytes) => bytes,
        Err(_) if !staged => {
            return crate::perforce::document(crate::perforce::DocumentInput {
                service,
                cwd,
                path,
                context,
                cancel,
            })
            .await;
        }
        Err(error) => return Err(error.into()),
    };
    let root = PathBuf::from(std::str::from_utf8(&root_bytes)?.trim());
    let (root, relative) = service
        .fs()
        .run(cancel, move || {
            let root = root.canonicalize().context("Resolve Git root")?;
            let relative = terminator_git::relative_path(&root, &path)?;
            anyhow::Ok((root, relative))
        })
        .await?;
    let mut args: Vec<std::ffi::OsString> = [
        "diff",
        "--raw",
        "-z",
        "--no-abbrev",
        "--no-ext-diff",
        "--no-textconv",
        "--find-renames",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    if staged {
        args.push("--cached".into());
    }
    let raw = terminator_git::run(
        service.processes(),
        service.fs(),
        terminator_git::RunInput {
            cwd: &root,
            args,
            options: options(),
        },
    )
    .await?;
    let oids = terminator_git::find_record(&raw, &relative)?;
    let (left, right) = if let Some((left, right)) = oids {
        let left = terminator_git::snapshot::blob_async(
            service.processes(),
            service.fs(),
            terminator_git::snapshot::BlobAsyncInput {
                root: &root,
                oid: &left,
                options: options(),
            },
        )
        .await?;
        let right = if staged {
            terminator_git::snapshot::blob_async(
                service.processes(),
                service.fs(),
                terminator_git::snapshot::BlobAsyncInput {
                    root: &root,
                    oid: &right,
                    options: options(),
                },
            )
            .await?
        } else {
            let path = root.join(&relative);
            service
                .fs()
                .run(cancel, move || {
                    terminator_git::worktree_file(&path).map_err(anyhow::Error::from)
                })
                .await?
        };
        (left, right)
    } else {
        let tracked = terminator_git::run(
            service.processes(),
            service.fs(),
            terminator_git::RunInput {
                cwd: &root,
                args: vec![
                    "ls-files".into(),
                    "--stage".into(),
                    "-z".into(),
                    "--".into(),
                    actions::literal(&relative),
                ],
                options: options(),
            },
        )
        .await?;
        let oid = tracked
            .split(|byte| *byte == b' ')
            .nth(1)
            .map(std::str::from_utf8)
            .transpose()?;
        let left = if let Some(oid) = oid {
            terminator_git::snapshot::blob_async(
                service.processes(),
                service.fs(),
                terminator_git::snapshot::BlobAsyncInput {
                    root: &root,
                    oid,
                    options: options(),
                },
            )
            .await?
        } else {
            Vec::new()
        };
        let right = if staged {
            left.clone()
        } else {
            let path = root.join(&relative);
            service
                .fs()
                .run(cancel, move || {
                    terminator_git::worktree_file(&path).map_err(anyhow::Error::from)
                })
                .await?
        };
        (left, right)
    };
    let source = actions::Source::capture(service, &root, &relative, cancel).await?;
    service
        .cpu()
        .run(cancel, move || {
            let (left, right) = decode_sides(&left, &right)?;
            let mut doc = build_with_context(BuildWithContextInput {
                path: &relative,
                left: &left,
                right: &right,
                staged,
                context,
            });
            doc.source = Some(source);
            Ok(doc)
        })
        .await
}
pub(crate) struct RemoteDocumentInput<'a> {
    pub(crate) service: &'a crate::gui_services::Services,
    pub(crate) client: &'a terminator_core::async_client::Client,
    pub(crate) tab: &'a crate::Tab,
    pub(crate) context: usize,
    pub(crate) cancel: &'a terminator_core::async_service::CancellationToken,
}

pub(crate) async fn remote_document(input: RemoteDocumentInput<'_>) -> Result<DiffDocument> {
    let RemoteDocumentInput {
        service,
        client,
        tab,
        context,
        cancel,
    } = input;
    use base64::Engine;
    let crate::Tab::Diff {
        remote: Some(project),
        cwd,
        path,
        staged,
    } = tab
    else {
        anyhow::bail!("Not a remote diff")
    };
    let relative = path
        .strip_prefix(cwd)?
        .to_str()
        .context("Remote diff path must be UTF-8")?
        .to_owned();
    terminator_core::remote::validate_relative(&relative)?;
    let response = client
        .rpc(terminator_core::Request::RemoteProject {
            project: project.clone(),
            operation: terminator_core::remote::Operation::GitDiff {
                path: relative.clone(),
                staged: *staged,
            },
        })
        .await?;
    let terminator_core::Response::RemoteProject(terminator_core::remote::Reply::Diff {
        path: actual,
        left,
        right,
        staged: actual_staged,
    }) = response
    else {
        anyhow::bail!("Unexpected remote diff response")
    };
    anyhow::ensure!(
        actual == relative && actual_staged == *staged,
        "Remote diff identity changed"
    );
    let path = path.clone();
    let staged = *staged;
    service
        .cpu()
        .run(cancel, move || {
            let left = base64::engine::general_purpose::STANDARD.decode(left)?;
            let right = base64::engine::general_purpose::STANDARD.decode(right)?;
            anyhow::ensure!(
                left.len() <= terminator_core::remote::MAX_FILE_BYTES
                    && right.len() <= terminator_core::remote::MAX_FILE_BYTES,
                "Remote diff exceeds its snapshot limit"
            );
            let left = String::from_utf8(left)
                .context("Remote binary files cannot be displayed as text diffs")?;
            let right = String::from_utf8(right)
                .context("Remote binary files cannot be displayed as text diffs")?;
            Ok(build_with_context(BuildWithContextInput {
                path: &path,
                left: &left,
                right: &right,
                staged,
                context,
            }))
        })
        .await
}
