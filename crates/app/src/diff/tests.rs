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
        options: DiffOptions::default(),
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
            options: DiffOptions {
                whitespace: Whitespace::All,
                ..DiffOptions::default()
            },
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

fn build_view(left: &str, right: &str, options: DiffOptions) -> DiffDocument {
    build_with_options(BuildWithOptionsInput {
        path: Path::new("f.txt"),
        left,
        right,
        staged: false,
        view: &DiffView {
            context: 3,
            revealed: Vec::new(),
            options,
        },
    })
}

/// Changed fragments of one painted line.
fn changed_runs(line: &DiffLine) -> Vec<String> {
    line.spans
        .iter()
        .filter(|span| span.intra == Intra::Change)
        .map(|span| span.text.clone())
        .collect()
}

#[test]
fn ignore_case_hides_case_only_changes_and_keeps_text() {
    let left = "Hello World\nkeep\n";
    let right = "hello world\nkeep\nadded\n";
    assert_eq!(
        build_view(left, right, DiffOptions::default()).hunks.len(),
        2
    );
    let loose = build_view(
        left,
        right,
        DiffOptions {
            ignore_case: true,
            ..DiffOptions::default()
        },
    );
    assert_eq!(loose.hunks.len(), 1);
    assert_eq!(loose.hunks[0].old, 2..2);
    assert_eq!(loose.hunks[0].new, 2..3);
    let shown: String = loose.unified[0]
        .spans
        .iter()
        .map(|s| s.text.as_str())
        .collect();
    assert_eq!(shown, "hello world");
}

#[test]
fn trim_ignores_edge_whitespace_but_not_inner_whitespace() {
    let options = DiffOptions {
        whitespace: Whitespace::Trim,
        ..DiffOptions::default()
    };
    let trimmed = build_view("  a b\nx\n", "a b  \nx y\n", options);
    assert_eq!(trimmed.hunks.len(), 1);
    assert_eq!(trimmed.hunks[0].old, 1..2);
    let inner = build_view("a b\n", "ab\n", options);
    assert_eq!(inner.hunks.len(), 1, "trim keeps inner whitespace changes");
    let all = build_view(
        "a b\n",
        "ab\n",
        DiffOptions {
            whitespace: Whitespace::All,
            ..DiffOptions::default()
        },
    );
    assert!(all.hunks.is_empty());
}

#[test]
fn highlight_granularity_marks_words_chars_or_whole_lines() {
    let with = |highlight| {
        build_view(
            "let total = count;\n",
            "let totals = count;\n",
            DiffOptions {
                highlight,
                ..DiffOptions::default()
            },
        )
    };
    assert_eq!(changed_runs(&with(Highlight::Words).unified[1]), ["totals"]);
    assert_eq!(changed_runs(&with(Highlight::Words).unified[0]), ["total"]);
    assert_eq!(changed_runs(&with(Highlight::Chars).unified[1]), ["s"]);
    for highlight in [Highlight::Lines, Highlight::None] {
        let doc = with(highlight);
        assert!(
            doc.unified.iter().all(|line| changed_runs(line).is_empty()),
            "{highlight:?} marks no words"
        );
        assert_eq!(doc.options.highlight, highlight);
    }
}

#[test]
fn ignored_whitespace_is_never_marked_as_a_word_change() {
    let options = DiffOptions {
        whitespace: Whitespace::All,
        ..DiffOptions::default()
    };
    let fragments = options.intra("a  =  b", "a = c").unwrap();
    let changed: Vec<&str> = fragments
        .new
        .iter()
        .filter(|(changed, _)| *changed)
        .map(|(_, text)| text.as_str())
        .collect();
    assert_eq!(changed, ["c"]);
    let joined: String = fragments
        .old
        .iter()
        .map(|(_, text)| text.as_str())
        .collect();
    assert_eq!(joined, "a  =  b");
}
