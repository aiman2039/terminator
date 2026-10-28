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
