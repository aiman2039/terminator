use super::*;
#[tokio::test]
async fn batched_ignore_check_marks_entries_across_directories() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("repo");
    fs::create_dir_all(root.join("src")).unwrap();
    let git = |args: &[&str]| {
        terminator_git::run_blocking(&root, args, CommandOptions::default()).unwrap();
    };
    git(&["init", "-q"]);
    fs::write(root.join(".gitignore"), "*.log\nbuild/\n").unwrap();
    for file in ["a.log", "keep.rs", "src/b.log", "src/c.rs"] {
        fs::write(root.join(file), "").unwrap();
    }
    fs::create_dir(root.join("build")).unwrap();
    let (updates, _) = std::sync::mpsc::channel();
    let (service, _owner) = crate::gui_services::Services::new(
        terminator_core::Paths::at(dir.path().join("state")),
        eframe::egui::Context::default(),
        updates,
    )
    .unwrap();
    let cancel = terminator_core::async_service::CancellationToken::new();
    let listings = vec![
        (
            root.clone(),
            entries_raw(&root, &cancel).map_err(|_| unreachable!()),
        ),
        (
            root.join("src"),
            entries_raw(&root.join("src"), &cancel).map_err(|_| unreachable!()),
        ),
    ];
    let listings = directories_with_ignored_async(DirectoriesWithIgnoredAsyncInput {
        service: &service,
        cwd: &root,
        listings,
        in_git: true,
    })
    .await;
    let ignored: Vec<String> = listings
        .iter()
        .flat_map(|(_, entries)| entries.as_ref().unwrap())
        .filter(|entry| entry.ignored && !entry.path.ends_with(".git"))
        .map(|entry| {
            entry
                .path
                .strip_prefix(&root)
                .unwrap()
                .display()
                .to_string()
        })
        .collect();
    assert_eq!(ignored, ["build", "a.log", "src/b.log"]);
}

#[test]
fn merge_stats_adds_index_worktree_and_untracked_lines() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let tracked = root.join("a.txt");
    fs::write(&tracked, "one\ntwo\nthree\n").unwrap();
    fs::write(root.join("new.txt"), "x\ny\n").unwrap();
    let changes = vec![
        Change {
            path: tracked.clone(),
            status: "MM".into(),
        },
        Change {
            path: root.join("new.txt"),
            status: "??".into(),
        },
    ];
    let stats = merge_stats(MergeStatsInput {
        root,
        working: b"1\t1\ta.txt\0",
        staged: b"3\t0\ta.txt\0",
        changes: &changes,
    });
    assert_eq!(stats[&tracked], (4, 1));
    assert_eq!(stats[&root.join("new.txt")], (2, 0));
}

#[test]
fn reopening_a_project_through_a_path_alias_preserves_its_identity() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("project");
    let alias = dir.path().join("old-location");
    fs::create_dir(&real).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&real, &alias).unwrap();
    let projects = vec![terminator_core::Project {
        id: "original".into(),
        name: "Project".into(),
        path: alias,
        layout: serde_json::Value::Null,
    }];
    assert_eq!(
        project_for_directory(&projects, &real.canonicalize().unwrap())
            .unwrap()
            .id,
        "original"
    );
    assert!(project_for_directory(&projects, dir.path()).is_none());
}

#[test]
fn untracked_descendants_get_individual_decorations() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    terminator_git::run_blocking(
        &root,
        &["init", "-q"],
        CommandOptions {
            stdout_limit: 4 * 1024 * 1024,
            ..Default::default()
        },
    )
    .unwrap();
    fs::create_dir_all(root.join("new/nested")).unwrap();
    fs::write(root.join("new/nested/space file.rs"), "").unwrap();
    let context = context_cached(&root, &mut std::collections::HashMap::default(), true);
    assert_eq!(context.changes.len(), 1);
    for relative in ["new", "new/nested", "new/nested/space file.rs"] {
        assert_eq!(context.decorations[&root.join(relative)], 'U');
    }
    assert_eq!(
        context.changes[0].letter(terminator_git::GitGroup::Untracked),
        'U'
    );
}

#[test]
fn space_and_line_paths() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("space file.rs"), "hello").unwrap();
    let (p, line) = resolve_path("space file.rs:12:3", tmp.path()).unwrap();
    assert_eq!(p.file_name().unwrap(), "space file.rs");
    assert_eq!(line, Some(12));
    let target = resolve_target("space file.rs:12:3", tmp.path()).unwrap();
    assert!(matches!(target, Target::File(_, Some(12), Some(3))));
}

#[test]
fn empty_and_failed_directory_reads_are_distinct() {
    let dir = tempfile::tempdir().unwrap();
    assert!(directory_result(dir.path(), false).unwrap().is_empty());
    let missing = dir.path().join("missing");
    let error = directory_result(&missing, false).unwrap_err();
    assert_eq!(error.path, missing);
    assert_eq!(error.kind, std::io::ErrorKind::NotFound);
    assert!(!error.message.is_empty());
}

#[test]
fn incomplete_listing_reports_its_limit_instead_of_returning_partial_success() {
    let dir = tempfile::tempdir().unwrap();
    for n in 0..=terminator_git::DISPLAY_LIMIT {
        fs::write(dir.path().join(n.to_string()), "").unwrap();
    }
    let error = directory_result(dir.path(), false).unwrap_err();
    assert!(error.message.contains("entry display limit"));
}

#[test]
fn compact_path_replaces_home_prefix_with_tilde() {
    let home = Path::new("/Users/me");
    assert_eq!(
        compact_path_under(&home.join("docs/PANIC_AUDIT.md"), Some(home)),
        "~/docs/PANIC_AUDIT.md"
    );
    assert_eq!(compact_path_under(home, Some(home)), "~");
    assert_eq!(
        compact_path_under(Path::new("/tmp/terminator-paste.png"), Some(home)),
        "/tmp/terminator-paste.png"
    );
    assert_eq!(
        compact_path_under(Path::new("/Users/me-other/docs/a.md"), Some(home)),
        "/Users/me-other/docs/a.md"
    );
    assert_eq!(
        compact_path_under(Path::new("/Users/me/docs/a.md"), None),
        "/Users/me/docs/a.md"
    );
    assert_eq!(
        compact_path_under(Path::new("/Users/me/docs/a.md"), Some(Path::new(""))),
        "/Users/me/docs/a.md"
    );
}

#[test]
fn target_compact_shortens_home_and_keeps_the_full_path_for_copy() {
    let home = PathBuf::from(std::env::var_os("HOME").expect("HOME"));
    let path = home.join("docs/a.md");
    let target = Target::File(path.clone(), Some(12), Some(3));
    assert_eq!(target.compact(), "~/docs/a.md:12:3");
    assert_eq!(target.display(), format!("{}:12:3", path.display()));
    assert!(!Target::File(path, Some(12), None).display().contains('~'));
    let url = Target::Url("https://example.com/a".into());
    assert_eq!(url.compact(), url.display());
    assert_eq!(
        Target::File(PathBuf::from("/tmp/terminator-paste.png"), None, None).compact(),
        "/tmp/terminator-paste.png"
    );
}
