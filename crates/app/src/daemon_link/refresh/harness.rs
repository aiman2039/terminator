//! Thread-based refresh coordinator used by the coalescing and suspend tests.
use super::{MAX_DIRTY_PATHS, Request, accumulate_dirty, normalize, watch_targets};
use crate::{Update, services};
use notify::{RecursiveMode, Watcher};
use std::{
    collections::HashMap,
    ops::ControlFlow,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};

type WatchEvent = notify::Result<notify::Event>;
type Listed = std::result::Result<Vec<terminator_git::Entry>, services::DirectoryError>;

pub fn spawn(tx: Sender<Update>, ctx: eframe::egui::Context) -> Sender<Option<Request>> {
    let (send, rx) = mpsc::channel();
    thread::spawn(move || Harness::new(rx, tx, ctx).run());
    send
}

/// A moment far enough back that the next tick refreshes at once.
fn overdue() -> Instant {
    Instant::now()
        .checked_sub(Duration::from_secs(1))
        .unwrap_or_else(Instant::now)
}

struct Harness {
    rx: Receiver<Option<Request>>,
    tx: Sender<Update>,
    ctx: eframe::egui::Context,
    event_rx: Receiver<WatchEvent>,
    overflow: Arc<AtomicBool>,
    watcher: Option<notify::PollWatcher>,
    watched: Vec<(PathBuf, RecursiveMode)>,
    active: Option<Request>,
    pending: Option<Instant>,
    last: Instant,
    cache: HashMap<PathBuf, (PathBuf, Vec<PathBuf>)>,
    directory_cache: HashMap<PathBuf, Listed>,
    dirty_paths: Vec<PathBuf>,
    refresh_all: bool,
    watch_ok: bool,
    missing_path: crate::retry_budget::RetryBudget,
}

