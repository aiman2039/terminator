//! Which changed paths Local History records, and the recording itself.
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
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

/// Build output folders skipped in roots that are not Git repositories,
/// where `.gitignore` cannot be asked (#192).
const BUILD_DIRS: &[&str] = &[
    "target",
    "node_modules",
    "dist",
    "build",
    "__pycache__",
    ".venv",
    ".tox",
    ".next",
    ".gradle",
];
/// Upper bound on Git index text read for one batch of baselines.
const BASELINE_BUDGET: u64 = 16 * 1024 * 1024;

/// Records each changed text file under one of `roots`. Gitignored files
/// (or build outputs outside Git) are skipped. A file's first snapshot is
/// preceded by its Git index version when that differs, so the timeline
/// starts before the change. Baselines come from one `git cat-file` pass.
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
        let paths: Vec<PathBuf> = match ignored(root, &paths) {
            Some(ignored) => paths
                .into_iter()
                .filter(|path| !ignored.contains(path))
                .collect(),
            None if !inside_work_tree(root) => paths
                .into_iter()
                .filter(|path| !build_output(root, path))
                .collect(),
            // One path (for example inside a submodule) failed the whole
            // batch: ask path by path so the others keep their filter.
            None => paths
                .into_iter()
                .filter(|path| match ignored(root, std::slice::from_ref(path)) {
                    Some(ignored) => ignored.is_empty(),
                    None => !build_output(root, path),
                })
                .collect(),
        };
        let first: Vec<&Path> = paths
            .iter()
            .filter(|path| !store.has_history(path))
            .map(PathBuf::as_path)
            .collect();
        let baselines = index_texts(root, &first);
        for path in &paths {
            record_one(store, path, baselines.get(path));
        }
    }
}

