use super::*;
use crate::Update;
use std::{
    collections::HashMap,
    sync::mpsc,
    time::{Duration, Instant},
};
#[test]
fn explorer_watches_only_visible_directories() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let target = root.join("target");
    let modules = root.join("node_modules");
    let git_dir = root.join(".git");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::create_dir_all(&modules).unwrap();
    std::fs::create_dir_all(&git_dir).unwrap();
    let context = services::ContextData {
        cwd: root.clone(),
        root: Some(root.clone()),
        perforce: None,
        git_dirs: vec![git_dir.clone()],
        branch: String::new(),
        changes: Vec::new(),
        decorations: HashMap::default(),
        stats: HashMap::default(),
        error: None,
    };
    let mut request = Request {
        cwd: root.clone(),
        generation: 1,
        directories: vec![root.clone()],
        include_stats: false,
    };
    assert_eq!(
        watch_targets(&request, &context),
        vec![
            (root.clone(), RecursiveMode::NonRecursive),
            (git_dir.clone(), RecursiveMode::Recursive),
        ]
    );
    request.directories.push(target.clone());
    assert!(watch_targets(&request, &context).contains(&(target, RecursiveMode::NonRecursive)));
    assert!(
        !watch_targets(&request, &context)
            .iter()
            .any(|(path, _)| path == &modules)
    );
    request.include_stats = true;
    assert_eq!(
        watch_targets(&request, &context),
        vec![(root, RecursiveMode::Recursive)]
    );
}

#[tokio::test]
async fn visible_directory_arrives_before_repository_refresh() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ready.md"), "# Ready\n").unwrap();
    let (legacy, _) = mpsc::channel();
    let paths = terminator_core::Paths::at(dir.path().join("app-state"));
    let (service, mut owner) =
        crate::gui_services::Services::new(paths, eframe::egui::Context::default(), legacy)
            .unwrap();
    let refresh = spawn_async(service);
    refresh
        .send(Some(Request {
            cwd: dir.path().to_owned(),
            generation: 1,
            directories: vec![dir.path().to_owned()],
            include_stats: false,
        }))
        .unwrap();
    let first = tokio::time::timeout(Duration::from_secs(5), owner.events.recv())
        .await
        .unwrap()
        .unwrap();
    let Update::DirectoryPreview(1, _, path, Ok(entries)) = first else {
        panic!("directory entries must arrive before Git status");
    };
    assert_eq!(path, dir.path());
    assert!(entries.iter().any(|entry| entry.path.ends_with("ready.md")));
}

#[test]
fn watch_events_relist_only_the_folders_they_touch() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let (src, docs) = (root.join("src"), root.join("docs"));
    std::fs::create_dir_all(src.join("nested")).unwrap();
    std::fs::create_dir_all(&docs).unwrap();
    let visible = [root.clone(), src.clone(), src.join("nested"), docs.clone()];
    let listed = |_: &std::path::Path| true;
    let list = |dirty: &[PathBuf], full| {
        directories_to_list(DirectoriesToListInput {
            directories: &visible,
            listed: &listed,
            dirty,
            full,
        })
    };
    // A new file re-lists its own folder only.
    assert_eq!(
        list(&[src.join("new.rs")], false),
        std::slice::from_ref(&src)
    );
    // A moved or deleted folder re-lists its parent and itself (and
    // everything under it).
    assert_eq!(
        list(std::slice::from_ref(&src), false),
        [root.clone(), src.clone(), src.join("nested")]
    );
    assert!(list(&[], false).is_empty());
    assert_eq!(list(&[], true), visible);
    // Folders never listed are always listed.
    let fresh = directories_to_list(DirectoriesToListInput {
        directories: &visible,
        listed: &|path| path != docs,
        dirty: &[],
        full: false,
    });
    assert_eq!(fresh, std::slice::from_ref(&docs));
    assert!(ignore_rules_changed(&[root.join(".gitignore")]));
    assert!(!ignore_rules_changed(&[src.join("lib.rs")]));
}

#[test]
fn settling_keeps_events_that_arrived_during_a_refresh() {
    let event =
        |path: &str| Ok(notify::Event::new(notify::EventKind::Any).add_path(PathBuf::from(path)));
    let mut dirty = Dirty::default();
    dirty.record(&event("a"));
    let seen = dirty.clone();
    dirty.settle(&seen);
    assert!(dirty.paths.is_empty() && !dirty.full);
    dirty.record(&event("b"));
    let seen = dirty.clone();
    dirty.record(&event("c"));
    dirty.settle(&seen);
    assert_eq!(dirty.paths, [PathBuf::from("c")]);
    // A rescan request during a full refresh stays for the next one.
    dirty.record(&Err(notify::Error::generic("overflow")));
    let seen = dirty.clone();
    dirty.record(&event("d"));
    dirty.settle(&seen);
    assert!(dirty.full);
}

