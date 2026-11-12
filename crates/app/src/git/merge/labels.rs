//! Branch names for the merge tab's Ours and Theirs headers. They come
//! from the operation in progress: during a rebase "ours" is the commit
//! being rebased onto and "theirs" is the rebased branch, the opposite of a
//! merge. Read-only Git reads; they run with the merge load on the file pool.
use std::path::{Path, PathBuf};
use terminator_core::{CommandOptions, git, run_command};

/// What the Ours and Theirs columns show; `None` when unknown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SideLabels {
    pub ours: Option<String>,
    pub theirs: Option<String>,
}

fn text(root: &Path, args: &[&str]) -> Option<String> {
    let options = CommandOptions {
        timeout: std::time::Duration::from_secs(5),
        ..Default::default()
    };
    let output = run_command(git::command(root, args), options).ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// A branch that points at `rev`, else its short hash.
fn name_of(root: &Path, rev: &str) -> Option<String> {
    let hash = text(root, &["rev-parse", "--verify", "--quiet", rev])?;
    let points = format!("--points-at={hash}");
    let format = "--format=%(refname:short)";
    text(
        root,
        &[
            "for-each-ref",
            &points,
            format,
            "refs/heads",
            "refs/remotes",
        ],
    )
    .and_then(|names| names.lines().next().map(str::to_owned))
    .or_else(|| text(root, &["rev-parse", "--short", &hash]))
}

fn read(path: PathBuf) -> Option<String> {
    let value = std::fs::read_to_string(path).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// The labels of the conflict in progress in the worktree at `root`.
pub(crate) fn side_labels(root: &Path) -> SideLabels {
    let Some(git_dir) = text(root, &["rev-parse", "--absolute-git-dir"]).map(PathBuf::from) else {
        return SideLabels::default();
    };
    let current = || text(root, &["symbolic-ref", "--short", "-q", "HEAD"]);
    for folder in ["rebase-merge", "rebase-apply"] {
        let state = git_dir.join(folder);
        if state.is_dir() {
            let onto = read(state.join("onto")).and_then(|onto| name_of(root, &onto));
            let rebased = read(state.join("head-name"))
                .map(|name| name.trim_start_matches("refs/heads/").to_owned())
                .or_else(|| name_of(root, "REBASE_HEAD"));
            return SideLabels {
                ours: onto,
                theirs: rebased,
            };
        }
    }
    for (head, prefix) in [
        ("MERGE_HEAD", ""),
        ("CHERRY_PICK_HEAD", "cherry-pick "),
        ("REVERT_HEAD", "revert "),
    ] {
        if git_dir.join(head).is_file() {
            return SideLabels {
                ours: current(),
                theirs: name_of(root, head).map(|name| format!("{prefix}{name}")),
            };
        }
    }
    SideLabels {
        ours: current(),
        theirs: None,
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;

    fn git(cwd: &Path, args: &[&str]) {
        let options = CommandOptions {
            timeout: std::time::Duration::from_secs(20),
            accepted_exit_codes: None,
            ..Default::default()
        };
        run_command(git::command(cwd, args), options).unwrap();
    }

    /// `main` and `feature` both change `a.txt`.
    fn conflicting(dir: &Path) -> PathBuf {
        let root = dir.join("repo");
        std::fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        git(&root, &["config", "user.email", "t@example.com"]);
        git(&root, &["config", "user.name", "T"]);
        git(&root, &["config", "core.hooksPath", "/dev/null"]);
        std::fs::write(root.join("a.txt"), "base\n").unwrap();
        git(&root, &["add", "a.txt"]);
        git(&root, &["commit", "-qm", "base"]);
        git(&root, &["switch", "-qc", "feature"]);
        std::fs::write(root.join("a.txt"), "feature\n").unwrap();
        git(&root, &["commit", "-qam", "feature"]);
        git(&root, &["switch", "-q", "main"]);
        std::fs::write(root.join("a.txt"), "main\n").unwrap();
        git(&root, &["commit", "-qam", "main"]);
        root
    }

    #[test]
    fn a_merge_names_the_current_and_the_merged_branch() {
        let dir = tempfile::tempdir().unwrap();
        let root = conflicting(dir.path());
        git(&root, &["merge", "--no-edit", "feature"]);
        let labels = side_labels(&root);
        assert_eq!(labels.ours.as_deref(), Some("main"));
        assert_eq!(labels.theirs.as_deref(), Some("feature"));
    }

    #[test]
    fn a_rebase_names_the_onto_branch_as_ours() {
        let dir = tempfile::tempdir().unwrap();
        let root = conflicting(dir.path());
        git(&root, &["switch", "-q", "feature"]);
        git(&root, &["rebase", "main"]);
        let labels = side_labels(&root);
        assert_eq!(labels.ours.as_deref(), Some("main"));
        assert_eq!(labels.theirs.as_deref(), Some("feature"));
    }
}
