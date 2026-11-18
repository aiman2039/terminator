//! New task setup: copy the ignored files that a `.worktreeinclude` file
//! names (gitignore patterns, such as `.env`) from the source checkout into
//! the new worktree. It runs on a worker after `git worktree add`, never on
//! the GUI thread and never in the daemon. It never follows a symbolic
//! link, never leaves the source checkout or the new worktree, and never
//! overwrites a file. Tracked files are already in the worktree; only files
//! Git ignores are candidates, and the patterns decide each file on its
//! own path, so a pattern can name one file inside an ignored folder and a
//! `!` pattern can keep one file of an included folder behind.
use std::path::{Component, Path};
use std::sync::Arc;

/// The pattern file, at the root of the source checkout.
pub(crate) const INCLUDE_FILE: &str = ".worktreeinclude";

/// What one copy did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CopyReport {
    pub copied: usize,
    /// Files left alone: they already exist in the worktree, or they are
    /// symbolic links.
    pub kept: usize,
    pub errors: Vec<String>,
}

impl CopyReport {
    /// One status line, or `None` when there was nothing to copy.
    pub(crate) fn summary(&self) -> Option<String> {
        if self.copied == 0 && self.kept == 0 && self.errors.is_empty() {
            return None;
        }
        let mut text = format!("{INCLUDE_FILE}: copied {} file(s)", self.copied);
        if self.kept > 0 {
            text.push_str(&format!(", kept {} existing or linked", self.kept));
        }
        if let Some(first) = self.errors.first() {
            text.push_str(&format!(", {} failed ({first})", self.errors.len()));
        }
        Some(text)
    }
}

/// Copy what `source/.worktreeinclude` names into `dest`.
pub(crate) fn copy_included(source: &Path, dest: &Path) -> CopyReport {
    let mut report = CopyReport::default();
    let (Ok(source), Ok(dest)) = (std::fs::canonicalize(source), std::fs::canonicalize(dest))
    else {
        return report;
    };
    let include = source.join(INCLUDE_FILE);
    // A linked pattern file could point anywhere: only a real file counts.
    if !std::fs::symlink_metadata(&include).is_ok_and(|meta| meta.is_file()) {
        return report;
    }
    let Some(rules) = Rules::load(&source, &include, &mut report) else {
        return report;
    };
    let entries = match ignored_entries(&source) {
        Ok(entries) => entries,
        Err(error) => {
            report.errors.push(error);
            return report;
        }
    };
    let rules = Arc::new(rules);
    for entry in entries {
        let dir = entry.ends_with('/');
        let relative = Path::new(entry.trim_end_matches('/'));
        if !inside(relative) {
            continue;
        }
        let wanted = if dir {
            rules.worth_walking(relative)
        } else {
            rules.includes(relative)
        };
        if wanted {
            copy_entry((&source, &dest), relative, &rules, &mut report);
        }
    }
    report
}

/// The `.worktreeinclude` patterns: the matcher that decides each file, and
/// what each pattern can reach, so a walk skips folders no pattern can
/// match below (such as `node_modules` for a pattern `/config/*.env`).
struct Rules {
    matcher: ignore::gitignore::Gitignore,
    reach: Vec<Reach>,
}

impl Rules {
    fn load(source: &Path, include: &Path, report: &mut CopyReport) -> Option<Self> {
        let text = match std::fs::read_to_string(include) {
            Ok(text) => text,
            Err(error) => {
                report.errors.push(format!("{INCLUDE_FILE}: {error}"));
                return None;
            }
        };
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let mut builder = ignore::gitignore::GitignoreBuilder::new(source);
        let mut reach = Vec::new();
        for (number, line) in (1_usize..).zip(text.lines()) {
            match builder.add_line(None, line) {
                Ok(_) => reach.extend(Reach::parse(line)),
                Err(error) => report
                    .errors
                    .push(format!("{INCLUDE_FILE}:{number}: {error}")),
            }
        }
        match builder.build() {
            Ok(matcher) if !matcher.is_empty() => Some(Self { matcher, reach }),
            Ok(_) => None,
            Err(error) => {
                report.errors.push(error.to_string());
                None
            }
        }
    }

    /// Whether the file at `relative` is to be copied. The file's own rules
    /// win over its folders' rules, so `!secrets/private.key` keeps one file
    /// of an included `secrets/` behind.
    fn includes(&self, relative: &Path) -> bool {
        self.matcher
            .matched_path_or_any_parents(relative, false)
            .is_ignore()
    }

    /// Whether a file below the folder `relative` can be included: the
    /// folder (or a parent) is included, or a pattern can match below it.
    fn worth_walking(&self, relative: &Path) -> bool {
        self.matcher
            .matched_path_or_any_parents(relative, true)
            .is_ignore()
            || self.reach.iter().any(|reach| reach.below(relative))
    }
}

/// Where one including (not `!`) pattern can match.
enum Reach {
    /// No slash inside: it matches a name at any depth. Also used for any
    /// pattern this check does not read exactly (escapes).
    Anywhere,
    /// Anchored at the source root: one entry per path part.
    Under(Vec<Segment>),
}

enum Segment {
    Literal(String),
    Glob(globset::GlobMatcher),
    /// `**`: any number of parts.
    Deep,
}

impl Reach {
    /// Mirrors `GitignoreBuilder::add_line`; `None` for a comment, a blank
    /// line or a `!` pattern (it can only exclude).
    fn parse(line: &str) -> Option<Self> {
        if line.starts_with('#') {
            return None;
        }
        if line.contains('\\') {
            return Some(Self::Anywhere);
        }
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('!') {
            return None;
        }
        let (anchored, line) = match line.strip_prefix('/') {
            Some(rest) => (true, rest),
            None => (false, line),
        };
        let line = line.strip_suffix('/').unwrap_or(line);
        if !anchored && !line.contains('/') {
            return Some(Self::Anywhere);
        }
        Some(Self::Under(line.split('/').map(Segment::parse).collect()))
    }

