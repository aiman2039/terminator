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
    let clean = hash(service, root, raw.clone(), vec![path_arg.clone()]).await?;
    if hash(service, root, raw.clone(), vec!["--no-filters".into()]).await? == clean {
        return Ok(raw);
    }
    // The usual conversion (autocrlf / eol) only drops CR before LF: rebuild
    // it here and check it hashes the same, so nothing is written to the
    // object store (#195).
    let lf = crlf_to_lf(&raw);
    if hash(service, root, lf.clone(), vec!["--no-filters".into()]).await? == clean {
        return Ok(lf);
    }
    // Other conversions (ident, working-tree-encoding): let Git store the
    // clean blob and read it back.
    let oid = hash(service, root, raw, vec!["-w".into(), path_arg]).await?;
    git(GitInput {
        service,
        root,
        args: vec!["cat-file".into(), "blob".into(), oid.into()],
        input: None,
    })
    .await
}

/// The object id Git gives `data`. `flags` holds `--path=<p>` (clean as
/// that path) or `--no-filters`, and `-w` to store the object.
async fn hash(
    service: &Services,
    root: &Path,
    data: Vec<u8>,
    flags: Vec<OsString>,
) -> Result<String> {
    let mut args: Vec<OsString> = vec!["hash-object".into(), "--stdin".into()];
    args.extend(flags);
    let oid = git(GitInput {
        service,
        root,
        args,
        input: Some(data),
    })
    .await?;
    Ok(std::str::from_utf8(&oid)?.trim().to_owned())
}

/// `data` with every CR that comes before an LF removed.
pub(crate) fn crlf_to_lf(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut bytes = data.iter().peekable();
    while let Some(&byte) = bytes.next() {
        if byte == b'\r' && bytes.peek() == Some(&&b'\n') {
            continue;
        }
        out.push(byte);
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn crlf_to_lf_keeps_lone_carriage_returns() {
        assert_eq!(super::crlf_to_lf(b"a\r\nb\rc\r\n"), b"a\nb\rc\n");
    }
}
