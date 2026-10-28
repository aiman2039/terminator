//! One refresh coordinator: coalesced invalidations and no overlapping Git commands.
use crate::{Update, services};
use notify::{RecursiveMode, Watcher};
#[cfg(test)]
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::Instant,
};
use std::{path::PathBuf, time::Duration};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub cwd: PathBuf,
    pub generation: u64,
    pub directories: Vec<PathBuf>,
    /// The Git view needs per-file untracked status, line counts, and a full watch.
    pub include_stats: bool,
}

fn watch_targets(
    request: &Request,
    context: &services::ContextData,
) -> Vec<(PathBuf, RecursiveMode)> {
    let mut targets = std::collections::BTreeMap::new();
    if request.include_stats {
        targets.insert(
            normalize(context.root.as_ref().unwrap_or(&request.cwd)),
            RecursiveMode::Recursive,
        );
    } else {
        for path in &request.directories {
            targets.insert(normalize(path), RecursiveMode::NonRecursive);
        }
    }
    for path in &context.git_dirs {
        targets.insert(normalize(path), RecursiveMode::Recursive);
    }
    let all: Vec<_> = targets.into_iter().collect();
    all.iter()
        .filter(|(path, _)| {
            !all.iter().any(|(parent, mode)| {
                parent != path && *mode == RecursiveMode::Recursive && path.starts_with(parent)
            })
        })
        .cloned()
        .collect()
}
#[cfg(test)]
pub fn spawn(tx: Sender<Update>, ctx: eframe::egui::Context) -> Sender<Option<Request>> {
    let (send, rx) = mpsc::channel();
    thread::spawn(move || run(rx, tx, ctx));
    send
}
#[cfg(test)]
fn run(rx: Receiver<Option<Request>>, tx: Sender<Update>, ctx: eframe::egui::Context) {
    let (events, event_rx) = mpsc::sync_channel(128);
    let overflow = Arc::new(AtomicBool::new(false));
    let callback_overflow = Arc::clone(&overflow);
    // FSEvents stream restart blocks for seconds and misses the test deadline.
    // This harness only checks coalescing and suspend; production uses FSEvents.
    let mut watcher = notify::PollWatcher::new(
        move |event: notify::Result<notify::Event>| {
            if event
                .as_ref()
                .is_ok_and(|event| matches!(event.kind, notify::EventKind::Access(_)))
            {
                return;
            }
            if event
                .as_ref()
                .is_ok_and(|event| event.paths.len() > MAX_DIRTY_PATHS)
                || events.try_send(event).is_err()
            {
                callback_overflow.store(true, Ordering::Release);
            }
        },
        notify::Config::default().with_poll_interval(Duration::from_millis(50)),
    )
    .ok();
    let mut watched = Vec::<(PathBuf, RecursiveMode)>::new();
    let mut active = None::<Request>;
    let mut pending = None::<Instant>;
    let mut last = Instant::now();
    let mut cache = HashMap::new();
    let mut directory_cache = HashMap::new();
    let mut dirty_paths = Vec::<PathBuf>::new();
    let mut refresh_all = true;
    let mut watch_ok = watcher.is_some();
    let mut missing_path = crate::retry_budget::RetryBudget::default();
    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(mut request) => {
                while let Ok(newer) = rx.try_recv() {
                    request = newer;
                }
                if active != request {
                    active = request;
                    refresh_all = true;
                    pending = Some(
                        Instant::now()
                            .checked_sub(Duration::from_secs(1))
                            .unwrap_or_else(Instant::now),
                    );
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(_) => {}
        }
        if overflow.swap(false, Ordering::AcqRel) {
            refresh_all = true;
            dirty_paths.clear();
            pending.get_or_insert_with(Instant::now);
        }
        for _ in 0..128 {
            let Ok(event) = event_rx.try_recv() else {
                break;
            };
            match event {
                Ok(event) if !matches!(event.kind, notify::EventKind::Access(_)) => {
                    if event.need_rescan() {
                        refresh_all = true;
                    }
                    accumulate_dirty(&mut dirty_paths, event.paths, &mut refresh_all);
                    pending.get_or_insert_with(Instant::now);
                }
                Ok(_) => {}
                Err(_) => {
                    watch_ok = false;
                    refresh_all = true;
                    pending.get_or_insert_with(Instant::now);
                }
            }
        }
        let Some(request) = &active else {
            if let Some(watcher) = &mut watcher {
                for (path, _) in watched.drain(..) {
                    let _ = watcher.unwatch(&path);
                }
            }
            pending = None;
            continue;
        };
        if missing_path.exhausted(&request.cwd, request.generation) {
            pending = None;
            continue;
        }
        let interval = if watch_ok {
            Duration::from_secs(30)
        } else {
            Duration::from_secs(3)
        };
        if last.elapsed() >= interval {
            pending = Some(
                Instant::now()
                    .checked_sub(Duration::from_secs(1))
                    .unwrap_or_else(Instant::now),
            );
            cache.clear();
            refresh_all = true;
        }
        if pending.is_none_or(|at| at.elapsed() < Duration::from_millis(250)) {
            continue;
        }
        pending = None;
        let context = services::context_cached(&request.cwd, &mut cache, request.include_stats);
        let targets = watch_targets(request, &context);
        if targets != watched || !watch_ok {
            watch_ok = watcher.is_some();
            if let Some(watcher) = &mut watcher {
                for (path, _) in watched.drain(..) {
                    let _ = watcher.unwatch(&path);
                }
                for (path, mode) in &targets {
                    if watcher.watch(path, *mode).is_err() {
                        watch_ok = false;
                    }
                }
            }
            watched = targets;
        }
        let mut directories = Vec::new();
        let ignore_changed = dirty_paths.iter().any(|p| {
            p.file_name()
                .is_some_and(|n| n == ".gitignore" || n == "exclude" || n == "index")
        });
        for path in &request.directories {
            let canonical = normalize(path);
            let affected = refresh_all
                || ignore_changed
                || !directory_cache.contains_key(path)
                || dirty_paths
                    .iter()
                    .any(|p| p == &canonical || p.parent() == Some(canonical.as_path()));
            if affected {
                let entries = services::directory_result(path, context.root.is_some());
                directory_cache.insert(path.clone(), entries);
            }
            if let Some(entries) = directory_cache.get(path) {
                directories.push((path.clone(), entries.clone()));
            }
        }
        directory_cache.retain(|path, _| request.directories.contains(path));
        dirty_paths.clear();
        refresh_all = false;
        if tx
            .send(Update::Refresh(
                request.generation,
                context,
                directories,
                !watch_ok,
            ))
            .is_err()
        {
            break;
        }
        ctx.request_repaint();
        last = Instant::now();
        let missing = crate::retry_budget::path_missing(&request.cwd);
        missing_path.record(&request.cwd, request.generation, missing);
        if missing && !missing_path.exhausted(&request.cwd, request.generation) {
            pending = Some(
                Instant::now()
                    .checked_sub(Duration::from_secs(1))
                    .unwrap_or_else(Instant::now),
            );
        }
    }
}

