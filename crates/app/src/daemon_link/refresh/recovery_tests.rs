//! Watcher recovery: deleted roots, symlink targets, overflow, slow consumers.
use super::*;
use crate::Update;
use std::{path::Path, process::Command, sync::mpsc, time::Duration};
use tokio::sync::mpsc::Receiver;

type Listings = Vec<(
    PathBuf,
    Result<Vec<terminator_git::Entry>, services::DirectoryError>,
)>;

struct Worker {
    refresh: tokio::sync::watch::Sender<Option<Request>>,
    owner: crate::gui_services::Owner,
}

fn worker(state: &Path) -> Worker {
    let (legacy, _) = mpsc::channel();
    let paths = terminator_core::Paths::at(state.to_owned());
    let (service, owner) =
        crate::gui_services::Services::new(paths, eframe::egui::Context::default(), legacy)
            .unwrap();
    Worker {
        refresh: spawn_async(service),
        owner,
    }
}

fn request(cwd: &Path, generation: u64, directories: &[PathBuf], stats: bool) -> Request {
    Request {
        cwd: cwd.to_owned(),
        generation,
        directories: directories.to_vec(),
        include_stats: stats,
    }
}

fn lists(directories: &Listings, folder: &Path, name: &str) -> bool {
    directories.iter().any(|(listed, entries)| {
        listed == folder
            && entries
                .as_ref()
                .is_ok_and(|entries| entries.iter().any(|entry| entry.path.ends_with(name)))
    })
}

/// Waits for a refresh of `generation` that satisfies `done`, counting the
/// refresh worker's updates (daemon polling noise is not counted).
async fn refresh_until(
    events: &mut Receiver<Update>,
    generation: u64,
    seconds: u64,
    done: impl Fn(&services::ContextData, &Listings) -> bool,
) -> usize {
    let deadline = tokio::time::Instant::now()
        .checked_add(Duration::from_secs(seconds))
        .unwrap();
    let mut updates = 0_usize;
    loop {
        let update = tokio::time::timeout_at(deadline, events.recv())
            .await
            .expect("refresh recovered before the deadline")
            .expect("refresh worker running");
        if matches!(update, Update::Refresh(..) | Update::DirectoryPreview(..)) {
            updates = updates.saturating_add(1);
        }
        if let Update::Refresh(actual, context, directories, _) = update
            && actual == generation
            && done(&context, &directories)
        {
            return updates;
        }
    }
}

fn git(root: &Path, args: &[&str]) {
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

#[test]
fn event_burst_stays_bounded_and_wakes_the_worker_once() {
    let dirty = std::sync::Mutex::new(Dirty::default());
    let (events, mut changed) = tokio::sync::watch::channel(0_u64);
    changed.borrow_and_update();
    for index in 0..10_000 {
        let event = notify::Event::new(notify::EventKind::Create(notify::event::CreateKind::File))
            .add_path(PathBuf::from(format!("/burst/{index}")));
        live::record_event(&dirty, &events, Ok(event));
    }
    let access = notify::Event::new(notify::EventKind::Access(notify::event::AccessKind::Any))
        .add_path(PathBuf::from("/burst/read"));
    live::record_event(&dirty, &events, Ok(access));
    let state = dirty.lock().unwrap().clone();
    assert!(state.full, "an overflowing burst becomes one full rescan");
    assert!(state.paths.is_empty(), "no path backlog is kept");
    assert_eq!(state.epoch, 10_000, "access events are ignored");
    // A slow consumer sees one pending wake, not 10 000 queued events.
    assert!(changed.has_changed().unwrap());
    assert_eq!(*changed.borrow_and_update(), 10_000);
    assert!(!changed.has_changed().unwrap());
}

#[test]
fn rescan_flag_and_watch_errors_relist_every_visible_folder() {
    let rescan = notify::Event::new(notify::EventKind::Other)
        .set_flag(notify::event::Flag::Rescan)
        .add_path(PathBuf::from("/root"));
    for event in [Ok(rescan), Err(notify::Error::generic("queue overflow"))] {
        let mut dirty = Dirty::default();
        dirty.record(&Ok(
            notify::Event::new(notify::EventKind::Any).add_path(PathBuf::from("/root/a"))
        ));
        dirty.record(&event);
        assert!(dirty.full && dirty.paths.is_empty());
        let visible = [PathBuf::from("/root"), PathBuf::from("/root/b")];
        let relisted = directories_to_list(DirectoriesToListInput {
            directories: &visible,
            listed: &|_| true,
            dirty: &dirty.paths,
            full: dirty.full,
        });
        assert_eq!(relisted, visible);
    }
}

#[cfg(unix)]
#[test]
fn symlink_seen_dangling_mid_retarget_relists_its_folder() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let target = base.join("new-target");
    std::fs::create_dir(&target).unwrap();
    let link = base.join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    // The watcher normalized the event while the link did not exist yet.
    let relisted = directories_to_list(DirectoriesToListInput {
        directories: &[base.clone(), link.clone()],
        listed: &|_| true,
        dirty: std::slice::from_ref(&link),
        full: false,
    });
    assert_eq!(relisted, [base, link]);
}

