//! Async directory listings with Git ignore flags.
use super::{DirectoryError, entries_raw};
use std::path::{Path, PathBuf};
use terminator_core::CommandOptions;
use terminator_git::Entry;

pub async fn directory_raw_async(
    service: &crate::gui_services::Services,
    path: PathBuf,
    cancel: &terminator_core::async_service::CancellationToken,
) -> std::result::Result<Vec<Entry>, DirectoryError> {
    let folder = path.clone();
    let operation = cancel.clone();
    service
        .fs()
        .run(cancel, move || entries_raw(&folder, &operation))
        .await
        .map_err(|error| DirectoryError {
            path,
            kind: error
                .downcast_ref::<std::io::Error>()
                .map_or(std::io::ErrorKind::Other, std::io::Error::kind),
            message: error.to_string(),
        })
}

pub(crate) type Listing = (PathBuf, std::result::Result<Vec<Entry>, DirectoryError>);

/// Apply Git ignore rules to every listed directory with one
/// `check-ignore` process from `cwd`, instead of one per directory. A
/// batch Git rejects (a nested repository, say) falls back to the
/// per-directory check, which runs Git from each directory itself.
pub async fn directories_with_ignored_async(
    service: &crate::gui_services::Services,
    cwd: &Path,
    listings: Vec<Listing>,
    in_git: bool,
) -> Vec<Listing> {
    let all: Vec<Entry> = listings
        .iter()
        .filter_map(|(_, entries)| entries.as_ref().ok())
        .flatten()
        .cloned()
        .collect();
    if in_git && listings.len() > 1 && !all.is_empty() {
        let output = terminator_git::run(
            service.processes(),
            service.fs(),
            terminator_git::RunInput {
                cwd,
                args: terminator_git::ignore::CHECK_IGNORE_ARGV
                    .iter()
                    .map(std::ffi::OsString::from)
                    .collect(),
                options: CommandOptions {
                    input: Some(terminator_git::ignore::stdin_batch(&all)),
                    stdout_limit: 4 * 1024 * 1024,
                    accepted_exit_codes: Some(vec![0, 1]),
                    ..Default::default()
                },
            },
        )
        .await;
        if let Ok(output) = output {
            let ignored: std::collections::HashSet<&[u8]> =
                output.split(|byte| *byte == 0).collect();
            return listings
                .into_iter()
                .map(|(path, entries)| {
                    let entries = entries.map(|mut entries| {
                        for entry in &mut entries {
                            entry.ignored |= ignored.contains(
                                terminator_core::os_bytes(entry.path.as_os_str()).as_ref(),
                            );
                        }
                        entries
                    });
                    (path, entries)
                })
                .collect();
        }
    }
    let mut out = Vec::with_capacity(listings.len());
    for (path, entries) in listings {
        let entries = directory_with_ignored_async(service, path.clone(), entries, in_git).await;
        out.push((path, entries));
    }
    out
}

async fn directory_with_ignored_async(
    service: &crate::gui_services::Services,
    path: PathBuf,
    entries: std::result::Result<Vec<Entry>, DirectoryError>,
    in_git: bool,
) -> std::result::Result<Vec<Entry>, DirectoryError> {
    let mut entries = entries?;
    let result = async {
        if in_git {
            let output = terminator_git::run(
                service.processes(),
                service.fs(),
                terminator_git::RunInput {
                    cwd: &path,
                    args: terminator_git::ignore::CHECK_IGNORE_ARGV
                        .iter()
                        .map(std::ffi::OsString::from)
                        .collect(),
                    options: CommandOptions {
                        input: Some(terminator_git::ignore::stdin_batch(&entries)),
                        stdout_limit: 4 * 1024 * 1024,
                        accepted_exit_codes: Some(vec![0, 1]),
                        ..Default::default()
                    },
                },
            )
            .await?;
            for entry in &mut entries {
                entry.ignored |= terminator_git::ignore::is_ignored(&output, &entry.path);
            }
        }
        Ok::<_, anyhow::Error>(entries)
    }
    .await;
    result.map_err(|error| DirectoryError {
        path,
        kind: error
            .downcast_ref::<std::io::Error>()
            .map_or(std::io::ErrorKind::Other, std::io::Error::kind),
        message: error.to_string(),
    })
}
