use super::*;

fn change(path: &str) -> terminator_git::Change {
    terminator_git::Change {
        path: PathBuf::from(path),
        status: " M".into(),
    }
}

#[test]
fn check_state_covers_all_none_and_some() {
    assert_eq!(
        check_state(CheckStateInput {
            checked: 0,
            total: 0
        }),
        CheckState::Unchecked
    );
    assert_eq!(
        check_state(CheckStateInput {
            checked: 0,
            total: 3
        }),
        CheckState::Unchecked
    );
    assert_eq!(
        check_state(CheckStateInput {
            checked: 3,
            total: 3
        }),
        CheckState::Checked
    );
    assert_eq!(
        check_state(CheckStateInput {
            checked: 1,
            total: 3
        }),
        CheckState::Partial
    );
}

#[test]
fn change_tree_compresses_single_child_chains() {
    let root = Path::new("/repo");
    let deep = change("/repo/apps/backend/.sqlx/query.json");
    let top = change("/repo/apps/top.rs");
    let tree = build_change_tree(&[&deep, &top], Some(root));
    // `apps` holds a file, so it stays; the chain under it compresses.
    let apps = tree.dirs.get("apps").expect("apps node");
    assert!(apps.dirs.contains_key("backend/.sqlx"));
    assert_eq!(apps.count_files(), 2);
}

#[test]
fn change_tree_keeps_branching_directories_separate() {
    let root = Path::new("/repo");
    let a = change("/repo/src/a.rs");
    let b = change("/repo/tests/b.rs");
    let tree = build_change_tree(&[&a, &b], Some(root));
    assert!(tree.dirs.contains_key("src"));
    assert!(tree.dirs.contains_key("tests"));
}
