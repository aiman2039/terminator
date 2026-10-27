use super::*;
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use terminator_core::remote::{Change, MAX_FILE_BYTES};
fn git(
    project: &RemoteProject,
    control: Option<&Path>,
    args: &[String],
    limit: usize,
) -> Result<Vec<u8>> {
    let mut parts = vec![
        "git".into(),
        "--no-pager".into(),
        "--no-optional-locks".into(),
        "-c".into(),
        "core.fsmonitor=false".into(),
        "-c".into(),
        "core.hooksPath=/dev/null".into(),
        "-c".into(),
        "status.renames=false".into(),
        "-C".into(),
        project.root.clone(),
    ];
    parts.extend_from_slice(args);
    execute(
        project,
        control,
        &format!(
            "unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES; exec {}",
            parts
                .iter()
                .map(|p| terminator_core::quote(p))
                .collect::<Vec<_>>()
                .join(" ")
        ),
        limit,
    )
}
fn changes(project: &RemoteProject, control: Option<&Path>) -> Result<Vec<Change>> {
    let prefix = git(
        project,
        control,
        &["rev-parse".into(), "--show-prefix".into()],
        8192,
    )?;
    let prefix = std::str::from_utf8(&prefix)?
        .trim_end_matches('\n')
        .to_owned();
    let output = git(
        project,
        control,
        &[
            "status".into(),
            "--porcelain=v1".into(),
            "-z".into(),
            "--untracked-files=all".into(),
            "--".into(),
            ".".into(),
        ],
        1024 * 1024,
    )?;
    std::str::from_utf8(&output).context("Remote Git paths must be UTF-8")?;
    let changes = terminator_git::parse_porcelain(std::path::Path::new(""), &output);
    ensure!(
        changes.len() <= 1000,
        "Remote Git status exceeds 1000 paths"
    );
    changes
        .into_iter()
        .map(|change| {
            let raw = change
                .path
                .to_str()
                .context("Remote Git path must be UTF-8")?;
            let path = raw
                .strip_prefix(&prefix)
                .context("Git path is outside the project root")?
                .to_owned();
            terminator_core::remote::validate_relative(&path)?;
            Ok(Change {
                path,
                status: change.status,
            })
        })
        .collect()
}
pub(super) fn status(project: &RemoteProject, control: Option<&Path>) -> Result<Reply> {
    let branch = git(
        project,
        control,
        &["branch".into(), "--show-current".into()],
        8192,
    )?;
    Ok(Reply::Git {
        branch: {
            let name = std::str::from_utf8(&branch)?.trim();
            if name.is_empty() {
                "(detached HEAD)".into()
            } else {
                name.into()
            }
        },
        changes: changes(project, control)?,
    })
}
pub(super) fn diff(
    project: &RemoteProject,
    control: Option<&Path>,
    path: &str,
    staged: bool,
) -> Result<Reply> {
    terminator_core::remote::validate_relative(path)?;
    let status = changes(project, control)?
        .into_iter()
        .find(|c| c.path == path)
        .context("Remote file is no longer modified; refresh Git status")?;
    let x = status.status.as_bytes().first().copied().unwrap_or(b' ');
    let y = status.status.as_bytes().get(1).copied().unwrap_or(b' ');
    let prefix = git(
        project,
        control,
        &["rev-parse".into(), "--show-prefix".into()],
        8192,
    )?;
    let prefix = std::str::from_utf8(&prefix)?.trim_end_matches('\n');
    let blob = |revision: &str| {
        git(
            project,
            control,
            &[
                "show".into(),
                "--no-ext-diff".into(),
                "--no-textconv".into(),
                format!("{revision}:{prefix}{path}"),
            ],
            MAX_FILE_BYTES,
        )
    };
    let (left, right) = if staged {
        ensure!(x != b'?' && x != b' ', "File has no staged change");
        (
            if x == b'A' { Vec::new() } else { blob("HEAD")? },
            if x == b'D' { Vec::new() } else { blob("")? },
        )
    } else {
        let left = if x == b'?' || x == b'D' {
            Vec::new()
        } else {
            blob("")?
        };
        let right = if y == b'D' {
            Vec::new()
        } else {
            match super::files::operation(project, control, Operation::Read { path: path.into() })?
            {
                Reply::File { base64, .. } => B64.decode(base64)?,
                _ => anyhow::bail!("Unexpected remote file response"),
            }
        };
        (left, right)
    };
    ensure!(
        left.len() <= MAX_FILE_BYTES && right.len() <= MAX_FILE_BYTES,
        "Remote diff exceeds its snapshot limit"
    );
    Ok(Reply::Diff {
        path: path.into(),
        left: B64.encode(left),
        right: B64.encode(right),
        staged,
    })
}
