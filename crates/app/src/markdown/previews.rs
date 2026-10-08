use super::html::{Document, Link, prepare, usize_from_f32};
use super::source::{INTERVAL, MAX_DOCUMENT, Snapshot, Source, read_saved_cancel};
use anyhow::{Context, Result, ensure};
use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
#[cfg(test)]
use terminator_core::Paths;

pub(crate) struct Preview {
    pub(super) document: Option<Document>,
    pub(super) snapshot_source: Option<(PathBuf, String)>,
    pub(super) snapshot_generation: u64,
    pub(super) applied_generation: u64,
    pub(super) error: Option<String>,
    pub(super) cache: CommonMarkCache,
    pub editor_focused: bool,
    pub(super) preview_rect: egui::Rect,
}
impl Default for Preview {
    fn default() -> Self {
        Self {
            document: None,
            snapshot_source: None,
            snapshot_generation: 0,
            applied_generation: 0,
            error: None,
            cache: CommonMarkCache::default(),
            editor_focused: true,
            preview_rect: egui::Rect::NOTHING,
        }
    }
}
impl Preview {
    pub(super) fn apply(&mut self, result: Result<Document, String>) {
        match result {
            Ok(document) => {
                self.cache.clear_scrollable();
                self.cache.link_hooks_clear();
                for link in document.links.keys() {
                    self.cache.add_link_hook(link);
                }
                self.document = Some(document);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub fn pointer_focus(&mut self, ui: &egui::Ui) {
        if ui.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|p| self.preview_rect.contains(p))
        }) {
            self.editor_focused = false;
        }
    }
    pub fn show(&mut self, ui: &mut egui::Ui, sid: &str) -> Option<Link> {
        self.preview_rect = ui.max_rect();
        self.pointer_focus(ui);
        #[cfg(feature = "test-support")]
        crate::diagnostics::record(ui.ctx(), "markdown-preview", self.preview_rect);
        if let Some(error) = &self.error {
            ui.colored_label(
                ui.visuals().error_fg_color,
                format!("Preview could not refresh: {error}"),
            );
        }
        let Some(document) = &self.document else {
            if self.error.is_none() {
                ui.spinner();
            }
            return None;
        };
        let status = if self.error.is_some() {
            "Last available preview"
        } else {
            document.status
        };
        ui.weak(status);
        #[cfg(feature = "test-support")]
        {
            crate::diagnostics::record(
                ui.ctx(),
                &format!("markdown-status:{status}"),
                self.preview_rect,
            );
        }
        egui::ScrollArea::both()
            .id_salt(("markdown-scroll", sid))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                egui::Frame::NONE.inner_margin(12).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    CommonMarkViewer::new()
                        .explicit_image_uri_scheme(true)
                        .max_image_width(Some(usize_from_f32(ui.available_width().max(1.0))))
                        .enable_scroll_to_heading(true)
                        .show(ui, &mut self.cache, &document.text);
                });
            });
        document.links.iter().find_map(|(url, target)| {
            (self.cache.get_link_hook(url) == Some(true))
                .then(|| target.clone())
                .flatten()
        })
    }
}

