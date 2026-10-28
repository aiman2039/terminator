//! Production refresh worker: watcher-driven, cancellable, partial re-listing.
use super::{
    DirectoriesToListInput, Dirty, Request, Wake, directories_to_list, ignore_rules_changed,
    normalize, watch_targets,
};
use crate::{Update, gui_services::Services, services};
use notify::{RecursiveMode, Watcher};
use std::{
    collections::{HashMap, HashSet},
    ops::ControlFlow,
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};
use terminator_core::async_service::{CancellationToken, OperationContext, Policy};
use tokio::sync::watch;

type Watches = Vec<(PathBuf, RecursiveMode)>;
type SharedWatcher = Arc<Mutex<Option<notify::RecommendedWatcher>>>;

/// Selection and filesystem notifications are latest-value channels: no event backlog.
pub fn spawn_async(service: Services) -> watch::Sender<Option<Request>> {
    let (sender, requests) = watch::channel::<Option<Request>>(None);
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
        let worker = RefreshWorker::start(service, token, requests).await?;
        worker.run().await
    });
    sender
}

/// One refresh: its request, the dirty paths it covers, and whether it
/// re-lists every visible folder.
struct RefreshPass {
    request: Request,
    seen: Dirty,
    full: bool,
}

/// What one refresh produced before it is merged into the listings.
struct Loaded {
    targets: Watches,
    fallback: bool,
    context: services::ContextData,
    directories: Vec<services::Listing>,
}

struct RefreshWorker {
    service: Services,
    token: CancellationToken,
    requests: watch::Receiver<Option<Request>>,
    /// Bumped by the watcher callback for each recorded event.
    changed: watch::Receiver<u64>,
    dirty: Arc<Mutex<Dirty>>,
    watcher: SharedWatcher,
    watched: Watches,
    missing_path: crate::retry_budget::RetryBudget,
    /// Final listings (with ignored flags) of the visible directories,
    /// for the request they were listed under: a watch event re-lists
    /// only the folders it touched and reuses the rest.
    listed: HashMap<PathBuf, services::Listing>,
    listed_for: Option<Request>,
    wake: Wake,
}

