use super::staging::{commit_rename, stage_rename};
use super::*;

struct EditInput<'a> {
    pub(crate) path: &'a Path,
    pub(crate) line: u32,
    pub(crate) start: u32,
    pub(crate) end: u32,
    pub(crate) replacement: &'a str,
}

fn edit(input: EditInput<'_>) -> native_lsp::RenameFileEdit {
    let EditInput {
        path,
        line,
        start,
        end,
        replacement,
    } = input;
    native_lsp::RenameFileEdit {
        uri: native_lsp::file_uri(path).unwrap().to_string(),
        version: None,
        edits: vec![native_lsp::RenameTextEdit {
            range: native_lsp::Range {
                start: native_lsp::Position {
                    line,
                    character: start,
                },
                end: native_lsp::Position {
                    line,
                    character: end,
                },
            },
            new_text: replacement.into(),
        }],
    }
}

#[test]
fn reviewed_multifile_rename_uses_utf16_and_changes_both_files() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("a.rs");
    let b = root.path().join("b.rs");
    std::fs::write(&a, "let 🦀 = foo;\n").unwrap();
    std::fs::write(&b, "foo();\n").unwrap();
    let preview = prepare_rename(
        root.path(),
        vec![
            edit(EditInput {
                path: &a,
                line: 0,
                start: 9,
                end: 12,
                replacement: "bar",
            }),
            edit(EditInput {
                path: &b,
                line: 0,
                start: 0,
                end: 3,
                replacement: "bar",
            }),
        ],
    )
    .unwrap();
    assert_eq!(preview.files.len(), 2);
    assert!(preview.files[0].diff.contains("bar"));
    assert_eq!(apply_rename(preview).unwrap().len(), 2);
    assert_eq!(std::fs::read_to_string(a).unwrap(), "let 🦀 = bar;\n");
    assert_eq!(std::fs::read_to_string(b).unwrap(), "bar();\n");
}

#[test]
fn stale_disk_file_stops_rename_before_any_file_changes() {
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("a.rs");
    let b = root.path().join("b.rs");
    std::fs::write(&a, "foo\n").unwrap();
    std::fs::write(&b, "foo\n").unwrap();
    let preview = prepare_rename(
        root.path(),
        vec![
            edit(EditInput {
                path: &a,
                line: 0,
                start: 0,
                end: 3,
                replacement: "bar",
            }),
            edit(EditInput {
                path: &b,
                line: 0,
                start: 0,
                end: 3,
                replacement: "bar",
            }),
        ],
    )
    .unwrap();
    std::fs::write(&b, "other\n").unwrap();
    assert!(apply_rename(preview).is_err());
    assert_eq!(std::fs::read_to_string(a).unwrap(), "foo\n");
    assert_eq!(std::fs::read_to_string(b).unwrap(), "other\n");
}

/// Prepares a rename of `a.rs` ("alpha") and `b.rs` ("beta") in `root`.
fn two_file_rename(root: &Path) -> (PathBuf, PathBuf, PreparedRename) {
    let a = root.join("a.rs");
    let b = root.join("b.rs");
    std::fs::write(&a, "alpha\n").unwrap();
    std::fs::write(&b, "beta\n").unwrap();
    let prepared = prepare_rename(
        root,
        vec![
            edit(EditInput {
                path: &a,
                line: 0,
                start: 0,
                end: 5,
                replacement: "gamma",
            }),
            edit(EditInput {
                path: &b,
                line: 0,
                start: 0,
                end: 4,
                replacement: "delta",
            }),
        ],
    )
    .unwrap();
    (a, b, prepared)
}

/// Paths of files in `root` other than `a.rs` and `b.rs` that hold `bytes`.
fn backups_with(root: &Path, bytes: &[u8]) -> Vec<PathBuf> {
    std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| !path.ends_with("a.rs") && !path.ends_with("b.rs"))
        .filter(|path| std::fs::read(path).is_ok_and(|read| read == bytes))
        .collect()
}

#[test]
fn failed_later_write_restores_earlier_file_and_leaves_no_temp_files() {
    let root = tempfile::tempdir().unwrap();
    let (a, b, prepared) = two_file_rename(root.path());
    let stages = stage_rename(&prepared).unwrap();
    let result = commit_rename(&prepared, stages, |path| {
        if path == b {
            std::fs::write(&b, "other\n").unwrap();
        }
    });
    assert!(result.is_err());
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "alpha\n");
    assert_eq!(std::fs::read_to_string(&b).unwrap(), "other\n");
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 2);
}

#[test]
fn rollback_keeps_backup_and_concurrent_edit_when_file_changed_after_apply() {
    let root = tempfile::tempdir().unwrap();
    let (a, b, prepared) = two_file_rename(root.path());
    let stages = stage_rename(&prepared).unwrap();
    let error = commit_rename(&prepared, stages, |path| {
        if path == b {
            std::fs::write(&a, "concurrent\n").unwrap();
            std::fs::write(&b, "other\n").unwrap();
        }
    })
    .unwrap_err()
    .to_string();
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "concurrent\n");
    let backups = backups_with(root.path(), b"alpha\n");
    assert_eq!(backups.len(), 1, "{error}");
    assert!(error.contains(&backups[0].display().to_string()), "{error}");
}

#[test]
#[cfg(unix)]
fn rollback_keeps_backup_when_restoring_original_fails() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = tempfile::tempdir().unwrap();
    let (a, b, prepared) = two_file_rename(root.path());
    let stages = stage_rename(&prepared).unwrap();
    let mut writable = true;
    let error = commit_rename(&prepared, stages, |path| {
        if path == b {
            let read_only = std::fs::Permissions::from_mode(0o555);
            std::fs::set_permissions(root.path(), read_only).unwrap();
            // A privileged user can still write; then nothing fails.
            writable = std::fs::write(root.path().join("probe"), b"").is_ok();
        }
    });
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    if writable {
        return;
    }
    let error = error.unwrap_err().to_string();
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "gamma\n");
    let backups = backups_with(root.path(), b"alpha\n");
    assert_eq!(backups.len(), 1, "{error}");
    assert!(error.contains(&backups[0].display().to_string()), "{error}");
}

#[test]
fn overlapping_or_outside_edits_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let a = root.path().join("a.rs");
    let b = outside.path().join("b.rs");
    std::fs::write(&a, "foobar\n").unwrap();
    std::fs::write(&b, "foo\n").unwrap();
    let mut same = edit(EditInput {
        path: &a,
        line: 0,
        start: 0,
        end: 3,
        replacement: "bar",
    });
    same.edits.extend(
        edit(EditInput {
            path: &a,
            line: 0,
            start: 2,
            end: 5,
            replacement: "baz",
        })
        .edits,
    );
    assert!(prepare_rename(root.path(), vec![same]).is_err());
    assert!(
        prepare_rename(
            root.path(),
            vec![edit(EditInput {
                path: &b,
                line: 0,
                start: 0,
                end: 3,
                replacement: "bar"
            })]
        )
        .is_err()
    );
}

#[test]
#[cfg(unix)]
fn symlinked_rename_target_is_rejected() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("target.rs");
    let link = root.path().join("link.rs");
    std::fs::write(&target, "foo\n").unwrap();
    symlink(&target, &link).unwrap();
    assert!(
        prepare_rename(
            root.path(),
            vec![edit(EditInput {
                path: &link,
                line: 0,
                start: 0,
                end: 3,
                replacement: "bar"
            })]
        )
        .is_err()
    );
    assert_eq!(std::fs::read_to_string(target).unwrap(), "foo\n");
}
