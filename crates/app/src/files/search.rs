//! Content search for the Explorer's Contents mode.
//!
//! Walking uses the `ignore` crate so repository `.gitignore` rules apply the
//! same way the file tree already treats them. Matching is `regex`-based; a
//! plain query is escaped, and match case, whole word, and regex modes map to
//! the search bar toggles.
use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;
use regex::{Regex, RegexBuilder};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub match_case: bool,
    pub whole_word: bool,
    pub use_regex: bool,
    pub include: String,
    pub exclude: String,
}

impl Query {
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    pub path: PathBuf,
    pub line: u32,
    pub text: String,
}

/// Bound the work and the painted list; a huge repo should not stall the GUI.
pub const MAX_HITS: usize = 2000;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_LINE_CHARS: usize = 200;

fn split_patterns(input: &str) -> Vec<&str> {
    input
        .split([',', ' ', '\n'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect()
}

/// Build a glob set from Orca-style include/exclude patterns. A pattern is
/// matched against the relative path both as written and as `**/<pattern>`, so
/// `*.ts` finds nested files and `src/**` keeps a subtree.
fn build_set(patterns: &[&str]) -> Option<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    let mut any = false;
    for pattern in patterns {
        let mut candidates = vec![(*pattern).to_string()];
        if !pattern.contains('/') {
            candidates.push(format!("**/{pattern}"));
        }
        if pattern.ends_with('/') {
            candidates.push(format!("{pattern}**"));
        }
        for candidate in candidates {
            if let Ok(glob) = Glob::new(&candidate) {
                builder.add(glob);
                any = true;
            }
        }
    }
    if any { builder.build().ok() } else { None }
}

fn build_regex(query: &Query) -> Result<Regex, String> {
    let text = query.text.trim();
    if text.is_empty() {
        return Err("Type to search in files".into());
    }
    let mut pattern = if query.use_regex {
        text.to_string()
    } else {
        regex::escape(text)
    };
    if query.whole_word {
        pattern = format!(r"\b(?:{pattern})\b");
    }
    RegexBuilder::new(&pattern)
        .case_insensitive(!query.match_case)
        .size_limit(4 * 1024 * 1024)
        .build()
        .map_err(|error| error.to_string())
}

fn matches(include: &Option<GlobSet>, exclude: &Option<GlobSet>, relative: &Path) -> bool {
    if let Some(exclude) = exclude
        && exclude.is_match(relative)
    {
        return false;
    }
    match include {
        Some(include) => include.is_match(relative),
        None => true,
    }
}

fn truncate(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.chars().count() <= MAX_LINE_CHARS {
        return trimmed.to_string();
    }
    let head: String = trimmed.chars().take(MAX_LINE_CHARS).collect();
    format!("{head}…")
}

/// Search `root` for `query.text`. `show_ignored` disables Git ignore rules,
/// matching the Explorer's ignored-file toggle.
pub fn run(root: &Path, query: &Query, show_ignored: bool) -> Result<Vec<Hit>, String> {
    let regex = build_regex(query)?;
    let include = build_set(&split_patterns(&query.include));
    let exclude = build_set(&split_patterns(&query.exclude));
    let root_owned = root.to_path_buf();
    let mut walker = WalkBuilder::new(root);
    walker.hidden(false);
    if show_ignored {
        walker.standard_filters(false);
    }
    if let Some(exclude) = exclude.clone() {
        let prune = Arc::new(exclude);
        let base = root_owned.clone();
        walker.filter_entry(move |entry| {
            let path = entry.path();
            let relative = path.strip_prefix(&base).unwrap_or(path);
            !prune.is_match(relative)
                && !entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| prune.is_match(Path::new(name)))
        });
    }
    let mut hits = Vec::new();
    'outer: for entry in walker.build() {
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.file_type().is_some_and(|kind| kind.is_file()) {
            continue;
        }
        let path = entry.path();
        let relative = path.strip_prefix(&root_owned).unwrap_or(path);
        if !matches(&include, &exclude, relative) {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if metadata.len() > MAX_FILE_BYTES {
            continue;
        }
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        if bytes.contains(&0) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        for (index, line) in text.lines().enumerate() {
            if regex.is_match(line) {
                hits.push(Hit {
                    path: path.to_path_buf(),
                    line: u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1),
                    text: truncate(line),
                });
                if hits.len() >= MAX_HITS {
                    break 'outer;
                }
            }
        }
    }
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(root: &Path, name: &str, body: &str) {
        let path = root.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, body).unwrap();
    }

    #[test]
    fn plain_query_is_literal_and_reports_line_numbers() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.txt", "first\nfind.me here\nlast\n");
        let query = Query {
            text: "find.me".into(),
            ..Default::default()
        };
        let hits = run(dir.path(), &query, false).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line, 2);
        assert_eq!(hits[0].text, "find.me here");
    }

    #[test]
    fn match_case_whole_word_and_regex_toggle() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.txt", "Find\nfinding\nFIND\n");
        let query = Query {
            text: "find".into(),
            match_case: true,
            ..Default::default()
        };
        assert_eq!(run(dir.path(), &query, false).unwrap().len(), 1);
        let query = Query {
            text: "find".into(),
            whole_word: true,
            ..Default::default()
        };
        assert_eq!(run(dir.path(), &query, false).unwrap().len(), 2);
        let query = Query {
            text: "f.n.".into(),
            use_regex: true,
            ..Default::default()
        };
        assert_eq!(run(dir.path(), &query, false).unwrap().len(), 3);
    }

    #[test]
    fn include_and_exclude_patterns_bound_the_walk() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "src/a.ts", "needle\n");
        write(dir.path(), "src/a.md", "needle\n");
        write(dir.path(), "dist/b.ts", "needle\n");
        let query = Query {
            text: "needle".into(),
            include: "*.ts".into(),
            exclude: "dist/**".into(),
            ..Default::default()
        };
        let hits = run(dir.path(), &query, false).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].path.ends_with("src/a.ts"));
    }

    #[test]
    fn binary_and_oversized_files_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "bin.dat", "nee\0dle");
        let query = Query {
            text: "nee".into(),
            ..Default::default()
        };
        assert!(run(dir.path(), &query, false).unwrap().is_empty());
    }
}
