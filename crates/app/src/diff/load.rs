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
    let git = GitSnapshots { service, cancel };
    let root_bytes = match git
        .run(&cwd, vec!["rev-parse".into(), "--show-toplevel".into()])
        .await
    {
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
    let (left, right) = git.sides(&root, &relative, staged).await?;
    let source = actions::Source::capture(actions::CaptureInput {
        service,
        root: &root,
        path: &relative,
        cancel,
    })
    .await?;
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

/// Reads Git blobs and worktree files through the GUI service pools.
#[derive(Clone, Copy)]
struct GitSnapshots<'a> {
    service: &'a crate::gui_services::Services,
    cancel: &'a terminator_core::async_service::CancellationToken,
}

fn git_options() -> CommandOptions {
    CommandOptions {
        stdout_limit: 4 * 1024 * 1024,
        ..Default::default()
    }
}

impl GitSnapshots<'_> {
    async fn run(
        self,
        cwd: &Path,
        args: Vec<std::ffi::OsString>,
    ) -> std::result::Result<Vec<u8>, terminator_core::async_process::AsyncProcessError> {
        terminator_git::run(
            self.service.processes(),
            self.service.fs(),
            terminator_git::RunInput {
                cwd,
                args,
                options: git_options(),
            },
        )
        .await
    }

    async fn blob(self, root: &Path, oid: &str) -> Result<Vec<u8>> {
        Ok(terminator_git::snapshot::blob_async(
            self.service.processes(),
            self.service.fs(),
            terminator_git::snapshot::BlobAsyncInput {
                root,
                oid,
                options: git_options(),
            },
        )
        .await?)
    }

    async fn worktree(self, root: &Path, relative: &Path) -> Result<Vec<u8>> {
        let path = root.join(relative);
        self.service
            .fs()
            .run(self.cancel, move || {
                terminator_git::worktree_file(&path).map_err(anyhow::Error::from)
            })
            .await
    }

    /// Left and right bytes: HEAD to index when `staged`, else index to
    /// worktree. Files without a diff record compare against the index.
    async fn sides(self, root: &Path, relative: &Path, staged: bool) -> Result<(Vec<u8>, Vec<u8>)> {
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
        let raw = self.run(root, args).await?;
        let Some((left, right)) = terminator_git::find_record(&raw, relative)? else {
            return self.unchanged_sides(root, relative, staged).await;
        };
        let left = self.blob(root, &left).await?;
        let right = if staged {
            self.blob(root, &right).await?
        } else {
            self.worktree(root, relative).await?
        };
        Ok((left, right))
    }

    async fn unchanged_sides(
        self,
        root: &Path,
        relative: &Path,
        staged: bool,
    ) -> Result<(Vec<u8>, Vec<u8>)> {
        let tracked = self
            .run(
                root,
                vec![
                    "ls-files".into(),
                    "--stage".into(),
                    "-z".into(),
                    "--".into(),
                    actions::literal(relative),
                ],
            )
            .await?;
        let oid = tracked
            .split(|byte| *byte == b' ')
            .nth(1)
            .map(std::str::from_utf8)
            .transpose()?;
        let left = if let Some(oid) = oid {
            self.blob(root, oid).await?
        } else {
            Vec::new()
        };
        let right = if staged {
            left.clone()
        } else {
            self.worktree(root, relative).await?
        };
        Ok((left, right))
    }
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