impl Harness {
    fn new(rx: Receiver<Option<Request>>, tx: Sender<Update>, ctx: eframe::egui::Context) -> Self {
        let (events, event_rx) = mpsc::sync_channel(128);
        let overflow = Arc::new(AtomicBool::new(false));
        let callback_overflow = Arc::clone(&overflow);
        // FSEvents stream restart blocks for seconds and misses the test deadline.
        // This harness only checks coalescing and suspend; production uses FSEvents.
        let watcher = notify::PollWatcher::new(
            move |event: WatchEvent| {
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
        let watch_ok = watcher.is_some();
        Self {
            rx,
            tx,
            ctx,
            event_rx,
            overflow,
            watcher,
            watched: Vec::new(),
            active: None,
            pending: None,
            last: Instant::now(),
            cache: HashMap::new(),
            directory_cache: HashMap::new(),
            dirty_paths: Vec::new(),
            refresh_all: true,
            watch_ok,
            missing_path: crate::retry_budget::RetryBudget::default(),
        }
    }

    fn run(mut self) {
        loop {
            if self.receive_request().is_break() {
                break;
            }
            self.drain_events();
            if self.tick().is_break() {
                break;
            }
        }
    }

    /// Takes the newest request; a changed one refreshes everything now.
    fn receive_request(&mut self) -> ControlFlow<()> {
        match self.rx.recv_timeout(Duration::from_millis(50)) {
            Ok(mut request) => {
                while let Ok(newer) = self.rx.try_recv() {
                    request = newer;
                }
                if self.active != request {
                    self.active = request;
                    self.refresh_all = true;
                    self.pending = Some(overdue());
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return ControlFlow::Break(()),
            Err(_) => {}
        }
        ControlFlow::Continue(())
    }

    fn drain_events(&mut self) {
        if self.overflow.swap(false, Ordering::AcqRel) {
            self.refresh_all = true;
            self.dirty_paths.clear();
            self.pending.get_or_insert_with(Instant::now);
        }
        for _ in 0..128 {
            let Ok(event) = self.event_rx.try_recv() else {
                break;
            };
            self.record_event(event);
        }
    }

    fn record_event(&mut self, event: WatchEvent) {
        match event {
            Ok(event) if !matches!(event.kind, notify::EventKind::Access(_)) => {
                if event.need_rescan() {
                    self.refresh_all = true;
                }
                accumulate_dirty(&mut self.dirty_paths, event.paths, &mut self.refresh_all);
                self.pending.get_or_insert_with(Instant::now);
            }
            Ok(_) => {}
            Err(_) => {
                self.watch_ok = false;
                self.refresh_all = true;
                self.pending.get_or_insert_with(Instant::now);
            }
        }
    }

    fn unwatch_all(&mut self) {
        if let Some(watcher) = &mut self.watcher {
            for (path, _) in self.watched.drain(..) {
                let _ = watcher.unwatch(&path);
            }
        }
    }

    fn tick(&mut self) -> ControlFlow<()> {
        let Some(request) = self.active.clone() else {
            self.unwatch_all();
            self.pending = None;
            return ControlFlow::Continue(());
        };
        if self
            .missing_path
            .exhausted(&request.cwd, request.generation)
        {
            self.pending = None;
            return ControlFlow::Continue(());
        }
        self.poll_when_idle();
        if self
            .pending
            .is_none_or(|at| at.elapsed() < Duration::from_millis(250))
        {
            return ControlFlow::Continue(());
        }
        self.pending = None;
        let context =
            services::context_cached(&request.cwd, &mut self.cache, request.include_stats);
        self.update_watches(watch_targets(&request, &context));
        let directories = self.list_directories(&request, context.root.is_some());
        if self
            .tx
            .send(Update::Refresh(
                request.generation,
                context,
                directories,
                !self.watch_ok,
            ))
            .is_err()
        {
            return ControlFlow::Break(());
        }
        self.ctx.request_repaint();
        self.last = Instant::now();
        self.retry_if_missing(&request);
        ControlFlow::Continue(())
    }

    /// Polls on a timer: rarely with a working watcher, often without.
    fn poll_when_idle(&mut self) {
        let interval = if self.watch_ok {
            Duration::from_secs(30)
        } else {
            Duration::from_secs(3)
        };
        if self.last.elapsed() >= interval {
            self.pending = Some(overdue());
            self.cache.clear();
            self.refresh_all = true;
        }
    }

    fn update_watches(&mut self, targets: Vec<(PathBuf, RecursiveMode)>) {
        if targets == self.watched && self.watch_ok {
            return;
        }
        self.watch_ok = self.watcher.is_some();
        self.unwatch_all();
        if let Some(watcher) = &mut self.watcher {
            for (path, mode) in &targets {
                if watcher.watch(path, *mode).is_err() {
                    self.watch_ok = false;
                }
            }
        }
        self.watched = targets;
    }

    fn list_directories(&mut self, request: &Request, in_git: bool) -> Vec<(PathBuf, Listed)> {
        let mut directories = Vec::new();
        let ignore_changed = self.dirty_paths.iter().any(|p| {
            p.file_name()
                .is_some_and(|n| n == ".gitignore" || n == "exclude" || n == "index")
        });
        for path in &request.directories {
            let canonical = normalize(path);
            let affected = self.refresh_all
                || ignore_changed
                || !self.directory_cache.contains_key(path)
                || self
                    .dirty_paths
                    .iter()
                    .any(|p| p == &canonical || p.parent() == Some(canonical.as_path()));
            if affected {
                let entries = services::directory_result(path, in_git);
                self.directory_cache.insert(path.clone(), entries);
            }
            if let Some(entries) = self.directory_cache.get(path) {
                directories.push((path.clone(), entries.clone()));
            }
        }
        self.directory_cache
            .retain(|path, _| request.directories.contains(path));
        self.dirty_paths.clear();
        self.refresh_all = false;
        directories
    }

    fn retry_if_missing(&mut self, request: &Request) {
        let missing = crate::retry_budget::path_missing(&request.cwd);
        self.missing_path
            .record(&request.cwd, request.generation, missing);
        if missing
            && !self
                .missing_path
                .exhausted(&request.cwd, request.generation)
        {
            self.pending = Some(overdue());
        }
    }
}
