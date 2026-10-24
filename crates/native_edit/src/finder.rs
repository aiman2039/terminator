//! Background file index and fuzzy go-to-file.
//!
//! [`FileIndex::scan`] walks workspace roots gitignore-aware (the `ignore`
//! crate: hidden files skipped, `.gitignore`/`.ignore` honored) and caps
//! the file count so pathological trees stay cheap. [`spawn_scan`] runs
//! that walk on a worker thread and delivers the index over a channel;
//! the host polls the receiver once per frame and swaps on arrival, so
//! the UI never blocks on the filesystem. Ranking is a synchronous
//! `nucleo::Matcher` over the indexed display paths.
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;

/// One indexed file: workspace-anchored display path plus absolute path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexedFile {
    pub display: String,
    pub path: PathBuf,
}

/// Cap for a single scan; beyond it the tail is dropped (still sorted).
pub const MAX_INDEX_FILES: usize = 50_000;

#[derive(Clone, Debug, Default)]
pub struct FileIndex {
    files: Vec<IndexedFile>,
}

impl FileIndex {
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// All indexed files in stable (display-sorted) order.
    #[must_use]
    pub fn files(&self) -> &[IndexedFile] {
        &self.files
    }

    /// Blocking scan of `roots`. Missing roots are skipped; the result is
    /// sorted by display path and capped at `max_files`.
    #[must_use]
    pub fn scan(roots: &[PathBuf], max_files: usize) -> Self {
        let mut files = Vec::new();
        // Give every project room in the bounded index. One large checkout
        // must not consume the whole cap before later projects are scanned.
        let per_root = max_files.div_ceil(roots.len().max(1));
        for root in roots {
            let anchor = root.clone();
            let mut taken = 0usize;
            let mut walker = ignore::WalkBuilder::new(root);
            // Ignore files count even outside git checkouts: a file finder
            // must be predictable in plain directories too.
            walker
                .hidden(true)
                .parents(true)
                .git_global(true)
                .require_git(false);
            walker.filter_entry(|entry| {
                if entry.depth() == 0 || !entry.file_type().is_some_and(|kind| kind.is_dir()) {
                    return true;
                }
                !matches!(
                    entry.file_name().to_str(),
                    Some(
                        "target"
                            | "node_modules"
                            | "dist"
                            | "build"
                            | ".next"
                            | ".venv"
                            | "venv"
                            | "coverage"
                    )
                )
            });
            for entry in walker.build().filter_map(Result::ok) {
                if files.len() >= max_files || taken >= per_root {
                    break;
                }
                let path = entry.path().to_path_buf();
                if entry.file_type().is_some_and(|kind| kind.is_file()) {
                    files.push(IndexedFile {
                        display: display_for(&anchor, &path),
                        path,
                    });
                    taken = taken.saturating_add(1);
                }
            }
        }
        files.sort_by(|a, b| a.display.cmp(&b.display));
        files.truncate(max_files);
        Self { files }
    }

    /// Fuzzy-rank display paths against `pattern` (best first, `limit`
    /// entries). An empty pattern returns stable order.
    #[must_use]
    pub fn query(&self, pattern: &str, limit: usize) -> Vec<PathBuf> {
        self.query_scored(pattern, limit)
            .into_iter()
            .map(|(_, file)| file.path)
            .collect()
    }

    /// Ranked file matches with the same score used by other palette
    /// categories. The caller can merge all kinds into one fuzzy list.
    #[must_use]
    pub fn query_scored(&self, pattern: &str, limit: usize) -> Vec<(u16, IndexedFile)> {
        self.query_scored_in(None, &[], pattern, limit)
    }

    /// Same ranking as [`Self::query_scored`], limited to files under `root`.
    /// `None` searches the whole index. `exclude` drops nested roots before
    /// the limit, so those files cannot fill the result list.
    #[must_use]
    pub fn query_scored_in(
        &self,
        root: Option<&Path>,
        exclude: &[PathBuf],
        pattern: &str,
        limit: usize,
    ) -> Vec<(u16, IndexedFile)> {
        let allowed = |path: &Path| {
            root.is_none_or(|root| path.starts_with(root))
                && exclude.iter().all(|child| !path.starts_with(child))
        };
        if pattern.is_empty() {
            return self
                .files
                .iter()
                .filter(|file| allowed(&file.path))
                .take(limit)
                .map(|file| (0, file.clone()))
                .collect();
        }
        let mut matcher = nucleo::Matcher::new(nucleo::Config::DEFAULT);
        let mut needle_buf = Vec::new();
        let mut haystack_buf = Vec::new();
        let needle = nucleo::Utf32Str::new(pattern, &mut needle_buf);
        let mut scored: Vec<(u16, &IndexedFile)> = Vec::new();
        for file in &self.files {
            if !allowed(&file.path) {
                continue;
            }
            let haystack = nucleo::Utf32Str::new(&file.display, &mut haystack_buf);
            if let Some(score) = matcher.fuzzy_match(haystack, needle) {
                scored.push((score, file));
            }
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.display.cmp(&b.1.display)));
        scored
            .into_iter()
            .take(limit)
            .map(|(score, file)| (score, file.clone()))
            .collect()
    }
}

