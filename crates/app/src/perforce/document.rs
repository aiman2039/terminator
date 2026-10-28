//! Native diff document for one Perforce file: have revision vs workspace.
use super::{field, filespec, local_path, number, query, run};
use crate::gui_services::Services;
use anyhow::{Result, ensure};
use serde_json::Value;
use std::path::{Path, PathBuf};
use terminator_core::async_service::CancellationToken;

pub struct DocumentInput<'a> {
    pub service: &'a Services,
    pub cwd: PathBuf,
    pub path: PathBuf,
    pub context: usize,
    pub cancel: &'a CancellationToken,
}

/// A file resolved inside the client: canonical root, client-relative path,
/// and its escaped file spec.
struct Located {
    canonical: PathBuf,
    relative: PathBuf,
    spec: String,
}

pub async fn document(input: DocumentInput<'_>) -> Result<crate::diff::DiffDocument> {
    let DocumentInput {
        service,
        cwd,
        path,
        context,
        cancel,
    } = input;
    let located = locate(service, &cwd, path, cancel).await?;
    ensure_mapped(service, &cwd, &located, cancel).await?;
    let files = query(
        service,
        &cwd,
        &[
            "fstat",
            "-T",
            "clientFile,depotFile,haveRev,action",
            &located.spec,
        ],
        true,
    )
    .await?;
    let record = files
        .iter()
        .find(|record| !field(record, "depotFile").is_empty());
    let left = have_revision(service, &cwd, record).await?;
    ensure!(
        left.len() <= terminator_git::SNAPSHOT_LIMIT,
        "Diff file exceeds 1 MiB"
    );
    let deleted =
        record.is_some_and(|record| matches!(field(record, "action"), "delete" | "move/delete"));
    let right = if deleted {
        Vec::new()
    } else {
        let path = cwd.join(&located.relative);
        service
            .fs()
            .run(cancel, move || {
                terminator_git::worktree_file(&path).map_err(anyhow::Error::from)
            })
            .await?
    };
    let relative = located.relative;
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

async fn locate(
    service: &Services,
    cwd: &Path,
    path: PathBuf,
    cancel: &CancellationToken,
) -> Result<Located> {
    let root = cwd.to_owned();
    service
        .fs()
        .run(cancel, move || {
            let canonical = root.canonicalize()?;
            let relative = terminator_git::relative_path(&canonical, &path)?;
            let spec = filespec(&root.join(&relative))?;
            anyhow::Ok(Located {
                canonical,
                relative,
                spec,
            })
        })
        .await
}

/// fstat reports EV_EMPTY for both new files and paths outside the client
/// view. Verify the mapping before treating an empty result as an addition.
async fn ensure_mapped(
    service: &Services,
    cwd: &Path,
    located: &Located,
    cancel: &CancellationToken,
) -> Result<()> {
    let mappings = query(service, cwd, &["where", &located.spec], false).await?;
    let canonical = located.canonical.clone();
    let expected = canonical.join(&located.relative);
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
        .await
}

/// The synced depot revision's bytes, or empty for a file never synced.
async fn have_revision(service: &Services, cwd: &Path, record: Option<&Value>) -> Result<Vec<u8>> {
    let Some(record) = record.filter(|record| number(record, "haveRev") > 0) else {
        return Ok(Vec::new());
    };
    let depot = field(record, "depotFile");
    run(
        service,
        cwd,
        &[
            "print".into(),
            "-q".into(),
            format!("{depot}#{}", number(record, "haveRev")),
        ],
        false,
    )
    .await
}
