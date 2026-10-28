use super::super::html::prepare;
use super::super::source::{INTERVAL, Snapshot, Source};
use super::read::{ReadSourceAsyncInput, read_source_async};
use super::{Loaded, Watch};
use eframe::egui;
use futures_util::{StreamExt, stream};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, PoisonError},
};
use terminator_core::async_service::CancellationToken;

type Snapshots = Arc<Mutex<HashMap<String, Arc<Snapshot>>>>;
type Failed = Arc<Mutex<HashSet<String>>>;

/// The service-lifetime task that re-reads visible Markdown sources every
/// interval (or when the watch set changes) and sends prepared documents.
pub(super) struct WatchLoop {
    pub(super) incoming: tokio::sync::watch::Receiver<Watch>,
    pub(super) loaded: tokio::sync::mpsc::Sender<Loaded>,
    pub(super) repaint: egui::Context,
    pub(super) service: crate::gui_services::Services,
    pub(super) token: CancellationToken,
}

impl WatchLoop {
    pub(super) async fn run(self) {
        let WatchLoop {
            mut incoming,
            loaded,
            repaint,
            service,
            token,
        } = self;
        let loader = Loader {
            loaded,
            repaint,
            service,
        };
        let snapshots: Snapshots = Arc::new(Mutex::new(HashMap::new()));
        let mut watched_generation = None;
        let failed: Failed = Arc::new(Mutex::new(HashSet::new()));
        loop {
            let watch = incoming.borrow_and_update().clone();
            snapshots
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .retain(|sid, _| watch.retained.contains(sid));
            let changed = watched_generation != Some(watch.generation);
            watched_generation = Some(watch.generation);
            failed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .retain(|sid| watch.retained.contains(sid));
            let operation = token.child_token();
            let _guard = operation.clone().drop_guard();
            let work = loader.load_all(LoadAll {
                watch: &watch,
                changed,
                operation: &operation,
                snapshots: &snapshots,
                failed: &failed,
            });
            tokio::select! {
                () = token.cancelled() => break,
                result = incoming.changed() => { if result.is_err() { break; } continue; }
                () = work => {},
            }
            tokio::select! { () = token.cancelled() => break, result = incoming.changed() => if result.is_err() { break }, () = tokio::time::sleep(INTERVAL) => {} }
        }
    }
}

/// Sends prepared documents back to the GUI.
struct Loader {
    loaded: tokio::sync::mpsc::Sender<Loaded>,
    repaint: egui::Context,
    service: crate::gui_services::Services,
}

impl Loader {
    /// Read every watched source, four at a time, sending each result.
    async fn load_all(&self, input: LoadAll<'_>) {
        let LoadAll {
            watch,
            changed,
            operation,
            snapshots,
            failed,
        } = input;
        let mut jobs = stream::iter(watch.sources.iter().cloned().map(|source| {
            let previous = snapshots
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&source.session)
                .cloned();
            let force = changed
                || failed
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .contains(&source.session);
            SourceJob {
                service: self.service.clone(),
                operation: operation.clone(),
                snapshots: Arc::clone(snapshots),
                failed: Arc::clone(failed),
                generation: watch.generation,
                source,
                previous,
                force,
            }
            .run()
        }))
        .buffer_unordered(4);
        while let Some(result) = jobs.next().await {
            if let Some(result) = result {
                if self.loaded.send(result).await.is_err() {
                    break;
                }
                self.repaint.request_repaint();
            }
        }
    }
}

struct LoadAll<'a> {
    watch: &'a Watch,
    changed: bool,
    operation: &'a CancellationToken,
    snapshots: &'a Snapshots,
    failed: &'a Failed,
}

/// Read and prepare one source; `None` when nothing changed.
struct SourceJob {
    service: crate::gui_services::Services,
    operation: CancellationToken,
    snapshots: Snapshots,
    failed: Failed,
    generation: u64,
    source: Source,
    previous: Option<Arc<Snapshot>>,
    force: bool,
}

impl SourceJob {
    async fn run(self) -> Option<Loaded> {
        let read = read_source_async(ReadSourceAsyncInput {
            service: &self.service,
            source: &self.source,
            previous: self.previous.clone(),
            cancel: &self.operation,
            force: self.force,
        })
        .await;
        let result = match read {
            Ok(None) => return None,
            Ok(Some(snapshot)) if !self.force && self.previous.as_deref() == Some(&snapshot) => {
                return None;
            }
            Ok(Some(snapshot)) => self.prepare(snapshot).await,
            Err(error) => {
                self.mark_failed();
                Err(format!("{error:#}"))
            }
        };
        Some((self.generation, self.source.session, result))
    }

    async fn prepare(&self, snapshot: Snapshot) -> Result<super::Document, String> {
        let retained = Arc::new(snapshot.clone());
        let result = self
            .service
            .cpu()
            .run(&self.operation, move || anyhow::Ok(prepare(snapshot)))
            .await
            .map_err(|e| format!("{e:#}"));
        if result.is_ok() {
            self.snapshots
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(self.source.session.clone(), retained);
            self.failed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&self.source.session);
        } else {
            self.mark_failed();
        }
        result
    }

    fn mark_failed(&self) {
        self.failed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(self.source.session.clone());
    }
}