/// Display path anchored at its workspace root when possible, so
/// different projects with same-named files stay distinguishable.
fn display_for(anchor: &Path, path: &Path) -> String {
    match path.strip_prefix(anchor) {
        Ok(relative) if !relative.as_os_str().is_empty() => relative.to_string_lossy().into_owned(),
        _ => path.to_string_lossy().into_owned(),
    }
}

/// Scan `roots` on a worker thread; the index arrives on the receiver.
/// The thread is fire-and-forget: dropping the receiver abandons it.
pub fn spawn_scan(roots: Vec<PathBuf>, max_files: usize) -> mpsc::Receiver<FileIndex> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(FileIndex::scan(&roots, max_files));
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture_tree() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let root = dir.path().join("proj");
        fs::create_dir_all(root.join("src")).expect("mkdir");
        fs::write(root.join("src/main.rs"), "fn main() {}").expect("write");
        fs::write(root.join("src/lib.rs"), "").expect("write");
        fs::write(root.join("README.md"), "").expect("write");
        // Ignored files never enter the index.
        fs::write(root.join(".gitignore"), "ignored/\n").expect("write");
        fs::create_dir_all(root.join("ignored")).expect("mkdir");
        fs::write(root.join("ignored/nope.rs"), "").expect("write");
        (dir, root)
    }

    #[test]
    fn scan_respects_gitignore_and_sorts() {
        let (_dir, root) = fixture_tree();
        let index = FileIndex::scan(&[root], MAX_INDEX_FILES);
        let displays: Vec<&str> = index
            .files()
            .iter()
            .map(|file| file.display.as_str())
            .collect();
        assert!(!displays.iter().any(|path| path.contains("ignored")));
        let main = Path::new("src").join("main.rs");
        assert!(displays.contains(&main.to_str().expect("display path")));
        let mut sorted = displays.clone();
        sorted.sort_unstable();
        assert_eq!(displays, sorted);
    }

    #[test]
    fn query_ranks_best_first() {
        let (_dir, root) = fixture_tree();
        let index = FileIndex::scan(&[root], MAX_INDEX_FILES);
        let hits = index.query("main", 10);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].ends_with("src/main.rs"));
    }

    #[test]
    fn empty_query_returns_stable_order() {
        let (_dir, root) = fixture_tree();
        let index = FileIndex::scan(&[root], MAX_INDEX_FILES);
        let hits = index.query("", 2);
        assert_eq!(hits.len(), 2);
        assert!(hits[0].ends_with("README.md"));
    }

    #[test]
    fn missing_roots_scan_empty() {
        let index = FileIndex::scan(
            &[PathBuf::from("/definitely/not/here-terminator")],
            MAX_INDEX_FILES,
        );
        assert!(index.is_empty());
    }

    #[test]
    fn background_scan_delivers() {
        let (_dir, root) = fixture_tree();
        let rx = spawn_scan(vec![root], MAX_INDEX_FILES);
        let index = rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("index");
        assert!(!index.is_empty());
    }

    #[test]
    fn large_first_project_does_not_hide_later_projects() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        for index in 0..8 {
            fs::write(first.join(format!("{index}.rs")), "").unwrap();
        }
        fs::write(second.join("answer.rs"), "").unwrap();
        let index = FileIndex::scan(&[first, second.clone()], 4);
        assert!(
            index
                .files()
                .iter()
                .any(|file| file.path == second.join("answer.rs"))
        );
        assert_eq!(index.query_scored("answer", 4).len(), 1);
    }

    #[test]
    fn generated_build_trees_are_skipped_without_an_ignore_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("target/debug")).unwrap();
        fs::create_dir_all(root.join("node_modules/dependency")).unwrap();
        fs::write(root.join("src/main.rs"), "").unwrap();
        fs::write(root.join("target/debug/dep-graph.bin"), "").unwrap();
        fs::write(root.join("node_modules/dependency/index.js"), "").unwrap();
        let index = FileIndex::scan(&[root], MAX_INDEX_FILES);
        assert_eq!(index.files().len(), 1);
        assert!(index.files()[0].path.ends_with("src/main.rs"));
    }
}
