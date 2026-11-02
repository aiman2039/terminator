//! Worktree files in index ("clean") form, so diff sides and the patches
//! built from them match what `git apply` compares (autocrlf, eol, ident,
//! working-tree-encoding).
use super::*;

pub(crate) const FILTERED: &str =
    "Hunk actions are not available for files with a Git filter (for example LFS)";

/// Whether a `filter` driver (for example Git LFS) cleans `path`.
pub(crate) async fn filtered(service: &Services, root: &Path, path: &Path) -> Result<bool> {
    let out = git(GitInput {
        service,
        root,
        args: vec![
            "check-attr".into(),
            "-z".into(),
            "filter".into(),
            "--".into(),
            path.into(),
        ],
        input: None,
    })
    .await?;
    let value = out.split(|byte| *byte == 0).nth(2).unwrap_or_default();
    Ok(!matches!(value, b"" | b"unspecified" | b"unset"))
}

pub(crate) struct WorktreeInput<'a> {
    pub service: &'a Services,
    pub root: &'a Path,
    pub path: &'a Path,
    pub cancel: &'a CancellationToken,
}

/// The worktree file as Git would stage it. Files with a filter driver keep
/// their raw bytes: running that filter is not a read-only operation.
pub(crate) async fn worktree(input: WorktreeInput<'_>) -> Result<Vec<u8>> {
    let WorktreeInput {
        service,
        root,
        path,
        cancel,
    } = input;
    let absolute = root.join(path);
    let raw = service
        .fs()
        .run(cancel, move || {
            terminator_git::worktree_file(&absolute).map_err(anyhow::Error::from)
        })
        .await?;
    if raw.is_empty() || filtered(service, root, path).await? {
        return Ok(raw);
    }
    let mut path_arg = OsString::from("--path=");
    path_arg.push(path);
    let oid = git(GitInput {
        service,
        root,
        args: vec![
            "hash-object".into(),
            "-w".into(),
            "--stdin".into(),
            path_arg,
        ],
        input: Some(raw),
    })
    .await?;
    let oid = std::str::from_utf8(&oid)?.trim().to_owned();
    git(GitInput {
        service,
        root,
        args: vec!["cat-file".into(), "blob".into(), oid.into()],
        input: None,
    })
    .await
}