const MAX_DIRTY_PATHS: usize = 1024;

/// Paths the watcher reported since the last finished refresh. `full`
/// stands for "too many to track" (or a rescan request); `epoch` counts
/// events, so a refresh can tell whether more arrived while it ran.
#[derive(Clone, Default)]
struct Dirty {
    paths: Vec<PathBuf>,
    full: bool,
    epoch: u64,
}

impl Dirty {
    fn record(&mut self, event: &notify::Result<notify::Event>) {
        self.epoch = self.epoch.wrapping_add(1);
        match event {
            Ok(event) if event.need_rescan() => {
                self.paths.clear();
                self.full = true;
            }
            Ok(event) => accumulate_dirty(&mut self.paths, event.paths.clone(), &mut self.full),
            Err(_) => {
                self.paths.clear();
                self.full = true;
            }
        }
    }

    /// Forget what a finished refresh (`seen`) covered. Events that arrived
    /// meanwhile stay; when that cannot be told apart, everything stays.
    fn settle(&mut self, seen: &Self) {
        if self.epoch == seen.epoch {
            *self = Self {
                epoch: self.epoch,
                ..Self::default()
            };
        } else if !seen.full && !self.full {
            self.paths.drain(..seen.paths.len().min(self.paths.len()));
        }
    }
}

/// Why a refresh runs: only watch events and newly shown folders can
/// re-list part of the tree; forced refreshes and polls re-list it all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wake {
    Request,
    Watch,
    Timer,
}

/// The visible directories a refresh lists again: every one when `full`,
/// else those not listed yet and those a changed path is in (or under a
/// changed path, for moved folders). `dirty` holds normalized paths.
fn directories_to_list(
    directories: &[PathBuf],
    listed: &dyn Fn(&std::path::Path) -> bool,
    dirty: &[PathBuf],
    full: bool,
) -> Vec<PathBuf> {
    directories
        .iter()
        .filter(|path| {
            if full || !listed(path) {
                return true;
            }
            let canonical = normalize(path);
            dirty.iter().any(|changed| {
                changed.parent() == Some(canonical.as_path()) || canonical.starts_with(changed)
            })
        })
        .cloned()
        .collect()
}

