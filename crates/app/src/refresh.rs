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
    let callback_overflow = overflow.clone();
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
    let mut watched = Vec::<PathBuf>::new();
    let mut active = None::<Request>;
    let mut pending = None::<Instant>;
    let mut last = Instant::now();
    let mut cache = HashMap::new();
    let mut directory_cache = HashMap::new();
    let mut dirty_paths = Vec::<PathBuf>::new();
    let mut refresh_all = true;
    let mut watch_ok = watcher.is_some();
    loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(mut request) => {
                while let Ok(newer) = rx.try_recv() {
                    request = newer;
                }
                if active != request {
                    active = request;
                    refresh_all = true;
                    pending = Some(Instant::now() - Duration::from_secs(1));
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
                for path in watched.drain(..) {
                    let _ = watcher.unwatch(&path);
                }
            }
            pending = None;
            continue;
        };
        let interval = if watch_ok {
            Duration::from_secs(30)
        } else {
            Duration::from_secs(3)
        };
        if last.elapsed() >= interval {
            pending = Some(Instant::now() - Duration::from_secs(1));
            cache.clear();
            refresh_all = true;
        }
        if !pending.is_some_and(|at| at.elapsed() >= Duration::from_millis(250)) {
            continue;
        }
        pending = None;
        let context = services::context_cached(&request.cwd, &mut cache);
        let mut targets = Vec::new();
        targets.push(context.root.clone().unwrap_or(request.cwd.clone()));
        targets.extend(context.git_dirs.iter().cloned());
        // FSEvents delivers canonical paths. Register those same paths rather
        // than aliases such as macOS /var or /tmp, including linked worktrees.
        targets = targets.into_iter().map(|path| normalize(&path)).collect();
        targets.sort();
        targets.dedup();
        let all_targets = targets.clone();
        targets.retain(|path| {
            !all_targets
                .iter()
                .any(|parent| parent != path && path.starts_with(parent))
        });
        if targets != watched || !watch_ok {
            watch_ok = watcher.is_some();
            if let Some(watcher) = &mut watcher {
                for path in watched.drain(..) {
                    let _ = watcher.unwatch(&path);
                }
                for path in &targets {
                    if watcher.watch(path, RecursiveMode::Recursive).is_err() {
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
    }
}

#[cfg(test)]
const MAX_DIRTY_PATHS: usize = 1024;

#[cfg(test)]
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
        let watcher = service.fs().run(&token, move || {
            Ok(notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                if !event.as_ref().is_ok_and(|e| matches!(e.kind, notify::EventKind::Access(_))) {
                    events.send_modify(|n| *n = n.wrapping_add(1));
                }
            }).ok())
        }).await?;
        let watcher = std::sync::Arc::new(std::sync::Mutex::new(watcher));
        let mut watched = Vec::<PathBuf>::new();
        loop {
            let request = requests.borrow_and_update().clone();
            let Some(request) = request else {
                tokio::select! { _ = token.cancelled() => break, result = requests.changed() => if result.is_err() { break } }
                continue;
            };
            let operation = token.child_token();
            let _guard = operation.clone().drop_guard();
            let work = async {
                let context = services::context_async(&service, request.cwd.clone(), &operation).await;
                let mut targets = vec![context.root.clone().unwrap_or(request.cwd.clone())];
                targets.extend(context.git_dirs.iter().cloned());
                let old = watched.clone(); let cache = watcher.clone();
                let (targets, fallback) = service.fs().run(&operation, move || {
                    let mut targets: Vec<_> = targets.iter().map(|p| normalize(p)).collect();
                    targets.sort(); targets.dedup();
                    let all = targets.clone(); targets.retain(|p| !all.iter().any(|parent| parent != p && p.starts_with(parent)));
                    let mut watcher = cache.lock().unwrap();
                    let mut fallback = watcher.is_none();
                    if let Some(watcher) = watcher.as_mut() && old != targets {
                        for path in old { let _ = watcher.unwatch(&path); }
                        for path in &targets { if watcher.watch(path, RecursiveMode::Recursive).is_err() { fallback = true; } }
                    }
                    Ok((targets, fallback))
                }).await?;
                let mut directories = Vec::new();
                for path in &request.directories {
                    let entries = services::directory_async(&service, path.clone(), context.root.clone(), &operation).await;
                    directories.push((path.clone(), entries));
                }
                service.emit_read(Update::Refresh(request.generation, context, directories, fallback)).await?;
                Ok::<_, anyhow::Error>((targets, fallback))
            };
            let result = tokio::select! {
                _ = token.cancelled() => break,
                result = requests.changed() => { if result.is_err() { break; } continue; }
                result = work => result,
            };
            let fallback = match result { Ok((targets, fallback)) => { watched = targets; fallback }, Err(error) => { service.emit(Update::Error(format!("Refresh: {error:#}"))).await?; true } };
            tokio::select! {
                _ = token.cancelled() => break,
                result = requests.changed() => if result.is_err() { break; },
                _ = changed.changed() => {
                    // Fixed coalescing window cannot be extended by a continuous writer.
                    tokio::select! { _ = token.cancelled() => break, _ = tokio::time::sleep(Duration::from_millis(250)) => {} }
                    changed.borrow_and_update();
                },
                _ = tokio::time::sleep(Duration::from_secs(if fallback { 3 } else { 30 })) => {},
            }
        }
        service.fs().run(&CancellationToken::new(), move || { drop(watcher); Ok(()) }).await?;
        Ok(Vec::new())
    });
    sender
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
