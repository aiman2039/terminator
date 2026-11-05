//! The Find in Files worker: walks the folder with the `ignore` crate (Git
//! ignore rules apply, dotfiles stay unless ignored, version control
//! metadata such as `.git` and `.svn` is skipped),
//! keeps the files the file mask allows and Exclude does not, and matches
//! line by line with the Explorer search's regex and glob builders. It runs
//! on a pool thread and hands each file's hits to a sink as soon as that
//! file is done; a cancel flag stops it between files and lines.
use crate::search::{Query, build_set, build_typed_regex, split_patterns, truncate};
use globset::GlobSet;
use ignore::WalkBuilder;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Files larger than this are skipped (generated or data files).
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// Lines kept above and below each hit for the preview.
pub(crate) const CONTEXT_LINES: u32 = 5;
/// Preview lines longer than this are cut (minified files).
pub(super) const MAX_PREVIEW_CHARS: usize = 400;
/// Matches highlighted per line at most.
const MAX_MATCHES: usize = 16;

/// One search: where, what, and how many hits to keep at most.
#[derive(Clone, Debug)]
pub(crate) struct SearchRequest {
    /// Relative paths (and so the globs) start here.
    pub root: PathBuf,
    /// The walk starts here: `root`, or a folder below it.
    pub folder: PathBuf,
    pub query: Query,
    pub limit: usize,
}

/// One matching line: 0-based line, UTF-16 column of the first match
/// (for opening the file there), the trimmed line text, and the char
/// ranges of the matches in that text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LineHit {
    pub line: u32,
    pub character: u32,
    pub preview: String,
    pub matches: Vec<(usize, usize)>,
}

/// Hits in one file, in line order, and the lines around them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileHits {
    pub path: PathBuf,
    pub hits: Vec<LineHit>,
    /// Hit lines and up to [`CONTEXT_LINES`] around each, by 0-based line,
    /// untrimmed (long lines cut): what the preview shows.
    pub lines: BTreeMap<u32, String>,
}

/// How a walk ended: hits sent, whether the limit stopped it, and whether
/// a newer query (or a closed window) did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Finish {
    pub total: usize,
    pub truncated: bool,
    pub cancelled: bool,
}

/// Run one search and hand each file with hits to `sink`, in walk order.
/// `sink` returns false when nobody listens any more, which stops the walk
/// like `cancel`. A bad regex is an error; unreadable, binary, and
/// oversized files are skipped.
pub(crate) fn run(
    request: &SearchRequest,
    cancel: &AtomicBool,
    sink: &mut dyn FnMut(FileHits) -> bool,
) -> Result<Finish, String> {
    let regex = build_typed_regex(&request.query)?;
    let include = build_set(&split_patterns(&request.query.include));
    let exclude = build_set(&split_patterns(&request.query.exclude));
    let mut finish = Finish::default();
    for entry in walker(request, exclude).build() {
        if cancel.load(Ordering::Relaxed) {
            finish.cancelled = true;
            break;
        }
        let Some(path) = wanted_file(entry, &request.root, include.as_ref()) else {
            continue;
        };
        let Some(text) = read_text(&path) else {
            continue;
        };
        let room = request.limit.saturating_sub(finish.total);
        let Some(mut hits) = file_hits(&text, &regex, room.saturating_add(1), cancel) else {
            finish.cancelled = true;
            break;
        };
        finish.truncated = hits.len() > room;
        hits.truncate(room);
        if !hits.is_empty() {
            finish.total = finish.total.saturating_add(hits.len());
            let lines = context_lines(&text, &hits);
            if !sink(FileHits { path, hits, lines }) {
                finish.cancelled = true;
                break;
            }
        }
        if finish.truncated {
            break;
        }
    }
    Ok(finish)
}