    /// Whether this pattern can match a path strictly below the folder `dir`.
    fn below(&self, dir: &Path) -> bool {
        let Self::Under(segments) = self else {
            return true;
        };
        let mut rest = segments.iter();
        for part in dir.components() {
            let Some(segment) = rest.next() else {
                return false;
            };
            if matches!(segment, Segment::Deep) {
                return true;
            }
            let Some(name) = part.as_os_str().to_str() else {
                return true;
            };
            if !segment.admits(name) {
                return false;
            }
        }
        // A segment is left: the pattern names a path below `dir`.
        rest.next().is_some()
    }
}

impl Segment {
    fn parse(text: &str) -> Self {
        if text.contains("**") {
            return Self::Deep;
        }
        if !text.contains(['*', '?', '[']) {
            return Self::Literal(text.to_owned());
        }
        match globset::GlobBuilder::new(text)
            .literal_separator(true)
            .build()
        {
            Ok(glob) => Self::Glob(glob.compile_matcher()),
            // Unsure: let the walk go on.
            Err(_) => Self::Deep,
        }
    }

    fn admits(&self, name: &str) -> bool {
        match self {
            Self::Literal(literal) => literal == name,
            Self::Glob(glob) => glob.is_match(name),
            Self::Deep => true,
        }
    }
}

/// Ignored files and folders (`--directory`: an ignored folder is one
/// entry), relative to `source`. Read-only Git.
fn ignored_entries(source: &Path) -> Result<Vec<String>, String> {
    let args = [
        "ls-files",
        "--others",
        "--ignored",
        "--exclude-standard",
        "--directory",
        "-z",
    ];
    let options = terminator_core::CommandOptions {
        stdout_limit: 4 * 1024 * 1024,
        timeout: std::time::Duration::from_secs(20),
        ..Default::default()
    };
    let bytes = terminator_core::git::run_blocking(source, &args, options)
        .map_err(|error| format!("git ls-files: {error}"))?;
    Ok(String::from_utf8_lossy(&bytes)
        .split('\0')
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect())
}

/// A relative path with only normal parts: no `..`, root, or prefix.
fn inside(relative: &Path) -> bool {
    !relative.as_os_str().is_empty()
        && relative
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

/// Copy one ignored file, or every included file below one ignored folder.
fn copy_entry(
    (source, dest): (&Path, &Path),
    relative: &Path,
    rules: &Arc<Rules>,
    report: &mut CopyReport,
) {
    let start = source.join(relative);
    // A linked folder on the way could lead out of the source checkout.
    if !std::fs::canonicalize(&start).is_ok_and(|real| real.starts_with(source)) {
        report.kept = report.kept.saturating_add(1);
        return;
    }
    let (root, prune) = (source.to_path_buf(), Arc::clone(rules));
    let walk = ignore::WalkBuilder::new(&start)
        .standard_filters(false)
        .follow_links(false)
        .filter_entry(move |item| {
            !item.file_type().is_some_and(|kind| kind.is_dir())
                || item
                    .path()
                    .strip_prefix(&root)
                    .is_ok_and(|part| prune.worth_walking(part))
        })
        .build();
    for item in walk {
        let item = match item {
            Ok(item) => item,
            Err(error) => {
                report.errors.push(error.to_string());
                continue;
            }
        };
        let Some(kind) = item.file_type() else {
            continue;
        };
        if kind.is_dir() {
            continue;
        }
        let Ok(part) = item.path().strip_prefix(source) else {
            continue;
        };
        if !rules.includes(part) {
            continue;
        }
        if !kind.is_file() {
            // Symbolic links stay behind: they can point anywhere.
            report.kept = report.kept.saturating_add(1);
            continue;
        }
        copy_file(item.path(), (dest, part), report);
    }
}

/// Copy `from` to `dest/part`, never over an existing file and never
/// through a link out of `dest`. The copy is staged next to the target and
/// published whole, so a failed copy leaves no file at the target.
pub(super) fn copy_file(from: &Path, (dest, part): (&Path, &Path), report: &mut CopyReport) {
    let target = dest.join(part);
    let result = (|| -> std::io::Result<bool> {
        let Some(parent) = target.parent() else {
            return Ok(false);
        };
        // A linked folder already in the worktree could lead out of it:
        // check before creating any folder below it.
        let existing = parent.ancestors().find(|dir| dir.exists());
        if !existing
            .and_then(|dir| std::fs::canonicalize(dir).ok())
            .is_some_and(|real| real.starts_with(dest))
        {
            return Ok(false);
        }
        std::fs::create_dir_all(parent)?;
        if !std::fs::canonicalize(parent)?.starts_with(dest) {
            return Ok(false);
        }
        // Skip the copy when the name is taken; the publish checks again.
        if std::fs::symlink_metadata(&target).is_ok() {
            return Ok(false);
        }
        let mut input = std::fs::File::open(from)?;
        let permissions = input.metadata()?.permissions();
        // Dropped on any error below: the staged file is removed.
        let mut staged = tempfile::NamedTempFile::new_in(parent)?;
        std::io::copy(&mut input, staged.as_file_mut())?;
        staged.as_file().set_permissions(permissions)?;
        match staged.persist_noclobber(&target) {
            Ok(_) => Ok(true),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
            Err(error) => Err(error.error),
        }
    })();
    match result {
        Ok(true) => report.copied = report.copied.saturating_add(1),
        Ok(false) => report.kept = report.kept.saturating_add(1),
        Err(error) => report.errors.push(format!("{}: {error}", part.display())),
    }
}