#[tokio::test]
async fn a_watch_event_relists_only_the_changed_folder() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let sub = root.join("sub");
    std::fs::create_dir(&sub).unwrap();
    std::fs::write(root.join("keep.txt"), "").unwrap();
    let (legacy, _) = mpsc::channel();
    let paths = terminator_core::Paths::at(root.join("app-state"));
    let (service, mut owner) =
        crate::gui_services::Services::new(paths, eframe::egui::Context::default(), legacy)
            .unwrap();
    let refresh = spawn_async(service);
    refresh
        .send(Some(Request {
            cwd: root.clone(),
            generation: 1,
            directories: vec![root.clone(), sub.clone()],
            include_stats: false,
        }))
        .unwrap();
    async fn next(events: &mut tokio::sync::mpsc::Receiver<Update>) -> Update {
        tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .expect("refresh update")
            .expect("channel open")
    }
    while !matches!(next(&mut owner.events).await, Update::Refresh(1, ..)) {}
    // Let the watcher settle, then add a file inside `sub`.
    tokio::time::sleep(Duration::from_millis(500)).await;
    std::fs::write(sub.join("added.rs"), "").unwrap();
    let mut previews = Vec::new();
    loop {
        match next(&mut owner.events).await {
            Update::DirectoryPreview(_, _, path, _) => previews.push(path),
            Update::Refresh(1, _, directories, _) => {
                let has = |path: &PathBuf, name: &str| {
                    directories.iter().any(|(listed, entries)| {
                        listed == path
                            && entries
                                .as_ref()
                                .is_ok_and(|entries| entries.iter().any(|e| e.path.ends_with(name)))
                    })
                };
                if has(&sub, "added.rs") {
                    // The untouched root comes from the cache, intact.
                    assert!(has(&root, "keep.txt"));
                    assert!(previews.contains(&sub));
                    assert!(!previews.contains(&root), "root re-listed: {previews:?}");
                    break;
                }
            }
            _ => {}
        }
    }
}

#[test]
fn dirty_path_overflow_becomes_one_full_rescan() {
    let mut paths = vec![PathBuf::from("old"); MAX_DIRTY_PATHS];
    let mut full = false;
    accumulate_dirty(&mut paths, vec![PathBuf::from("new")], &mut full);
    assert!(full);
    assert!(paths.is_empty());
    accumulate_dirty(&mut paths, vec![PathBuf::from("later")], &mut full);
    assert!(paths.is_empty());
}

#[test]
fn watches_atomic_saves_and_suspends_when_hidden() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, rx) = mpsc::channel();
    let coordinator = spawn(tx, eframe::egui::Context::default());
    coordinator
        .send(Some(Request {
            cwd: dir.path().into(),
            generation: 1,
            directories: vec![dir.path().into()],
            include_stats: false,
        }))
        .unwrap();
    assert!(matches!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        Update::Refresh(1, ..)
    ));
    std::fs::write(dir.path().join("temporary"), "content").unwrap();
    std::fs::rename(dir.path().join("temporary"), dir.path().join("saved.rs")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let Update::Refresh(_, _, dirs, _) = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap()
        else {
            continue;
        };
        if dirs
            .iter()
            .flat_map(|(_, entries)| entries.as_ref().into_iter().flatten())
            .any(|e| e.path.ends_with("saved.rs"))
        {
            break;
        }
    }
    coordinator.send(None).unwrap();
    std::thread::sleep(Duration::from_millis(150));
    while rx.try_recv().is_ok() {}
    std::fs::remove_file(dir.path().join("saved.rs")).unwrap();
    assert!(rx.recv_timeout(Duration::from_millis(500)).is_err());
    coordinator
        .send(Some(Request {
            cwd: dir.path().into(),
            generation: 2,
            directories: vec![dir.path().into()],
            include_stats: false,
        }))
        .unwrap();
    let Update::Refresh(2, _, dirs, _) = rx.recv_timeout(Duration::from_secs(5)).unwrap() else {
        panic!("expected reopened refresh")
    };
    assert!(
        !dirs
            .iter()
            .flat_map(|(_, entries)| entries.as_ref().into_iter().flatten())
            .any(|e| e.path.ends_with("saved.rs"))
    );
}
#[test]
fn forced_retry_refreshes_an_unchanged_path_without_a_watch_event() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, rx) = mpsc::channel();
    let coordinator = spawn(tx, eframe::egui::Context::default());
    for generation in [1, 2] {
        coordinator
            .send(Some(Request {
                cwd: dir.path().into(),
                generation,
                directories: vec![dir.path().into()],
                include_stats: false,
            }))
            .unwrap();
        let Update::Refresh(actual, _, dirs, _) = rx.recv_timeout(Duration::from_secs(5)).unwrap()
        else {
            panic!("Expected refresh");
        };
        assert_eq!(actual, generation);
        assert!(dirs[0].1.as_ref().unwrap().is_empty());
    }
}

#[test]
fn missing_cwd_refresh_stops_after_three_attempts() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().join("deleted-worktree");
    let (tx, rx) = mpsc::channel();
    let coordinator = spawn(tx, eframe::egui::Context::default());
    coordinator
        .send(Some(Request {
            cwd: cwd.clone(),
            generation: 1,
            directories: vec![cwd.clone()],
            include_stats: false,
        }))
        .unwrap();
    let mut refreshes = 0;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Update::Refresh(1, ..)) => refreshes += 1,
            Ok(_) => panic!("unexpected refresh update"),
            Err(_) => {
                if refreshes >= 3 {
                    break;
                }
            }
        }
    }
    assert_eq!(refreshes, 3, "a missing directory is refreshed three times");
    assert!(
        rx.recv_timeout(Duration::from_millis(500)).is_err(),
        "refresh keeps running after the retry limit"
    );
    std::fs::create_dir(&cwd).unwrap();
    coordinator
        .send(Some(Request {
            cwd,
            generation: 2,
            directories: vec![dir.path().join("deleted-worktree")],
            include_stats: false,
        }))
        .unwrap();
    assert!(matches!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        Update::Refresh(2, ..)
    ));
}