/// A change to ignore rules or the index can flip any entry's ignored
/// state, so it re-lists everything.
fn ignore_rules_changed(dirty: &[PathBuf]) -> bool {
    dirty.iter().any(|path| {
        path.file_name()
            .is_some_and(|name| name == ".gitignore" || name == "exclude" || name == "index")
    })
}

fn accumulate_dirty(paths: &mut Vec<PathBuf>, incoming: Vec<PathBuf>, full: &mut bool) {
    if *full || paths.len().saturating_add(incoming.len()) > MAX_DIRTY_PATHS {
        paths.clear();
        *full = true;
        return;
    }
    paths.extend(incoming.into_iter().map(|path| normalize(&path)));
}

fn normalize(path: &std::path::Path) -> PathBuf {
    if let Ok(path) = path.canonicalize() {
        return path;
    }
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
        return normalize(parent).join(name);
    }
    path.into()
}

/// Selection and filesystem notifications are latest-value channels: no event backlog.
pub fn spawn_async(
    service: crate::gui_services::Services,
) -> tokio::sync::watch::Sender<Option<Request>> {
    use terminator_core::async_service::{CancellationToken, OperationContext, Policy};
    let (sender, mut requests) = tokio::sync::watch::channel::<Option<Request>>(None);
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    let mut context = OperationContext::new(
        "refresh",
        "visible-repository".into(),
        Policy::ServiceLifetime,
    );
    context.deadline = None;
    let handle = service.handle().clone();
    let _ = handle.submit(context, cancel, async move {
        let (events, mut changed) = tokio::sync::watch::channel(0_u64);
        let dirty = std::sync::Arc::new(std::sync::Mutex::new(Dirty::default()));
        let recorder = std::sync::Arc::clone(&dirty);
        let watcher = service.fs().run(&token, move || {
            anyhow::Ok(notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if !event.as_ref().is_ok_and(|e| matches!(e.kind, notify::EventKind::Access(_))) {
                    let event = event.map(|mut event| {
                        event.paths = event.paths.iter().map(|path| normalize(path)).collect();
                        event
                    });
                    recorder.lock().unwrap_or_else(std::sync::PoisonError::into_inner).record(&event);
                    events.send_modify(|n| *n = n.wrapping_add(1));
                }
            }).ok())
        }).await?;
        let watcher = std::sync::Arc::new(std::sync::Mutex::new(watcher));
        let mut watched = Vec::<(PathBuf, RecursiveMode)>::new();
        let mut missing_path = crate::retry_budget::RetryBudget::default();
        // Final listings (with ignored flags) of the visible directories,
        // for the request they were listed under: a watch event re-lists
        // only the folders it touched and reuses the rest.
        let mut listed: std::collections::HashMap<PathBuf, services::Listing> = std::collections::HashMap::new();
        let mut listed_for: Option<Request> = None;
        let mut wake = Wake::Request;
        loop {
            let request = requests.borrow_and_update().clone();
            let Some(request) = request else {
                tokio::select! { () = token.cancelled() => break, result = requests.changed() => if result.is_err() { break } }
                wake = Wake::Request;
                continue;
            };
            if missing_path.exhausted(&request.cwd, request.generation) {
                tokio::select! {
                    () = token.cancelled() => break,
                    result = requests.changed() => if result.is_err() { break; },
                }
                continue;
            }
            let operation = token.child_token();
            let _guard = operation.clone().drop_guard();
            let seen = dirty.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
            let same_scope = listed_for.as_ref().is_some_and(|last| {
                last.cwd == request.cwd && last.include_stats == request.include_stats
            });
            // A new generation with the same folders is a forced refresh.
            let forced = wake == Wake::Request
                && listed_for.as_ref().is_some_and(|last| last.directories == request.directories);
            let full = !same_scope
                || forced
                || wake == Wake::Timer
                || seen.full
                || ignore_rules_changed(&seen.paths);
            let known: std::collections::HashSet<PathBuf> = if full { std::collections::HashSet::new() } else { listed.keys().cloned().collect() };
            let wanted = request.directories.clone();
            let changed_paths = seen.paths.clone();
            let relist = service.fs().run(&operation, move || {
                anyhow::Ok(directories_to_list(&wanted, &|path| known.contains(path), &changed_paths, full))
            }).await.unwrap_or_else(|_| request.directories.clone());
            let work = async {
                // Show the bounded directory entries before repository status,
                // line counts, ignore checks, or watcher setup can delay them.
                let mut raw_directories = Vec::new();
                for path in &relist {
                    let entries = services::directory_raw_async(&service, path.clone(), &operation).await;
                    service.emit_read(Update::DirectoryPreview(
                        request.generation,
                        request.cwd.clone(),
                        path.clone(),
                        entries.clone(),
                    )).await?;
                    raw_directories.push((path.clone(), entries));
                }
                let context = services::context_async(
                    &service, request.cwd.clone(), request.include_stats, &operation,
                ).await;
                let targets = watch_targets(&request, &context);
                let old = watched.clone();
                let cache = std::sync::Arc::clone(&watcher);
                let (targets, fallback) = service.fs().run(&operation, move || {
                    let mut watcher = cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    let mut fallback = watcher.is_none();
                    if let Some(watcher) = watcher.as_mut() && old != targets {
                        for (path, _) in old { let _ = watcher.unwatch(&path); }
                        for (path, mode) in &targets { if watcher.watch(path, *mode).is_err() { fallback = true; } }
                    }
                    anyhow::Ok((targets, fallback))
                }).await?;
                let directories = services::directories_with_ignored_async(
                    &service, &request.cwd, raw_directories, context.root.is_some() && context.perforce.is_none(),
                ).await;
                Ok::<_, anyhow::Error>((targets, fallback, context, directories))
            };
            let result = tokio::select! {
                () = token.cancelled() => break,
                result = requests.changed() => { if result.is_err() { break; } wake = Wake::Request; continue; }
                result = work => result,
            };
            let fallback = match result {
                Ok((targets, fallback, context, fresh)) => {
                    watched = targets;
                    if full {
                        listed.clear();
                    }
                    for listing in fresh {
                        listed.insert(listing.0.clone(), listing);
                    }
                    listed.retain(|path, _| request.directories.contains(path));
                    let directories = request
                        .directories
                        .iter()
                        .filter_map(|path| listed.get(path).cloned())
                        .collect();
                    service.emit_read(Update::Refresh(request.generation, context, directories, fallback)).await?;
                    listed_for = Some(request.clone());
                    dirty.lock().unwrap_or_else(std::sync::PoisonError::into_inner).settle(&seen);
                    fallback
                }
                Err(error) => {
                    listed.clear();
                    listed_for = None;
                    service.emit(Update::Error(format!("Refresh: {error:#}"))).await?;
                    true
                }
            };
            let cwd = request.cwd.clone();
            let generation = request.generation;
            let missing = service
                .fs()
                .run(&operation, move || anyhow::Ok(crate::retry_budget::path_missing(&cwd)))
                .await
                .unwrap_or(false);
            missing_path.record(&request.cwd, generation, missing);
            if missing {
                wake = Wake::Timer;
                continue;
            }
            // A missed event cannot be told apart from none in fallback
            // polling, so only a working watcher allows partial refreshes.
            wake = tokio::select! {
                () = token.cancelled() => break,
                result = requests.changed() => { if result.is_err() { break; } Wake::Request },
                _ = changed.changed() => {
                    // Fixed coalescing window cannot be extended by a continuous writer.
                    tokio::select! { () = token.cancelled() => break, () = tokio::time::sleep(Duration::from_millis(250)) => {} }
                    changed.borrow_and_update();
                    if fallback { Wake::Timer } else { Wake::Watch }
                },
                () = tokio::time::sleep(Duration::from_secs(if fallback { 3 } else { 30 })) => Wake::Timer,
            };
        }
        service.fs().run(&CancellationToken::new(), move || { drop(watcher); anyhow::Ok(()) }).await?;
        Ok(Vec::new())
    });
    sender
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let list = |dirty: &[PathBuf], full| directories_to_list(&visible, &listed, dirty, full);
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
        let fresh = directories_to_list(&visible, &|path| path != docs, &[], false);
        assert_eq!(fresh, std::slice::from_ref(&docs));
        assert!(ignore_rules_changed(&[root.join(".gitignore")]));
        assert!(!ignore_rules_changed(&[src.join("lib.rs")]));
    }

    #[test]
    fn settling_keeps_events_that_arrived_during_a_refresh() {
        let event = |path: &str| {
            Ok(notify::Event::new(notify::EventKind::Any).add_path(PathBuf::from(path)))
        };
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
                                && entries.as_ref().is_ok_and(|entries| {
                                    entries.iter().any(|e| e.path.ends_with(name))
                                })
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
        let Update::Refresh(2, _, dirs, _) = rx.recv_timeout(Duration::from_secs(5)).unwrap()
        else {
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
            let Update::Refresh(actual, _, dirs, _) =
                rx.recv_timeout(Duration::from_secs(5)).unwrap()
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
}
