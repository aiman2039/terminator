use super::*;
use std::process::Command;

fn git_cmd(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git_cmd(dir.path(), &["init", "-q"]);
    git_cmd(dir.path(), &["config", "core.autocrlf", "false"]);
    git_cmd(
        dir.path(),
        &["config", "user.email", "fixture@example.invalid"],
    );
    git_cmd(dir.path(), &["config", "user.name", "Fixture"]);
    dir
}

/// 100 numbered lines with lines 10 and 80 changed.
fn long_pair() -> (String, String) {
    let original: String = (0..100).fold(String::new(), |mut text, n| {
        use std::fmt::Write as _;
        let _ = writeln!(text, "{n}");
        text
    });
    let changed = original
        .replacen("10\n", "changed\n", 1)
        .replacen("80\n", "another\n", 1);
    (original, changed)
}

#[test]
fn gap_steps_reveal_twenty_lines_from_either_edge() {
    let (original, changed) = long_pair();
    let doc = build_with_context(BuildWithContextInput {
        path: Path::new("f.txt"),
        left: &original,
        right: &changed,
        staged: false,
        context: 3,
    });
    // Leading, middle, trailing gaps.
    assert_eq!(doc.gaps.len(), 3);
    let middle = doc.gaps[1].old.clone();
    let top = doc
        .expand_gap(ExpandGapInput {
            path: Path::new("f.txt"),
            gap: 1,
            part: GapPart::Top,
            staged: false,
        })
        .unwrap();
    assert_eq!(top.gaps.len(), 3);
    assert_eq!(top.gaps[1].old, middle.start + GAP_STEP..middle.end);
    let both = top
        .expand_gap(ExpandGapInput {
            path: Path::new("f.txt"),
            gap: 1,
            part: GapPart::Bottom,
            staged: false,
        })
        .unwrap();
    assert_eq!(
        both.gaps[1].old,
        middle.start + GAP_STEP..middle.end - GAP_STEP
    );
    assert_eq!(both.revealed.len(), 2);
}

#[test]
fn rebuild_keeps_revealed_ranges_and_whitespace_mode() {
    let (original, changed) = long_pair();
    let doc = build_with_context(BuildWithContextInput {
        path: Path::new("f.txt"),
        left: &original,
        right: &changed,
        staged: false,
        context: 3,
    });
    let expanded = doc
        .expand_gap(ExpandGapInput {
            path: Path::new("f.txt"),
            gap: 1,
            part: GapPart::All,
            staged: false,
        })
        .unwrap();
    let view = DiffView {
        context: 3,
        revealed: expanded.revealed.clone(),
        whitespace: false,
    };
    let refreshed = doc.rebuild(Path::new("f.txt"), false, &view);
    assert_eq!(refreshed.gaps.len(), 2);
    assert_eq!(refreshed.unified.len(), expanded.unified.len());
}

#[test]
fn ignore_whitespace_hides_indent_only_changes_and_keeps_text() {
    let left = "fn a() {\nx();\n}\n";
    let right = "fn a() {\n    x();\n}\nnew();\n";
    let exact = build_with_context(BuildWithContextInput {
        path: Path::new("a.rs"),
        left,
        right,
        staged: false,
        context: 3,
    });
    assert_eq!(exact.hunks.len(), 2);
    let loose = exact.rebuild(
        Path::new("a.rs"),
        false,
        &DiffView {
            context: 3,
            revealed: Vec::new(),
            whitespace: true,
        },
    );
    // Only the added line is a change; the re-indent is not.
    assert_eq!(loose.hunks.len(), 1);
    assert_eq!(loose.hunks[0].new, 3..4);
    assert_eq!(loose.right_text, right);
    // Display keeps the real indentation.
    let shown: String = loose.unified[1]
        .spans
        .iter()
        .map(|s| s.text.as_str())
        .collect();
    assert_eq!(shown, "    x();");
}

#[test]
fn aligned_left_has_one_row_per_buffer_line() {
    let left = "a\nb\nc\nd\n";
    let right = "a\nB\nnew\nd\n";
    let aligned = align_left(left, right, &[]);
    assert_eq!(aligned.doc.split.len(), 4);
    let left_no = |row: usize| aligned.doc.split[row].left.as_ref().and_then(|l| l.old_no);
    assert_eq!(left_no(0), Some(1));
    // `b`/`c` replaced by `B`/`new`: paired rows show the originals.
    assert_eq!(left_no(1), Some(2));
    assert_eq!(left_no(2), Some(3));
    assert_eq!(left_no(3), Some(4));
    assert!(aligned.deleted.is_empty());
    let removed = align_left("a\nb\nc\n", "a\nc\n", &[]);
    assert_eq!(removed.doc.split.len(), 2);
    assert_eq!(removed.deleted, [(1, vec!["b".to_owned()])]);
}

#[test]
fn binary_sides_report_their_sizes() {
    let error = decode_sides(b"text", b"\0\x01\x02")
        .unwrap_err()
        .to_string();
    assert!(error.starts_with("Binary file · 4 B → 3 B"), "{error}");
    assert_eq!(human_size(2048), "2.0 KB");
}

#[test]
fn staged_and_working_sides_and_word_highlights() {
    let dir = repo();
    let root = dir.path();
    let name = if cfg!(windows) {
        Path::new("space & ' 日本.rs")
    } else {
        Path::new("space | ' 日本.rs")
    };
    std::fs::write(root.join(name), "fn main() { base(); }\n").unwrap();
    git_cmd(root, &["add", "."]);
    git_cmd(root, &["commit", "-qm", "base"]);
    std::fs::write(root.join(name), "fn main() { staged(); }\n").unwrap();
    git_cmd(root, &["add", "."]);
    std::fs::write(root.join(name), "fn main() { working(); }\n").unwrap();
    let staged = document(DiffRequest {
        cwd: root,
        path: &root.join(name),
        staged: true,
    })
    .unwrap();
    assert_eq!(staged.left_label, "HEAD");
    assert_eq!(staged.right_label, "Index");
    assert_eq!(
        staged.left_text, "fn main() { base(); }\n",
        "left_text should contain the old blob content"
    );
    assert_eq!(
        staged.right_text, "fn main() { staged(); }\n",
        "right_text should contain the staged content"
    );
    let joined: String = staged
        .unified
        .iter()
        .flat_map(|line| line.spans.iter().map(|s| s.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("staged"));
    assert!(!joined.contains("working"));
    assert!(
        staged
            .unified
            .iter()
            .any(|line| line.kind == LineKind::Insert
                && line.spans.iter().any(|s| s.intra == Intra::Change))
    );
    let working = document(DiffRequest {
        cwd: root,
        path: &root.join(name),
        staged: false,
    })
    .unwrap();
    assert_eq!(
        working.left_text, "fn main() { staged(); }\n",
        "working left_text should be the staged content"
    );
    assert_eq!(
        working.right_text, "fn main() { working(); }\n",
        "working right_text should be the working tree content"
    );
    let joined: String = working
        .unified
        .iter()
        .flat_map(|line| line.spans.iter().map(|s| s.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("working"));
    assert!(!joined.contains("base"));
    assert!(!working.split.is_empty());
}
