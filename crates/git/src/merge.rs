//! Read-only conflict stages for the three-way merge tool: the common
//! ancestor (`:1:`), ours (`:2:`), and theirs (`:3:`) blobs of one
//! unmerged path. Resolving and staging stay explicit GUI actions.
use crate::{Error, Result, snapshot};
use std::path::{Component, Path};
use terminator_core::{CommandOptions, git};

/// The index stages of one conflicted path. A missing stage is a side that
/// does not have the file (add/add has no base, modify/delete lacks a side).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConflictStages {
    pub base: Option<Vec<u8>>,
    pub ours: Option<Vec<u8>>,
    pub theirs: Option<Vec<u8>>,
}

/// `(stage, object id)` pairs from `git ls-files -u -z` output.
#[must_use]
pub fn parse_unmerged(raw: &[u8]) -> Vec<(u8, String)> {
    raw.split(|byte| *byte == 0)
        .filter_map(|record| {
            let text = std::str::from_utf8(record).ok()?;
            let (meta, _path) = text.split_once('\t')?;
            let mut fields = meta.split(' ');
            let _mode = fields.next()?;
            let oid = fields.next()?;
            let stage = fields.next()?.parse::<u8>().ok()?;
            (1..=3).contains(&stage).then(|| (stage, oid.to_owned()))
        })
        .collect()
}

/// Read the conflict stages of `path` (repository-relative).
pub fn conflict_stages(
    root: &Path,
    path: &Path,
    options: &CommandOptions,
) -> Result<ConflictStages> {
    if path.is_absolute() || !path.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(Error::PathEscapes { path: path.into() });
    }
    let mut spec = std::ffi::OsString::from(":(literal)");
    spec.push(path);
    let raw = git::run_blocking(
        root,
        &[
            std::ffi::OsStr::new("ls-files"),
            "-u".as_ref(),
            "-z".as_ref(),
            "--".as_ref(),
            spec.as_os_str(),
        ],
        CommandOptions {
            timeout: options.timeout,
            stdout_limit: options.stdout_limit,
            stderr_limit: options.stderr_limit,
            ..CommandOptions::default()
        },
    )?;
    let entries = parse_unmerged(&raw);
    if entries.is_empty() {
        return Err(Error::NotConflicted { path: path.into() });
    }
    let mut stages = ConflictStages::default();
    for (stage, oid) in entries {
        let bytes = Some(snapshot::blob(root, &oid, options)?);
        match stage {
            1 => stages.base = bytes,
            2 => stages.ours = bytes,
            _ => stages.theirs = bytes,
        }
    }
    Ok(stages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unmerged_records_parse_stages_and_skip_noise() {
        let raw = b"100644 aaa 1\tf.txt\x00100644 bbb 2\tf.txt\x00100644 ccc 3\tf.txt\0garbage\0";
        assert_eq!(
            parse_unmerged(raw),
            vec![
                (1, "aaa".to_owned()),
                (2, "bbb".to_owned()),
                (3, "ccc".to_owned())
            ]
        );
        assert!(parse_unmerged(b"100644 aaa 0\tf.txt\0").is_empty());
    }

    #[test]
    fn stages_of_a_real_conflict_are_read_and_clean_files_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .output()
                .unwrap();
            output.status.success()
        };
        assert!(git(&["init", "-q", "-b", "main"]));
        assert!(git(&["config", "user.email", "fixture@example.invalid"]));
        assert!(git(&["config", "user.name", "Fixture"]));
        std::fs::write(root.join("f.txt"), "base\n").unwrap();
        assert!(git(&["add", "f.txt"]));
        assert!(git(&["commit", "-qm", "base"]));
        assert!(git(&["checkout", "-qb", "other"]));
        std::fs::write(root.join("f.txt"), "theirs\n").unwrap();
        assert!(git(&["commit", "-qam", "theirs"]));
        assert!(git(&["checkout", "-q", "main"]));
        std::fs::write(root.join("f.txt"), "ours\n").unwrap();
        assert!(git(&["commit", "-qam", "ours"]));
        assert!(!git(&["merge", "-q", "other"]));
        let options = CommandOptions::default();
        let stages = conflict_stages(root, Path::new("f.txt"), &options).unwrap();
        assert_eq!(stages.base.as_deref(), Some(&b"base\n"[..]));
        assert_eq!(stages.ours.as_deref(), Some(&b"ours\n"[..]));
        assert_eq!(stages.theirs.as_deref(), Some(&b"theirs\n"[..]));
        assert!(matches!(
            conflict_stages(root, Path::new("missing.txt"), &options),
            Err(Error::NotConflicted { .. })
        ));
        assert!(conflict_stages(root, Path::new("../x"), &options).is_err());
    }
}
