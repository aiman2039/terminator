use super::*;

#[test]
fn remote_missing_and_nested_roots_are_not_watched() {
    let dir = tempfile::tempdir().unwrap();
    let outer = dir.path().to_path_buf();
    let inner = outer.join("inner");
    std::fs::create_dir(&inner).unwrap();
    let roots = project_roots(
        [
            inner,
            outer.clone(),
            PathBuf::from("/__terminator_remote__/abc"),
            outer.join("missing"),
        ]
        .into_iter(),
    );
    assert_eq!(roots, BTreeSet::from([outer]));
}

#[test]
fn git_internals_and_swap_files_are_not_recorded() {
    assert!(!eligible_path(Path::new("/p/.git/index")));
    assert!(!eligible_path(Path::new("/p/.a.rs.swp")));
    assert!(!eligible_path(Path::new("/p/a.rs~")));
    assert!(eligible_path(Path::new("/p/src/a.rs")));
}

#[test]
fn a_changed_file_is_recorded_once_and_large_files_are_skipped() {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let store = local_history::Store::in_data(data.path());
    let roots = BTreeSet::from([project.path().to_path_buf()]);
    let text = project.path().join("a.txt");
    let large = project.path().join("big.txt");
    std::fs::write(&text, "hello").unwrap();
    std::fs::write(&large, vec![b'a'; local_history::MAX_BYTES + 1]).unwrap();
    record_batch(&store, &roots, vec![text.clone(), large.clone()]);
    record_batch(&store, &roots, vec![text.clone()]);
    assert_eq!(store.list(&text).unwrap().len(), 1);
    assert!(!store.has_history(&large));
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_to_a_file_outside_the_project_is_not_recorded() {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("secret.txt");
    std::fs::write(&secret, "TOKEN=1").unwrap();
    let link = project.path().join("link.txt");
    std::os::unix::fs::symlink(&secret, &link).unwrap();
    let source = project.path().join("main.rs");
    std::fs::write(&source, "fn main() {}").unwrap();
    let store = local_history::Store::in_data(data.path());
    let roots = BTreeSet::from([project.path().to_path_buf()]);
    record_batch(&store, &roots, vec![link.clone(), source.clone()]);
    assert!(!store.has_history(&link));
    assert!(store.has_history(&source));
    assert_eq!(read_regular(&link), None);
}

#[test]
fn build_outputs_outside_git_are_not_recorded() {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let store = local_history::Store::in_data(data.path());
    let roots = BTreeSet::from([project.path().to_path_buf()]);
    std::fs::create_dir_all(project.path().join("target/debug")).unwrap();
    let output = project.path().join("target/debug/out.txt");
    let source = project.path().join("main.rs");
    std::fs::write(&output, "built").unwrap();
    std::fs::write(&source, "fn main() {}").unwrap();
    record_batch(&store, &roots, vec![output.clone(), source.clone()]);
    assert!(!store.has_history(&output));
    assert!(store.has_history(&source));
}

#[test]
fn a_submodule_path_in_the_batch_keeps_gitignored_files_out() {
    let data = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .is_ok_and(|out| out.status.success())
    };
    if !git(&["init", "-q"]) {
        return; // No git on this machine.
    }
    std::fs::write(root.join(".gitignore"), ".env\n").unwrap();
    std::fs::create_dir(root.join("sub")).unwrap();
    let oid = "0123456789abcdef0123456789abcdef01234567";
    assert!(git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("160000,{oid},sub")
    ]));
    let in_submodule = root.join("sub/a.txt");
    let secret = root.join(".env");
    let source = root.join("main.rs");
    std::fs::write(&in_submodule, "sub").unwrap();
    std::fs::write(&secret, "TOKEN=1").unwrap();
    std::fs::write(&source, "fn main() {}").unwrap();
    let store = local_history::Store::in_data(data.path());
    let roots = BTreeSet::from([root.to_path_buf()]);
    record_batch(
        &store,
        &roots,
        vec![in_submodule.clone(), secret.clone(), source.clone()],
    );
    assert!(!store.has_history(&secret), "gitignored file stays out");
    assert!(store.has_history(&source));
    assert!(store.has_history(&in_submodule));
}

#[test]
fn a_failed_batch_is_rechecked_in_halves_not_path_by_path() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .is_ok_and(|out| out.status.success())
    };
    if !git(&["init", "-q"]) {
        return; // No git on this machine.
    }
    std::fs::write(root.join(".gitignore"), "*.log\n").unwrap();
    let oid = "0123456789abcdef0123456789abcdef01234567";
    assert!(git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("160000,{oid},sub")
    ]));
    let mut paths: Vec<PathBuf> = (0..500).map(|i| root.join(format!("f{i}.rs"))).collect();
    paths.push(root.join("x.log"));
    paths.insert(250, root.join("sub/a.txt"));
    assert!(ignored(root, &paths).is_none(), "the whole batch fails");
    let mut split = SplitCheck::new();
    split.check(root, &paths);
    assert!(split.runs <= 2 * 10, "{} runs", split.runs);
    assert_eq!(split.unknown, HashSet::from([root.join("sub/a.txt")]));
    assert_eq!(split.ignored, HashSet::from([root.join("x.log")]));
}

#[test]
fn batch_output_is_split_into_blobs_by_object_id() {
    let out = b"aaa blob 5\nhello\nbbb blob 0\n\nccc tree 3\nxyz\n";
    let blobs = parse_batch(out);
    assert_eq!(blobs.get("aaa").unwrap(), b"hello");
    assert_eq!(blobs.get("bbb").unwrap(), b"");
    assert!(!blobs.contains_key("ccc"));
    assert_eq!(blob_header(b":./gone missing"), None);
}