fn record_one(store: &local_history::Store, path: &Path, base: Option<&Vec<u8>>) {
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    if !local_history::recordable(&bytes) {
        return;
    }
    let now_ms = local_history::now_ms();
    if let Some(base) = base
        && *base != bytes
        && !store.has_history(path)
    {
        let _ = store.record(local_history::Record {
            path,
            bytes: base,
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

/// Whether `path` lies in a common build output folder below `root`.
fn build_output(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root).is_ok_and(|relative| {
        relative.components().any(|part| {
            matches!(part, Component::Normal(name)
                if name.to_str().is_some_and(|name| BUILD_DIRS.contains(&name)))
        })
    })
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

/// Whether `root` is inside a Git work tree.
fn inside_work_tree(root: &Path) -> bool {
    git::run_blocking(root, &["rev-parse", "--is-inside-work-tree"], options(None))
        .is_ok_and(|out| out.trim_ascii() == b"true")
}

/// The subset of `paths` Git ignores, or `None` when Git cannot tell
/// (outside a repository, a path inside a submodule, a timeout).
fn ignored(root: &Path, paths: &[PathBuf]) -> Option<HashSet<PathBuf>> {
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
    .ok()
}

/// The staged (index) text of each tracked path, read with one
/// `cat-file --batch-check` and one `cat-file --batch` process. Blobs over
/// `MAX_BYTES` or past `BASELINE_BUDGET` are left out.
fn index_texts(root: &Path, paths: &[&Path]) -> HashMap<PathBuf, Vec<u8>> {
    let specs: Vec<(&Path, String)> = paths
        .iter()
        .filter_map(|path| {
            let relative = path.strip_prefix(root).ok()?.to_str()?;
            (!relative.contains('\n')).then(|| (*path, format!(":./{relative}")))
        })
        .collect();
    if specs.is_empty() {
        return HashMap::new();
    }
    let input = specs
        .iter()
        .flat_map(|(_, spec)| format!("{spec}\n").into_bytes());
    let Ok(check) = git::run_blocking(
        root,
        &["cat-file", "--batch-check"],
        options(Some(input.collect())),
    ) else {
        return HashMap::new();
    };
    let mut budget = BASELINE_BUDGET;
    let mut wanted: Vec<(&Path, String)> = Vec::new();
    for ((path, _), line) in specs.iter().zip(check.split(|byte| *byte == b'\n')) {
        if let Some((oid, size)) = blob_header(line)
            && size <= local_history::MAX_BYTES as u64
            && let Some(left) = budget.checked_sub(size)
        {
            budget = left;
            wanted.push((path, oid));
        }
    }
    let blobs = blobs(root, &wanted, BASELINE_BUDGET.saturating_sub(budget));
    wanted
        .into_iter()
        .filter_map(|(path, oid)| Some((path.to_path_buf(), blobs.get(&oid)?.clone())))
        .collect()
}

/// `(oid, size)` from a `cat-file` header line naming a blob.
fn blob_header(line: &[u8]) -> Option<(String, u64)> {
    let line = std::str::from_utf8(line).ok()?;
    let parts: Vec<&str> = line.split(' ').collect();
    match parts.as_slice() {
        [oid, "blob", size] => Some(((*oid).to_owned(), size.parse().ok()?)),
        _ => None,
    }
}

/// Contents of the blobs `wanted` names, keyed by object id.
fn blobs(root: &Path, wanted: &[(&Path, String)], bytes: u64) -> HashMap<String, Vec<u8>> {
    if wanted.is_empty() {
        return HashMap::new();
    }
    let input: Vec<u8> = wanted
        .iter()
        .flat_map(|(_, oid)| format!("{oid}\n").into_bytes())
        .collect();
    let mut opts = options(Some(input));
    opts.accepted_exit_codes = Some(vec![0]);
    opts.stdout_limit = usize::try_from(bytes)
        .unwrap_or(usize::MAX)
        .saturating_add(wanted.len().saturating_mul(128));
    let Ok(out) = git::run_blocking(root, &["cat-file", "--batch"], opts) else {
        return HashMap::new();
    };
    parse_batch(&out)
}

/// Splits `cat-file --batch` output into object id and contents.
fn parse_batch(mut out: &[u8]) -> HashMap<String, Vec<u8>> {
    let mut blobs = HashMap::new();
    while let Some(end) = out.iter().position(|byte| *byte == b'\n') {
        let (header, rest) = out.split_at(end);
        let rest = rest.get(1..).unwrap_or_default();
        let Some((oid, size)) = blob_header(header) else {
            break;
        };
        let Some(body) = usize::try_from(size).ok().and_then(|n| rest.get(..n)) else {
            break;
        };
        blobs.insert(oid, body.to_vec());
        out = rest.get(body.len().saturating_add(1)..).unwrap_or_default();
    }
    blobs
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

    #[test]
    fn build_outputs_outside_git_are_not_recorded() {
        let data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let store = local_history::Store::in_data(data.path());
        let roots = BTreeSet::from([project.path().to_path_buf()]);
        std::fs::create_dir_all(project.path().join("target/debug")).unwrap();
        let output = project.path().join("target/debug/out.txt");
        let source = project.path().join("main.rs");
        std::fs::write(&output, "built").unwrap();
        std::fs::write(&source, "fn main() {}").unwrap();
        record_batch(&store, &roots, vec![output.clone(), source.clone()]);
        assert!(!store.has_history(&output));
        assert!(store.has_history(&source));
    }

    #[test]
    fn a_submodule_path_in_the_batch_keeps_gitignored_files_out() {
        let data = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let root = project.path();
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(root)
                .output()
                .is_ok_and(|out| out.status.success())
        };
        if !git(&["init", "-q"]) {
            return; // No git on this machine.
        }
        std::fs::write(root.join(".gitignore"), ".env\n").unwrap();
        std::fs::create_dir(root.join("sub")).unwrap();
        let oid = "0123456789abcdef0123456789abcdef01234567";
        assert!(git(&[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{oid},sub")
        ]));
        let in_submodule = root.join("sub/a.txt");
        let secret = root.join(".env");
        let source = root.join("main.rs");
        std::fs::write(&in_submodule, "sub").unwrap();
        std::fs::write(&secret, "TOKEN=1").unwrap();
        std::fs::write(&source, "fn main() {}").unwrap();
        let store = local_history::Store::in_data(data.path());
        let roots = BTreeSet::from([root.to_path_buf()]);
        record_batch(
            &store,
            &roots,
            vec![in_submodule.clone(), secret.clone(), source.clone()],
        );
        assert!(!store.has_history(&secret), "gitignored file stays out");
        assert!(store.has_history(&source));
        assert!(store.has_history(&in_submodule));
    }

    #[test]
    fn batch_output_is_split_into_blobs_by_object_id() {
        let out = b"aaa blob 5\nhello\nbbb blob 0\n\nccc tree 3\nxyz\n";
        let blobs = parse_batch(out);
        assert_eq!(blobs.get("aaa").unwrap(), b"hello");
        assert_eq!(blobs.get("bbb").unwrap(), b"");
        assert!(!blobs.contains_key("ccc"));
        assert_eq!(blob_header(b":./gone missing"), None);
    }
}