/// A Git-ignore-aware walk of the request's folder. It keeps dotfiles,
/// skips version control metadata (a `.git` folder, a linked worktree's
/// `.git` file, `.svn` pristine copies, …), and prunes what `exclude`
/// matches by path below the root or by name.
fn walker(request: &SearchRequest, exclude: Option<GlobSet>) -> WalkBuilder {
    let mut walker = WalkBuilder::new(&request.folder);
    walker.hidden(false);
    let root = request.root.clone();
    walker.filter_entry(move |entry| {
        if entry.depth() == 0 {
            return true;
        }
        if terminator_native_edit::finder::is_vcs_metadata(entry.file_name()) {
            return false;
        }
        let Some(exclude) = exclude.as_ref() else {
            return true;
        };
        let path = entry.path();
        let relative = path.strip_prefix(&root).unwrap_or(path);
        !exclude.is_match(relative) && !exclude.is_match(Path::new(entry.file_name()))
    });
    walker
}

/// A walked file the file mask allows (matched below `root`).
fn wanted_file(
    entry: Result<ignore::DirEntry, ignore::Error>,
    root: &Path,
    include: Option<&GlobSet>,
) -> Option<PathBuf> {
    let entry = entry.ok()?;
    if !entry.file_type().is_some_and(|kind| kind.is_file()) {
        return None;
    }
    let path = entry.into_path();
    let allowed = include.is_none_or(|set| set.is_match(path.strip_prefix(root).unwrap_or(&path)));
    allowed.then_some(path)
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

/// Up to `limit` matching lines of `text`, or `None` when `cancel` is set
/// during the scan.
pub(super) fn file_hits(
    text: &str,
    regex: &regex::Regex,
    limit: usize,
    cancel: &AtomicBool,
) -> Option<Vec<LineHit>> {
    let mut hits = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        if hits.len() >= limit {
            break;
        }
        let Some(found) = regex.find(line) else {
            continue;
        };
        let character = line
            .get(..found.start())
            .map_or(0, |head| head.encode_utf16().count());
        let preview = truncate(line);
        let matches = preview_matches(line, regex, preview.chars().count());
        hits.push(LineHit {
            line: u32::try_from(index).unwrap_or(u32::MAX),
            character: u32::try_from(character).unwrap_or(u32::MAX),
            preview,
            matches,
        });
    }
    Some(hits)
}

/// Char ranges of the matches in `line`, moved onto its trimmed preview
/// of `shown` chars (see [`truncate`]). Matches past the preview end are
/// left out.
fn preview_matches(line: &str, regex: &regex::Regex, shown: usize) -> Vec<(usize, usize)> {
    let indent = line.chars().take_while(|c| c.is_whitespace()).count();
    let mut ranges = Vec::new();
    // Count chars once, from one match to the next.
    let (mut byte, mut char_at) = (0usize, 0usize);
    for found in regex.find_iter(line).take(MAX_MATCHES) {
        let (Some(gap), Some(body)) = (
            line.get(byte..found.start()),
            line.get(found.start()..found.end()),
        ) else {
            break;
        };
        char_at = char_at.saturating_add(gap.chars().count());
        byte = found.start();
        let start = char_at.saturating_sub(indent);
        if start >= shown {
            break;
        }
        let end = char_at
            .saturating_add(body.chars().count())
            .saturating_sub(indent);
        ranges.push((start, end.min(shown)));
    }
    ranges
}

/// The hit lines and up to [`CONTEXT_LINES`] around each, by line.
fn context_lines(text: &str, hits: &[LineHit]) -> BTreeMap<u32, String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut kept = BTreeMap::new();
    for hit in hits {
        let from = hit.line.saturating_sub(CONTEXT_LINES);
        for number in from..=hit.line.saturating_add(CONTEXT_LINES) {
            let Some(line) = usize::try_from(number)
                .ok()
                .and_then(|index| lines.get(index))
            else {
                break;
            };
            kept.entry(number)
                .or_insert_with(|| line.chars().take(MAX_PREVIEW_CHARS).collect());
        }
    }
    kept
}
