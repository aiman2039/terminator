//! One refresh coordinator: coalesced invalidations and no overlapping Git commands.
use crate::{Update, services};
use notify::{RecursiveMode, Watcher};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub cwd: PathBuf,
    pub generation: u64,
    pub directories: Vec<PathBuf>,
}
pub fn spawn(tx: Sender<Update>, ctx: eframe::egui::Context) -> Sender<Option<Request>> {
    let (send, rx) = mpsc::channel();
    thread::spawn(move || run(rx, tx, ctx));
    send
}
fn run(rx: Receiver<Option<Request>>, tx: Sender<Update>, ctx: eframe::egui::Context) {
    let (events, event_rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = events.send(event);
    })
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
        while let Ok(event) = event_rx.try_recv() {
            match event {
                Ok(event) if !matches!(event.kind, notify::EventKind::Access(_)) => {
                    if event.need_rescan() {
                        refresh_all = true;
                    }
                    dirty_paths.extend(event.paths.into_iter().map(|p| normalize(&p)));
                    pending = Some(Instant::now())
                }
                Ok(_) => {}
                Err(_) => {
                    watch_ok = false;
                    refresh_all = true;
                    pending = Some(Instant::now());
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

fn normalize(path: &std::path::Path) -> PathBuf {
    if let Ok(path) = path.canonicalize() {
        return path;
    }
    if let (Some(parent), Some(name)) = (path.parent(), path.file_name()) {
        return normalize(parent).join(name);
    }
    path.into()
}

#[cfg(test)]
mod tests {
    use super::*;
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
