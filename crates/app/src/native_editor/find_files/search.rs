//! The Find in Files worker: walks the project with the `ignore` crate
//! (Git ignore rules apply) and matches line by line with the Explorer
//! search's regex and file-mask builders. Runs on a pool thread; a
//! cancel flag stops it between files and lines.
use crate::search::{Query, build_regex, build_set, split_patterns, truncate};
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Files larger than this are skipped (generated or data files).
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// One search: where, what, and how many hits to keep at most.
#[derive(Clone, Debug)]
pub(crate) struct SearchRequest {
    pub root: PathBuf,
    pub query: Query,
    pub limit: usize,
}

/// One matching line: 0-based line, UTF-16 column of the first match
/// (for opening the file there), and the trimmed line text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LineHit {
    pub line: u32,
    pub character: u32,
    pub preview: String,
}

/// Hits in one file, in line order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileHits {
    pub path: PathBuf,
    pub hits: Vec<LineHit>,
}

/// Results grouped by file in walk order. `truncated` when the limit
/// stopped the walk; `cancelled` when a newer query did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SearchOutcome {
    pub files: Vec<FileHits>,
    pub total: usize,
    pub truncated: bool,
    pub cancelled: bool,
}

/// Run one search. A bad regex is an error; unreadable, binary, and
/// oversized files are skipped.
pub(crate) fn run(request: &SearchRequest, cancel: &AtomicBool) -> Result<SearchOutcome, String> {
    let regex = build_regex(&request.query)?;
    let include = build_set(&split_patterns(&request.query.include));
    let mut outcome = SearchOutcome::default();
    for entry in WalkBuilder::new(&request.root).build() {
        if cancel.load(Ordering::Relaxed) {
            outcome.cancelled = true;
            return Ok(outcome);
        }
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let path = entry.path();
        let relative = path.strip_prefix(&request.root).unwrap_or(path);
        if include.as_ref().is_some_and(|set| !set.is_match(relative)) {
            continue;
        }
        let Some(text) = read_text(path) else {
            continue;
        };
        let room = request.limit.saturating_sub(outcome.total);
        let hits = file_hits(&text, &regex, room.saturating_add(1));
        if hits.is_empty() {
            continue;
        }
        let over = hits.len() > room;
        let hits: Vec<LineHit> = hits.into_iter().take(room).collect();
        outcome.total = outcome.total.saturating_add(hits.len());
        if !hits.is_empty() {
            outcome.files.push(FileHits {
                path: path.to_path_buf(),
                hits,
            });
        }
        if over {
            outcome.truncated = true;
            break;
        }
    }
    Ok(outcome)
}

/// Text of a searchable file: small enough and not binary.
fn read_text(path: &Path) -> Option<String> {
    if std::fs::metadata(path).ok()?.len() > MAX_FILE_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Up to `limit` matching lines of `text`.
fn file_hits(text: &str, regex: &regex::Regex, limit: usize) -> Vec<LineHit> {
    let mut hits = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if hits.len() >= limit {
            break;
        }
        let Some(found) = regex.find(line) else {
            continue;
        };
        let character = line
            .get(..found.start())
            .map_or(0, |head| head.encode_utf16().count());
        hits.push(LineHit {
            line: u32::try_from(index).unwrap_or(u32::MAX),
            character: u32::try_from(character).unwrap_or(u32::MAX),
            preview: truncate(line),
        });
    }
    hits
}
