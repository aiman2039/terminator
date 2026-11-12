//! New task setup: copy the ignored files that a `.worktreeinclude` file
//! names (gitignore patterns, such as `.env`) from the source checkout into
//! the new worktree. It runs on a worker after `git worktree add`, never on
//! the GUI thread and never in the daemon. It never follows a symbolic
//! link, never leaves the source checkout or the new worktree, and never
//! overwrites a file. Tracked files are already in the worktree; only files
//! Git ignores are candidates.
use std::path::{Component, Path};

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
    let mut builder = ignore::gitignore::GitignoreBuilder::new(&source);
    if let Some(error) = builder.add(&include) {
        report.errors.push(error.to_string());
    }
    let matcher = match builder.build() {
        Ok(matcher) if !matcher.is_empty() => matcher,
        Ok(_) => return report,
        Err(error) => {
            report.errors.push(error.to_string());
            return report;
        }
    };
    let entries = match ignored_entries(&source) {
        Ok(entries) => entries,
        Err(error) => {
            report.errors.push(error);
            return report;
        }
    };
    for entry in entries {
        let dir = entry.ends_with('/');
        let relative = Path::new(entry.trim_end_matches('/'));
        if inside(relative)
            && matcher
                .matched_path_or_any_parents(relative, dir)
                .is_ignore()
        {
            copy_entry(&source, &dest, relative, &mut report);
        }
    }
    report
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

/// Copy one ignored file, or every file below one ignored folder.
fn copy_entry(source: &Path, dest: &Path, relative: &Path, report: &mut CopyReport) {
    let start = source.join(relative);
    // A linked folder on the way could lead out of the source checkout.
    if !std::fs::canonicalize(&start).is_ok_and(|real| real.starts_with(source)) {
        report.kept = report.kept.saturating_add(1);
        return;
    }
    let walk = ignore::WalkBuilder::new(&start)
        .standard_filters(false)
        .follow_links(false)
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
        if !kind.is_file() {
            // Symbolic links stay behind: they can point anywhere.
            report.kept = report.kept.saturating_add(1);
            continue;
        }
        let Ok(part) = item.path().strip_prefix(source) else {
            continue;
        };
        copy_file(item.path(), (dest, part), report);
    }
}

/// Copy `from` to `dest/part`, never over an existing file and never
/// through a link out of `dest`.
fn copy_file(from: &Path, (dest, part): (&Path, &Path), report: &mut CopyReport) {
    let target = dest.join(part);
    let result = (|| -> std::io::Result<bool> {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
            if !std::fs::canonicalize(parent)?.starts_with(dest) {
                return Ok(false);
            }
        }
        let mut input = std::fs::File::open(from)?;
        let permissions = input.metadata()?.permissions();
        let mut output = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(output) => output,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
            Err(error) => return Err(error),
        };
        std::io::copy(&mut input, &mut output)?;
        output.set_permissions(permissions)?;
        Ok(true)
    })();
    match result {
        Ok(true) => report.copied = report.copied.saturating_add(1),
        Ok(false) => report.kept = report.kept.saturating_add(1),
        Err(error) => report.errors.push(format!("{}: {error}", part.display())),
    }
}