#[test]
fn recreated_root_is_watched_again_under_the_same_path() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap().join("root");
    std::fs::create_dir(&root).unwrap();
    let (tx, rx) = mpsc::channel();
    let state = std::sync::Mutex::new(live::WatchState::new(Box::new(move || {
        let tx = tx.clone();
        notify::recommended_watcher(move |event| {
            let _ = tx.send(event);
        })
        .ok()
    })));
    if state.lock().unwrap().watcher.is_none() {
        return;
    }
    let targets = vec![(root.clone(), RecursiveMode::NonRecursive)];
    assert!(!live::rewatch(&state, targets.clone()));
    std::fs::remove_dir(&root).unwrap();
    std::fs::create_dir(&root).unwrap();
    // Same target list, new directory: the watch is replaced.
    assert!(!live::rewatch(&state, targets));
    std::thread::sleep(Duration::from_millis(300));
    while rx.try_recv().is_ok() {}
    std::fs::write(root.join("after.txt"), "").unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let event = rx
            .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
            .expect("an event from the recreated root");
        if event.is_ok_and(|event| event.paths.iter().any(|p| p.ends_with("after.txt"))) {
            break;
        }
    }
}

#[test]
fn failed_watch_keeps_polling_until_the_path_returns() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().canonicalize().unwrap().join("missing");
    let state = std::sync::Mutex::new(live::WatchState::new(Box::new(|| {
        notify::recommended_watcher(|_: notify::Result<notify::Event>| {}).ok()
    })));
    if state.lock().unwrap().watcher.is_none() {
        return;
    }
    let targets = vec![(missing.clone(), RecursiveMode::NonRecursive)];
    assert!(live::rewatch(&state, targets.clone()));
    assert!(
        live::rewatch(&state, targets.clone()),
        "a later pass with the same failed targets still polls"
    );
    std::fs::create_dir(&missing).unwrap();
    assert!(
        !live::rewatch(&state, targets),
        "the returned path is watched"
    );
}

