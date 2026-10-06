//! Directory listings with Git ignore rules applied.
use anyhow::{Result, ensure};
use std::path::{Path, PathBuf};
use terminator_core::{CommandOptions, git};

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub directory: bool,
    pub ignored: bool,
}

/// `check-ignore` in batch mode: NUL-delimited paths on stdin, ignored paths
/// back on stdout. Exit status 1 means no matches, not failure.
pub const CHECK_IGNORE_ARGV: &[&str] = &["check-ignore", "--stdin", "-z"];

fn status_options(input: Option<Vec<u8>>, allow_no_matches: bool) -> CommandOptions {
    CommandOptions {
        input,
        stdout_limit: 4 * 1024 * 1024,
        accepted_exit_codes: Some(if allow_no_matches {
            vec![0, 1]
        } else {
            vec![0]
        }),
        ..Default::default()
    }
}

/// NUL-delimited stdin batch for [`CHECK_IGNORE_ARGV`].
pub fn stdin_batch(entries: &[Entry]) -> Vec<u8> {
    let mut input = Vec::new();
    for entry in entries {
        input.extend_from_slice(&terminator_core::os_bytes(entry.path.as_os_str()));
        input.push(0);
    }
    input
}

/// Whether `path` appears in [`CHECK_IGNORE_ARGV`] output.
pub fn is_ignored(output: &[u8], path: &Path) -> bool {
    output
        .split(|b| *b == 0)
        .any(|p| p == terminator_core::os_bytes(path.as_os_str()).as_ref())
}

pub fn entries(path: &Path) -> Result<Vec<Entry>> {
    entries_known(
        path,
        git::run_blocking(
            path,
            &["rev-parse", "--show-toplevel"],
            status_options(None, false),
        )
        .is_ok(),
    )
}

pub fn entries_known(path: &Path, in_git: bool) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        ensure!(
            entries.len() < 1000,
            "Directory exceeds the 1,000-entry display limit"
        );
        entries.push(Entry {
            ignored: entry.file_name() == ".git",
            path: entry.path(),
            directory: entry.file_type()?.is_dir(),
        });
    }
    // Ask Git so nested ignore files, negations, global excludes and tracked
    // files use the same rules as the user's repository. Keep dotfiles visible.
    if in_git {
        for batch in entries.chunks_mut(1000) {
            let output = git::run_blocking(
                path,
                CHECK_IGNORE_ARGV,
                status_options(Some(stdin_batch(batch)), true),
            )?;
            for entry in batch.iter_mut() {
                entry.ignored |= is_ignored(&output, &entry.path);
            }
        }
    }
    entries.sort_by(|a, b| b.directory.cmp(&a.directory).then(a.path.cmp(&b.path)));
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn git(root: &Path, args: &[&str]) {
        git::run_blocking(root, args, status_options(None, false)).unwrap();
    }

    #[test]
    fn git_ignore_rules_preserve_dotfiles_negations_and_tracked_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        git(root, &["init", "-q"]);
        fs::write(root.join("tracked.log"), "tracked").unwrap();
        git(root, &["add", "tracked.log"]);
        fs::write(root.join(".gitignore"), "*.log\n!keep.log\nbuild/\n").unwrap();
        for name in ["ignored.log", "keep.log", ".visible"] {
            fs::write(root.join(name), "").unwrap();
        }
        fs::create_dir(root.join("build")).unwrap();
        let entries = entries(root).unwrap();
        let ignored = |name: &str| {
            entries
                .iter()
                .find(|e| e.path.file_name().unwrap() == name)
                .unwrap()
                .ignored
        };
        assert!(ignored("ignored.log"));
        assert!(ignored("build"));
        assert!(ignored(".git"));
        assert!(!ignored(".visible"));
        assert!(!ignored(".gitignore"));
        assert!(!ignored("keep.log"));
        assert!(!ignored("tracked.log"));
    }
}
