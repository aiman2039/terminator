//! Moved-line detection, like `git diff --color-moved`: a block of deleted
//! lines that is added again in another place gets the moved color, not
//! the delete and insert colors.
use super::*;
use std::collections::{HashMap, HashSet};

/// Lines with fewer alphanumeric characters are too common to be a move
/// (same limit as Git's default `--color-moved` mode).
const MIN_ALNUM: usize = 20;
/// A moved block must have at least this many lines.
const MIN_BLOCK: usize = 3;

pub(super) struct MarkMovedInput<'a> {
    pub(super) old: &'a [&'a str],
    pub(super) new: &'a [&'a str],
    pub(super) options: DiffOptions,
}

/// Sets `moved` on each deleted and inserted row that is part of a moved
/// block. Lines compare under the document's compare options.
pub(super) fn mark_moved(doc: &mut DiffDocument, input: MarkMovedInput<'_>) {
    let MarkMovedInput { old, new, options } = input;
    let key = |lines: &[&str], index: usize| {
        let text = lines
            .get(index)
            .map_or("", |line| line.trim_end_matches(['\n', '\r']));
        let alnum = text.chars().filter(|c| c.is_alphanumeric()).count();
        (alnum >= MIN_ALNUM).then(|| options.normalize_line(text))
    };
    let deleted: HashSet<usize> = doc.hunks.iter().flat_map(|h| h.old.clone()).collect();
    let inserted: HashSet<usize> = doc.hunks.iter().flat_map(|h| h.new.clone()).collect();
    let mut inserts_by_key: HashMap<String, Vec<usize>> = HashMap::new();
    let mut new_keys: Vec<usize> = inserted.iter().copied().collect();
    new_keys.sort_unstable();
    for index in new_keys {
        if let Some(text) = key(new, index) {
            inserts_by_key.entry(text).or_default().push(index);
        }
    }
    let mut old_order: Vec<usize> = deleted.iter().copied().collect();
    old_order.sort_unstable();
    let mut moved_old = HashSet::new();
    let mut moved_new = HashSet::new();
    for start in old_order {
        if moved_old.contains(&start) {
            continue;
        }
        let Some(first) = key(old, start) else {
            continue;
        };
        let Some(targets) = inserts_by_key.get(&first) else {
            continue;
        };
        for &target in targets {
            if moved_new.contains(&target) {
                continue;
            }
            let mut len = 1;
            loop {
                let (o, n) = (start.saturating_add(len), target.saturating_add(len));
                let same = deleted.contains(&o)
                    && inserted.contains(&n)
                    && !moved_new.contains(&n)
                    && key(old, o).is_some_and(|k| key(new, n).as_ref() == Some(&k));
                if !same {
                    break;
                }
                len = len.saturating_add(1);
            }
            if len >= MIN_BLOCK {
                moved_old.extend(start..start.saturating_add(len));
                moved_new.extend(target..target.saturating_add(len));
                break;
            }
        }
    }
    if moved_old.is_empty() {
        return;
    }
    let apply = |line: &mut DiffLine| {
        let index = |no: Option<u32>| no.and_then(|n| usize::try_from(n).ok()?.checked_sub(1));
        line.moved = match line.kind {
            LineKind::Delete => index(line.old_no).is_some_and(|i| moved_old.contains(&i)),
            LineKind::Insert => index(line.new_no).is_some_and(|i| moved_new.contains(&i)),
            LineKind::Hunk | LineKind::Equal => false,
        };
    };
    doc.unified.iter_mut().for_each(apply);
    for row in &mut doc.split {
        row.left.iter_mut().for_each(apply);
        row.right.iter_mut().for_each(apply);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::build::{BuildWithOptionsInput, build_with_options};

    const BLOCK: &str = "fn alpha_function_name() -> usize {\n    compute_the_first_value(1)\n    compute_the_second_value(2)\n}\n";

    fn build(left: &str, right: &str, options: DiffOptions) -> DiffDocument {
        build_with_options(BuildWithOptionsInput {
            path: Path::new("f.rs"),
            left,
            right,
            staged: false,
            view: &DiffView {
                context: 3,
                options,
                ..DiffView::default()
            },
        })
    }

    fn moved_kinds(doc: &DiffDocument) -> Vec<LineKind> {
        doc.unified
            .iter()
            .filter(|line| line.moved)
            .map(|line| line.kind)
            .collect()
    }

    #[test]
    fn moved_block_is_marked_on_both_sides() {
        let left = format!("{BLOCK}one\ntwo\nthree\nfour\nfive\nsix\nseven\n");
        let right = format!("one\ntwo\nthree\nfour\nfive\nsix\nseven\n{BLOCK}");
        let doc = build(&left, &right, DiffOptions::default());
        let kinds = moved_kinds(&doc);
        assert_eq!(kinds.iter().filter(|k| **k == LineKind::Delete).count(), 3);
        assert_eq!(kinds.iter().filter(|k| **k == LineKind::Insert).count(), 3);
        assert!(
            doc.split
                .iter()
                .any(|row| row.left.as_ref().is_some_and(|l| l.moved))
        );
    }

    #[test]
    fn short_lines_are_not_marked_as_moved() {
        let doc = build("a\nb\nc\nx\n", "x\na\nb\nc\n", DiffOptions::default());
        assert!(moved_kinds(&doc).is_empty());
    }

    #[test]
    fn moved_is_off_when_the_option_is_off() {
        let left = format!("{BLOCK}one\ntwo\nthree\nfour\nfive\nsix\n");
        let right = format!("one\n{BLOCK}");
        let options = DiffOptions {
            moved: false,
            ..DiffOptions::default()
        };
        assert!(moved_kinds(&build(&left, &right, options)).is_empty());
    }

    #[test]
    fn moved_respects_ignore_whitespace() {
        let left = format!("{BLOCK}one\ntwo\nthree\nfour\nfive\nsix\n");
        let right = format!(
            "one\ntwo\nthree\nfour\nfive\nsix\n{}",
            BLOCK.replace("    ", "\t\t")
        );
        let exact = build(&left, &right, DiffOptions::default());
        assert!(moved_kinds(&exact).len() < 6);
        let trimmed = build(
            &left,
            &right,
            DiffOptions {
                whitespace: Whitespace::Trim,
                ..DiffOptions::default()
            },
        );
        assert_eq!(moved_kinds(&trimmed).len(), 6);
    }
}