impl RefreshWorker {
    async fn start(
        service: Services,
        token: CancellationToken,
        requests: watch::Receiver<Option<Request>>,
    ) -> anyhow::Result<Self> {
        let (events, changed) = watch::channel(0_u64);
        let dirty = Arc::new(Mutex::new(Dirty::default()));
        let recorder = Arc::clone(&dirty);
        let watcher = service
            .fs()
            .run(&token, move || {
                anyhow::Ok(
                    notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                        record_event(&recorder, &events, event);
                    })
                    .ok(),
                )
            })
            .await?;
        Ok(Self {
            service,
            token,
            requests,
            changed,
            dirty,
            watcher: Arc::new(Mutex::new(watcher)),
            watched: Vec::new(),
            missing_path: crate::retry_budget::RetryBudget::default(),
            listed: HashMap::new(),
            listed_for: None,
            wake: Wake::Request,
        })
    }

    async fn run(mut self) -> anyhow::Result<Vec<Update>> {
        loop {
            let request = self.requests.borrow_and_update().clone();
            let flow = match request {
                None => self.await_first_request().await,
                Some(request)
                    if self
                        .missing_path
                        .exhausted(&request.cwd, request.generation) =>
                {
                    self.await_request().await
                }
                Some(request) => self.refresh(request).await?,
            };
            if flow.is_break() {
                break;
            }
        }
        let watcher = self.watcher;
        self.service
            .fs()
            .run(&CancellationToken::new(), move || {
                drop(watcher);
                anyhow::Ok(())
            })
            .await?;
        Ok(Vec::new())
    }

    async fn await_first_request(&mut self) -> ControlFlow<()> {
        let flow = self.await_request().await;
        self.wake = Wake::Request;
        flow
    }

    async fn await_request(&mut self) -> ControlFlow<()> {
        tokio::select! {
            () = self.token.cancelled() => ControlFlow::Break(()),
            result = self.requests.changed() => {
                if result.is_err() {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            }
        }
    }

    async fn refresh(&mut self, request: Request) -> anyhow::Result<ControlFlow<()>> {
        let operation = self.token.child_token();
        let _guard = operation.clone().drop_guard();
        let seen = self
            .dirty
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let full = self.full_refresh(&request, &seen);
        let pass = RefreshPass {
            request,
            seen,
            full,
        };
        let relist = self.relist(&pass, &operation).await;
        let request = &pass.request;
        let work = load(LoadInput {
            service: &self.service,
            watcher: &self.watcher,
            watched: self.watched.clone(),
            request,
            relist,
            operation: &operation,
        });
        let result = tokio::select! {
            () = self.token.cancelled() => return Ok(ControlFlow::Break(())),
            result = self.requests.changed() => {
                if result.is_err() {
                    return Ok(ControlFlow::Break(()));
                }
                self.wake = Wake::Request;
                return Ok(ControlFlow::Continue(()));
            }
            result = work => result,
        };
        let fallback = match result {
            Ok(loaded) => self.publish(&pass, loaded).await?,
            Err(error) => {
                self.listed.clear();
                self.listed_for = None;
                self.service
                    .emit(Update::Error(format!("Refresh: {error:#}")))
                    .await?;
                true
            }
        };
        if self.record_missing(request, &operation).await {
            self.wake = Wake::Timer;
            return Ok(ControlFlow::Continue(()));
        }
        Ok(self.await_wake(fallback).await)
    }

    /// Whether this refresh re-lists every visible folder.
    fn full_refresh(&self, request: &Request, seen: &Dirty) -> bool {
        let same_scope = self.listed_for.as_ref().is_some_and(|last| {
            last.cwd == request.cwd && last.include_stats == request.include_stats
        });
        // A new generation with the same folders is a forced refresh.
        let forced = self.wake == Wake::Request
            && self
                .listed_for
                .as_ref()
                .is_some_and(|last| last.directories == request.directories);
        !same_scope
            || forced
            || self.wake == Wake::Timer
            || seen.full
            || ignore_rules_changed(&seen.paths)
    }

    async fn relist(&self, pass: &RefreshPass, operation: &CancellationToken) -> Vec<PathBuf> {
        let RefreshPass {
            request,
            seen,
            full,
        } = pass;
        let full = *full;
        let known: HashSet<PathBuf> = if full {
            HashSet::new()
        } else {
            self.listed.keys().cloned().collect()
        };
        let wanted = request.directories.clone();
        let changed_paths = seen.paths.clone();
        self.service
            .fs()
            .run(operation, move || {
                anyhow::Ok(directories_to_list(DirectoriesToListInput {
                    directories: &wanted,
                    listed: &|path| known.contains(path),
                    dirty: &changed_paths,
                    full,
                }))
            })
            .await
            .unwrap_or_else(|_| request.directories.clone())
    }

    /// Merges fresh listings and emits the refresh. Returns whether the
    /// watcher fell back to polling.
    async fn publish(&mut self, pass: &RefreshPass, loaded: Loaded) -> anyhow::Result<bool> {
        let RefreshPass {
            request,
            seen,
            full,
        } = pass;
        let Loaded {
            targets,
            fallback,
            context,
            directories: fresh,
        } = loaded;
        self.watched = targets;
        if *full {
            self.listed.clear();
        }
        for listing in fresh {
            self.listed.insert(listing.0.clone(), listing);
        }
        self.listed
            .retain(|path, _| request.directories.contains(path));
        let directories = request
            .directories
            .iter()
            .filter_map(|path| self.listed.get(path).cloned())
            .collect();
        self.service
            .emit_read(Update::Refresh(
                request.generation,
                context,
                directories,
                fallback,
            ))
            .await?;
        self.listed_for = Some(request.clone());
        self.dirty
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .settle(seen);
        Ok(fallback)
    }

    async fn record_missing(&mut self, request: &Request, operation: &CancellationToken) -> bool {
        let cwd = request.cwd.clone();
        let missing = self
            .service
            .fs()
            .run(operation, move || {
                anyhow::Ok(crate::retry_budget::path_missing(&cwd))
            })
            .await
            .unwrap_or(false);
        self.missing_path
            .record(&request.cwd, request.generation, missing);
        missing
    }

    /// A missed event cannot be told apart from none in fallback
    /// polling, so only a working watcher allows partial refreshes.
    async fn await_wake(&mut self, fallback: bool) -> ControlFlow<()> {
        let wake = tokio::select! {
            () = self.token.cancelled() => return ControlFlow::Break(()),
            result = self.requests.changed() => {
                if result.is_err() {
                    return ControlFlow::Break(());
                }
                Wake::Request
            }
            _ = self.changed.changed() => {
                // Fixed coalescing window cannot be extended by a continuous writer.
                tokio::select! {
                    () = self.token.cancelled() => return ControlFlow::Break(()),
                    () = tokio::time::sleep(Duration::from_millis(250)) => {}
                }
                self.changed.borrow_and_update();
                if fallback { Wake::Timer } else { Wake::Watch }
            }
            () = tokio::time::sleep(Duration::from_secs(if fallback { 3 } else { 30 })) => Wake::Timer,
        };
        self.wake = wake;
        ControlFlow::Continue(())
    }
}