#[tokio::test]
async fn deleted_and_recreated_root_refreshes_and_keeps_watching() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let root = base.join("project");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("before.txt"), "").unwrap();
    let mut w = worker(&base.join("state"));
    let visible = [root.clone()];
    w.refresh
        .send(Some(request(&root, 1, &visible, false)))
        .unwrap();
    refresh_until(&mut w.owner.events, 1, 20, |_, d| {
        lists(d, &root, "before.txt")
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    std::fs::remove_dir_all(&root).unwrap();
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("recreated.txt"), "").unwrap();
    // The GUI retries a vanished project with a new generation.
    w.refresh
        .send(Some(request(&root, 2, &visible, false)))
        .unwrap();
    refresh_until(&mut w.owner.events, 2, 20, |_, d| {
        lists(d, &root, "recreated.txt") && !lists(d, &root, "before.txt")
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    std::fs::write(root.join("after.txt"), "").unwrap();
    // Well inside the 30 s safety poll: only a live watch sees this.
    refresh_until(&mut w.owner.events, 2, 20, |_, d| {
        lists(d, &root, "after.txt")
    })
    .await;
}

#[cfg(unix)]
#[tokio::test]
async fn symlinked_folder_follows_retarget_removal_and_recreation() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let (root, first, second) = (base.join("project"), base.join("a"), base.join("b"));
    for folder in [&root, &first, &second] {
        std::fs::create_dir(folder).unwrap();
    }
    std::fs::write(first.join("a.txt"), "").unwrap();
    std::fs::write(second.join("b.txt"), "").unwrap();
    let link = root.join("link");
    std::os::unix::fs::symlink(&first, &link).unwrap();
    let mut w = worker(&base.join("state"));
    let visible = [root.clone(), link.clone()];
    w.refresh
        .send(Some(request(&root, 1, &visible, false)))
        .unwrap();
    refresh_until(&mut w.owner.events, 1, 10, |_, d| lists(d, &link, "a.txt")).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    // Retarget: the link now shows the second folder and watches it.
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&second, &link).unwrap();
    refresh_until(&mut w.owner.events, 1, 10, |_, d| lists(d, &link, "b.txt")).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    std::fs::write(second.join("b2.txt"), "").unwrap();
    refresh_until(&mut w.owner.events, 1, 10, |_, d| lists(d, &link, "b2.txt")).await;
    // Target removed: the stale listing goes away.
    std::fs::remove_dir_all(&second).unwrap();
    refresh_until(&mut w.owner.events, 1, 10, |_, d| {
        !lists(d, &link, "b2.txt")
    })
    .await;
    // Target recreated outside the watched project: polling finds it.
    std::fs::create_dir(&second).unwrap();
    std::fs::write(second.join("c.txt"), "").unwrap();
    refresh_until(&mut w.owner.events, 1, 10, |_, d| lists(d, &link, "c.txt")).await;
}

#[tokio::test]
async fn overflow_with_a_slow_consumer_refreshes_tree_and_git_once() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let root = base.join("repo");
    let burst = root.join("burst");
    std::fs::create_dir_all(&burst).unwrap();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "fixture@example.invalid"]);
    git(&root, &["config", "user.name", "Fixture"]);
    std::fs::write(root.join("tracked.txt"), "one\n").unwrap();
    git(&root, &["add", "tracked.txt"]);
    git(&root, &["commit", "-q", "-m", "init"]);
    let mut w = worker(&base.join("state"));
    let visible = [root.clone(), burst.clone()];
    w.refresh
        .send(Some(request(&root, 1, &visible, true)))
        .unwrap();
    refresh_until(&mut w.owner.events, 1, 10, |c, _| c.root.is_some()).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let files = MAX_DIRTY_PATHS + 200;
    for index in 0..files {
        std::fs::write(burst.join(format!("file-{index:04}.txt")), "").unwrap();
    }
    std::fs::write(root.join("tracked.txt"), "two\n").unwrap();
    // The consumer stalls while the burst is recorded.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let last = format!("file-{:04}.txt", files - 1);
    let updates = refresh_until(&mut w.owner.events, 1, 20, |context, directories| {
        let count = directories
            .iter()
            .find(|(path, _)| path == &burst)
            .and_then(|(_, entries)| entries.as_ref().ok())
            .map_or(0, Vec::len);
        count == files
            && lists(directories, &burst, &last)
            && context
                .changes
                .iter()
                .any(|change| change.path.ends_with("tracked.txt"))
    })
    .await;
    assert!(
        updates < 64,
        "{files} file events produced {updates} updates; events must coalesce"
    );
}