#[derive(Clone)]
pub(super) struct Watch {
    generation: u64,
    sources: Vec<Source>,
    retained: HashSet<String>,
}
type Loaded = (u64, String, Result<Document, String>);
pub(crate) struct Previews {
    pub entries: HashMap<String, Preview>,
    pub(super) requests: tokio::sync::watch::Sender<Watch>,
    pub(super) results: tokio::sync::mpsc::Receiver<Loaded>,
    pub(super) outgoing: tokio::sync::mpsc::Sender<Loaded>,
    pub(super) services: crate::gui_services::Services,
    pub(super) next_document: u64,
    #[cfg(test)]
    pub(super) _owner: Option<crate::gui_services::Owner>,
    pub(super) visible: Vec<Source>,
    pub(super) watching: Vec<Source>,
    pub(super) retained: HashSet<String>,
    pub(super) generation: u64,
    pub(super) refresh: bool,
    pub(super) images: std::sync::Arc<crate::markdown_images::Images>,
}
impl Previews {
    #[cfg(test)]
    pub fn new(ctx: &egui::Context) -> Self {
        let paths = Paths::at(std::env::temp_dir().join(terminator_core::id()));
        let (tx, _) = std::sync::mpsc::channel();
        let (services, owner) = crate::gui_services::Services::new(paths, ctx.clone(), tx).unwrap();
        let mut previews = Self::with_services(ctx, services);
        previews._owner = Some(owner);
        previews
    }
    pub fn with_services(ctx: &egui::Context, services: crate::gui_services::Services) -> Self {
        use futures_util::{StreamExt, stream};
        use terminator_core::async_service::{CancellationToken, OperationContext, Policy};
        let (requests, mut incoming) = tokio::sync::watch::channel(Watch {
            generation: 0,
            sources: Vec::new(),
            retained: HashSet::new(),
        });
        let (outgoing, results) = tokio::sync::mpsc::channel(8);
        let loaded = outgoing.clone();
        let repaint = ctx.clone();
        let service = services.clone();
        let cancellation = CancellationToken::new();
        let token = cancellation.clone();
        let mut context = OperationContext::new(
            "markdown",
            "visible-editors".into(),
            Policy::ServiceLifetime,
        );
        context.deadline = None;
        let _ = services.handle().submit(context, cancellation, async move {
            let snapshots = Arc::new(Mutex::new(HashMap::<String, Arc<Snapshot>>::new()));
            let mut watched_generation = None;
            let failed = Arc::new(Mutex::new(HashSet::<String>::new()));
            loop {
                let watch = incoming.borrow_and_update().clone();
                snapshots.lock().unwrap_or_else(std::sync::PoisonError::into_inner).retain(|sid, _| watch.retained.contains(sid));
                let changed = watched_generation != Some(watch.generation);
                watched_generation = Some(watch.generation);
                failed.lock().unwrap_or_else(std::sync::PoisonError::into_inner).retain(|sid| watch.retained.contains(sid));
                let operation = token.child_token();
                let _guard = operation.clone().drop_guard();
                let work = async {
                    let mut jobs = stream::iter(watch.sources.iter().cloned().map(|source| {
                        let previous = snapshots.lock().unwrap_or_else(std::sync::PoisonError::into_inner).get(&source.session).cloned();
                        let service = service.clone(); let operation = operation.clone(); let snapshots = Arc::clone(&snapshots);
                        let failed = Arc::clone(&failed);
                        let force = changed || failed.lock().unwrap_or_else(std::sync::PoisonError::into_inner).contains(&source.session);
                        async move {
                            let result = match read_source_async(&service, &source, previous.clone(), &operation, force).await {
                                Ok(None) => return None,
                                Ok(Some(snapshot)) if !force && previous.as_deref() == Some(&snapshot) => return None,
                                Ok(Some(snapshot)) => {
                                    let retained = Arc::new(snapshot.clone());
                                    let result = service.cpu().run(&operation, move || Ok(prepare(snapshot))).await.map_err(|e| format!("{e:#}"));
                                    if result.is_ok() { snapshots.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(source.session.clone(), retained); failed.lock().unwrap_or_else(std::sync::PoisonError::into_inner).remove(&source.session); }
                                    else { failed.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(source.session.clone()); }
                                    result
                                }
                                Err(error) => {
                                    failed.lock().unwrap_or_else(std::sync::PoisonError::into_inner).insert(source.session.clone());
                                    Err(format!("{error:#}"))
                                }
                            };
                            Some((watch.generation, source.session, result))
                        }
                    })).buffer_unordered(4);
                    while let Some(result) = jobs.next().await {
                        if let Some(result) = result {
                            if loaded.send(result).await.is_err() { break; }
                            repaint.request_repaint();
                        }
                    }
                };
                tokio::select! {
                    () = token.cancelled() => break,
                    result = incoming.changed() => { if result.is_err() { break; } continue; }
                    () = work => {},
                }
                tokio::select! { () = token.cancelled() => break, result = incoming.changed() => if result.is_err() { break }, () = tokio::time::sleep(INTERVAL) => {} }
            }
            Ok(Vec::new())
        });
        let images = crate::markdown_images::Images::with_submit(ctx, services.submit());
        Self {
            entries: HashMap::new(),
            requests,
            results,
            outgoing,
            services,
            next_document: 1 << 63,
            visible: Vec::new(),
            watching: Vec::new(),
            retained: HashSet::new(),
            generation: 0,
            refresh: false,
            images,
            #[cfg(test)]
            _owner: None,
        }
    }
    pub fn begin_frame(&mut self) {
        self.visible.clear();
        self.retained.clear();
        self.process_results();
    }
    fn process_results(&mut self) {
        while let Ok((generation, sid, result)) = self.results.try_recv() {
            if let Some(preview) = self.entries.get_mut(&sid)
                && (generation == self.generation || generation == preview.snapshot_generation)
            {
                preview.apply(result);
                preview.applied_generation = generation;
            }
        }
    }
    #[cfg(test)]
    pub fn wait_prepared(&mut self) {
        let deadline = std::time::Instant::now()
            .checked_add(Duration::from_secs(3))
            .unwrap_or_else(std::time::Instant::now);
        loop {
            self.process_results();
            if self.entries.values().all(|p| {
                p.snapshot_source.is_none() || p.applied_generation == p.snapshot_generation
            }) {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Markdown preparation did not complete"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    pub fn retain(&mut self, sid: &str) -> &mut Preview {
        self.retained.insert(sid.into());
        self.entries.entry(sid.into()).or_default()
    }
    /// Render immutable Git content through the same resolver and bounded image loader.
    pub fn snapshot(&mut self, key: &str, path: &Path, text: &str) -> &mut Preview {
        self.retained.insert(key.into());
        let preview = self.entries.entry(key.into()).or_default();
        if !preview
            .snapshot_source
            .as_ref()
            .is_some_and(|(old_path, old_text)| old_path == path && old_text == text)
        {
            use terminator_core::async_service::{CancellationToken, OperationContext, Policy};
            self.next_document = self.next_document.wrapping_add(1);
            let generation = self.next_document;
            let snapshot = Snapshot {
                path: path.to_owned(),
                text: text.to_owned(),
                revision: None,
                paused: false,
            };
            let service = self.services.clone();
            let outgoing = self.outgoing.clone();
            let id = key.to_owned();
            let context =
                OperationContext::new("markdown-document", id.clone(), Policy::ReplaceableRead);
            let cancel = CancellationToken::new();
            let token = cancel.clone();
            if self
                .services
                .handle()
                .submit(context, cancel, async move {
                    let result = service
                        .cpu()
                        .run(&token, move || Ok(prepare(snapshot)))
                        .await
                        .map_err(|e| format!("{e:#}"));
                    let _ = outgoing.send((generation, id, result)).await;
                    Ok(Vec::new())
                })
                .is_ok()
            {
                preview.snapshot_source = Some((path.to_owned(), text.to_owned()));
                preview.snapshot_generation = generation;
            }
        }
        preview
    }
    pub fn watch(&mut self, source: Source) {
        self.visible.push(source);
    }
    pub fn refresh(&mut self, ctx: &egui::Context) {
        self.refresh = true;
        self.images.clear(ctx);
    }
    pub fn end_frame(&mut self, ctx: &egui::Context) {
        let previous_count = self.entries.len();
        self.entries.retain(|sid, _| self.retained.contains(sid));
        if self.visible != self.watching || self.refresh || previous_count != self.entries.len() {
            self.generation = self.generation.wrapping_add(1);
            self.watching.clone_from(&self.visible);
            let _ = self.requests.send(Watch {
                generation: self.generation,
                sources: self.visible.clone(),
                retained: self.retained.clone(),
            });
            self.refresh = false;
        }
        let used = self
            .entries
            .values()
            .filter_map(|p| p.document.as_ref())
            .flat_map(|d| d.images.iter().cloned())
            .collect();
        self.images.retain(ctx, &used);
    }

    #[cfg(feature = "test-support")]
    pub fn diagnostics(&self) -> serde_json::Value {
        self.entries.iter().map(|(sid, preview)| {
            (sid.clone(), serde_json::json!({
                "visible":self.watching.iter().any(|s| &s.session == sid),
                "text":preview.document.as_ref().map(|d| &d.text),
                "status":preview.document.as_ref().map(|d| d.status),
                "error":preview.error,
                "editor_focused":preview.editor_focused,
                "rect":[preview.preview_rect.min.x,preview.preview_rect.min.y,preview.preview_rect.width(),preview.preview_rect.height()]
            }))
        }).collect::<serde_json::Map<_, _>>().into()
    }
}

async fn read_source_async(
    service: &crate::gui_services::Services,
    source: &Source,
    previous: Option<Arc<Snapshot>>,
    cancel: &terminator_core::async_service::CancellationToken,
    force: bool,
) -> Result<Option<Snapshot>> {
    let socket = source.socket.clone();
    // Windows Neovim listens on a named pipe with no filesystem record, so
    // the existence probe is Unix-only; on Windows the connect attempt below
    // decides, falling back to saved content on timeout.
    #[cfg(unix)]
    let live = service
        .fs()
        .run(cancel, move || Ok(socket.exists()))
        .await?;
    #[cfg(windows)]
    let live = {
        let _ = socket;
        true
    };
    if live {
        let response = (async {
            let mut rpc = crate::nvim_rpc::AsyncConnection::connect(
                &source.socket,
                Duration::from_millis(400),
                service.cpu().clone(),
            )
            .await?;
            // Fast requests still work at swap-file, hit-enter and input() prompts.
            // Ordinary evaluation would be deferred until the user answers them.
            let mode = rpc
                .call("nvim_get_mode", serde_json::json!([]), 4096)
                .await?;
            if mode
                .get("blocking")
                .and_then(serde_json::Value::as_bool)
                .context("Invalid Neovim mode")?
            {
                return Ok(None);
            }
            let revision = previous
                .as_deref()
                .filter(|s| !force && !s.paused)
                .and_then(|s| s.revision.as_ref());
            let arguments =
                serde_json::json!({"path":source.path, "previous":revision, "limit":MAX_DOCUMENT});
            let code = format!(
                "local _A = ...\n{}",
                include_str!("../markdown_snapshot.lua")
            );
            Ok(Some(
                rpc.call(
                    "nvim_exec_lua",
                    serde_json::json!([code, [arguments]]),
                    MAX_DOCUMENT * 6 + 16384,
                )
                .await?,
            ))
        })
        .await;
        let value = match response {
            Ok(Some(value)) => value,
            Ok(None) => {
                return paused_async(service, source.clone(), previous.clone(), cancel)
                    .await
                    .map(Some);
            }
            Err(error) if crate::nvim_rpc::timed_out(&error) => {
                return paused_async(service, source.clone(), previous.clone(), cancel)
                    .await
                    .map(Some);
            }
            Err(error) => return Err(error.context("Could not read the live editor buffer")),
        };
        let value: serde_json::Value =
            serde_json::from_str(value.as_str().context("Invalid editor preview response")?)?;
        if let Some(error) = value.get("error").and_then(serde_json::Value::as_str) {
            anyhow::bail!("{error}");
        }
        if value.get("unchanged").and_then(serde_json::Value::as_bool) == Some(true) {
            return Ok(None);
        }
        let snapshot: Snapshot = serde_json::from_value(value)?;
        ensure!(
            snapshot.text.len() <= MAX_DOCUMENT,
            "Markdown preview is limited to 1 MiB"
        );
        return Ok(Some(snapshot));
    }
    if previous.as_deref().is_some_and(|s| s.revision.is_some()) {
        return paused_async(service, source.clone(), previous, cancel)
            .await
            .map(Some);
    }
    let source = source.clone();
    let operation = cancel.clone();
    service
        .fs()
        .run(cancel, move || read_saved_cancel(&source, &operation))
        .await
        .map(Some)
}
async fn paused_async(
    service: &crate::gui_services::Services,
    source: Source,
    previous: Option<Arc<Snapshot>>,
    cancel: &terminator_core::async_service::CancellationToken,
) -> Result<Snapshot> {
    let operation = cancel.clone();
    service
        .fs()
        .run(cancel, move || {
            let mut snapshot = match previous.as_deref().filter(|s| s.revision.is_some()) {
                Some(snapshot) => snapshot.clone(),
                None => read_saved_cancel(&source, &operation)?,
            };
            snapshot.paused = true;
            Ok(snapshot)
        })
        .await
}