fn record_event(
    dirty: &Mutex<Dirty>,
    events: &watch::Sender<u64>,
    event: notify::Result<notify::Event>,
) {
    if event
        .as_ref()
        .is_ok_and(|e| matches!(e.kind, notify::EventKind::Access(_)))
    {
        return;
    }
    let event = event.map(|mut event| {
        event.paths = event.paths.iter().map(|path| normalize(path)).collect();
        event
    });
    dirty
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .record(&event);
    events.send_modify(|n| *n = n.wrapping_add(1));
}

struct LoadInput<'a> {
    service: &'a Services,
    watcher: &'a SharedWatcher,
    watched: Watches,
    request: &'a Request,
    relist: Vec<PathBuf>,
    operation: &'a CancellationToken,
}

/// Shows the bounded directory entries before repository status, line
/// counts, ignore checks, or watcher setup can delay them.
async fn load(input: LoadInput<'_>) -> anyhow::Result<Loaded> {
    let LoadInput {
        service,
        watcher,
        watched,
        request,
        relist,
        operation,
    } = input;
    let mut raw_directories = Vec::new();
    for path in &relist {
        let entries = services::directory_raw_async(service, path.clone(), operation).await;
        service
            .emit_read(Update::DirectoryPreview(
                request.generation,
                request.cwd.clone(),
                path.clone(),
                entries.clone(),
            ))
            .await?;
        raw_directories.push((path.clone(), entries));
    }
    let context = services::context_async(services::ContextAsyncInput {
        service,
        cwd: request.cwd.clone(),
        include_stats: request.include_stats,
        cancel: operation,
    })
    .await;
    let targets = watch_targets(request, &context);
    let cache = Arc::clone(watcher);
    let (targets, fallback) = service
        .fs()
        .run(operation, move || {
            anyhow::Ok(rewatch(&cache, watched, targets))
        })
        .await?;
    let directories =
        services::directories_with_ignored_async(services::DirectoriesWithIgnoredAsyncInput {
            service,
            cwd: &request.cwd,
            listings: raw_directories,
            in_git: context.root.is_some() && context.perforce.is_none(),
        })
        .await;
    Ok(Loaded {
        targets,
        fallback,
        context,
        directories,
    })
}

/// Swaps watched paths when the targets changed. Returns the targets and
/// whether watching fell back to polling.
fn rewatch(watcher: &SharedWatcher, old: Watches, targets: Watches) -> (Watches, bool) {
    let mut watcher = watcher.lock().unwrap_or_else(PoisonError::into_inner);
    let mut fallback = watcher.is_none();
    if let Some(watcher) = watcher.as_mut()
        && old != targets
    {
        for (path, _) in old {
            let _ = watcher.unwatch(&path);
        }
        for (path, mode) in &targets {
            if watcher.watch(path, *mode).is_err() {
                fallback = true;
            }
        }
    }
    (targets, fallback)
}
