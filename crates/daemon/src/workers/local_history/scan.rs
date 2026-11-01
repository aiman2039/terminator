//! Which changed paths Local History records, and the recording itself.
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    ffi::OsStr,
    path::{Component, Path, PathBuf},
    time::Duration,
};
use terminator_core::{CommandOptions, git, local_history};

/// SSH projects use this non-filesystem identity path.
const REMOTE_PREFIX: &str = "/__terminator_remote__";
/// Label of the Git index baseline recorded before a file's first change.
pub(super) const GIT_INDEX: &str = "Git index";

/// Local project folders, without remote identities, missing folders, or
/// folders inside another root (one recursive watch covers them).
pub(super) fn project_roots(paths: impl Iterator<Item = PathBuf>) -> BTreeSet<PathBuf> {
    let candidates: BTreeSet<PathBuf> = paths
        .filter(|path| path.is_absolute() && !path.starts_with(REMOTE_PREFIX) && path.is_dir())
        .collect();
    candidates
        .iter()
        .filter(|path| {
            !candidates
                .iter()
                .any(|other| other != *path && path.starts_with(other))
        })
        .cloned()
        .collect()
}

/// Creations and content or name changes; not metadata or removals.
pub(super) fn content_event(kind: &notify::EventKind) -> bool {
    use notify::{EventKind, event::ModifyKind};
    matches!(
        kind,
        EventKind::Create(_)
            | EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Name(_) | ModifyKind::Any)
            | EventKind::Any
    )
}

/// Paths that are never recorded: anything inside `.git`, and editor swap
/// or backup files.
pub(super) fn eligible_path(path: &Path) -> bool {
    if path
        .components()
        .any(|part| part == Component::Normal(OsStr::new(".git")))
    {
        return false;
    }
    let name = path.file_name().and_then(OsStr::to_str).unwrap_or("");
    !(name.is_empty()
        || name.ends_with('~')
        || name.ends_with(".swp")
        || name.ends_with(".swx")
        || name.starts_with(".terminator-")
        || name == "4913")
}

/// Records each changed text file under one of `roots`. Gitignored files
/// are skipped. A file's first snapshot is preceded by its Git index
/// version when that differs, so the timeline starts before the change.
pub(super) fn record_batch(
    store: &local_history::Store,
    roots: &BTreeSet<PathBuf>,
    paths: Vec<PathBuf>,
) {
    let mut by_root: BTreeMap<&Path, Vec<PathBuf>> = BTreeMap::new();
    for path in paths {
        if path.starts_with(store.root()) || !small_file(&path) {
            continue;
        }
        if let Some(root) = roots.iter().find(|root| path.starts_with(root)) {
            by_root.entry(root.as_path()).or_default().push(path);
        }
    }
    for (root, paths) in by_root {
        let ignored = ignored(root, &paths);
        for path in paths.iter().filter(|path| !ignored.contains(*path)) {
            let Ok(bytes) = std::fs::read(path) else {
                continue;
            };
            if !local_history::recordable(&bytes) {
                continue;
            }
            let now_ms = local_history::now_ms();
            if !store.has_history(path)
                && let Some(base) = index_text(root, path)
                && base != bytes
            {
                let _ = store.record(local_history::Record {
                    path,
                    bytes: &base,
                    source: local_history::Source::Open,
                    label: Some(GIT_INDEX),
                    now_ms: now_ms.saturating_sub(1),
                });
            }
            let _ = store.record(local_history::Record {
                path,
                bytes: &bytes,
                source: local_history::Source::External,
                label: None,
                now_ms,
            });
        }
    }
}

fn small_file(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| {
        meta.is_file()
            && usize::try_from(meta.len()).is_ok_and(|len| len <= local_history::MAX_BYTES)
    })
}

fn options(input: Option<Vec<u8>>) -> CommandOptions {
    CommandOptions {
        timeout: Duration::from_secs(5),
        input,
        stdout_limit: 4 * 1024 * 1024,
        accepted_exit_codes: Some(vec![0, 1]),
        ..CommandOptions::default()
    }
}

/// The subset of `paths` Git ignores. Outside a repository nothing is.
fn ignored(root: &Path, paths: &[PathBuf]) -> HashSet<PathBuf> {
    let mut input = Vec::new();
    for path in paths {
        if let Ok(relative) = path.strip_prefix(root) {
            input.extend_from_slice(relative.as_os_str().as_encoded_bytes());
            input.push(0);
        }
    }
    git::run_blocking(
        root,
        &["check-ignore", "-z", "--stdin"],
        options(Some(input)),
    )
    .map(|out| {
        out.split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
            .map(|part| PathBuf::from(String::from_utf8_lossy(part).into_owned()))
            .map(|path| root.join(path))
            .collect()
    })
    .unwrap_or_default()
}

/// The staged (index) text of `path`, if Git tracks it.
fn index_text(root: &Path, path: &Path) -> Option<Vec<u8>> {
    let relative = path.strip_prefix(root).ok()?;
    let spec = format!(":./{}", relative.to_str()?);
    let mut opts = options(None);
    opts.accepted_exit_codes = Some(vec![0]);
    opts.stdout_limit = local_history::MAX_BYTES.saturating_add(1);
    git::run_blocking(root, &["show", spec.as_str()], opts).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_missing_and_nested_roots_are_not_watched() {
        let dir = tempfile::tempdir().unwrap();
        let outer = dir.path().to_path_buf();
        let inner = outer.join("inner");
        std::fs::create_dir(&inner).unwrap();
        let roots = project_roots(
            [
                inner,
                outer.clone(),
                PathBuf::from("/__terminator_remote__/abc"),
                outer.join("missing"),
            ]
            .into_iter(),
        );
        assert_eq!(roots, BTreeSet::from([outer]));
    }

    #[test]
    fn git_internals_and_swap_files_are_not_recorded() {
        assert!(!eligible_path(Path::new("/p/.git/index")));
        assert!(!eligible_path(Path::new("/p/.a.rs.swp")));
        assert!(!eligible_path(Path::new("/p/a.rs~")));
        assert!(eligible_path(Path::new("/p/src/a.rs")));
    }

    #[test]
    fn a_changed_file_is_recorded_once_and_large_files_are_skipped() {
        let data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let store = local_history::Store::in_data(data.path());
        let roots = BTreeSet::from([project.path().to_path_buf()]);
        let text = project.path().join("a.txt");
        let large = project.path().join("big.txt");
        std::fs::write(&text, "hello").unwrap();
        std::fs::write(&large, vec![b'a'; local_history::MAX_BYTES + 1]).unwrap();
        record_batch(&store, &roots, vec![text.clone(), large.clone()]);
        record_batch(&store, &roots, vec![text.clone()]);
        assert_eq!(store.list(&text).unwrap().len(), 1);
        assert!(!store.has_history(&large));
    }
}
