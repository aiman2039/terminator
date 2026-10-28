use super::html::{Document, Link, prepare};
use super::source::{Snapshot, Source};
use anyhow::Result;
use eframe::egui;
use egui_commonmark::CommonMarkCache;
#[cfg(test)]
use std::time::Duration;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

mod loader;
mod read;
mod render;

pub(crate) struct ShowSyncedInput<'a> {
    pub ui: &'a mut egui::Ui,
    pub sid: &'a str,
    pub status_override: Option<&'a str>,
    pub scroll_fraction: Option<f32>,
}

#[cfg(test)]
use terminator_core::Paths;

pub(crate) struct Preview {
    pub(super) document: Option<Document>,
    pub(super) snapshot_source: Option<(PathBuf, String)>,
    pub(super) snapshot_generation: u64,
    pub(super) applied_generation: u64,
    pub(super) error: Option<String>,
    pub(super) cache: CommonMarkCache,
    pub(super) mermaid_textures: Vec<egui::TextureHandle>,
    pub editor_focused: bool,
    pub(super) preview_rect: egui::Rect,
    pub scroll_fraction: f32,
    pub(super) cache_scroll_height: f32,
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
            mermaid_textures: Vec::new(),
            editor_focused: true,
            preview_rect: egui::Rect::NOTHING,
            scroll_fraction: 0.0,
            cache_scroll_height: 0.0,
        }
    }
}
impl Preview {
    pub(super) fn apply(&mut self, result: Result<Document, String>) {
        match result {
            Ok(document) => {
                self.cache.clear_viewers();
                self.cache.link_hooks_clear();
                self.mermaid_textures.clear();
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
    pub fn contains_pointer(&self, ui: &egui::Ui) -> bool {
        ui.input(|input| {
            input
                .pointer
                .hover_pos()
                .is_some_and(|point| self.preview_rect.contains(point))
        })
    }
    pub fn show(&mut self, ui: &mut egui::Ui, sid: &str) -> Option<Link> {
        self.show_with_status(ui, sid, None)
    }
    pub fn show_with_status(
        &mut self,
        ui: &mut egui::Ui,
        sid: &str,
        status_override: Option<&str>,
    ) -> Option<Link> {
        self.show_synced(ShowSyncedInput {
            ui,
            sid,
            status_override,
            scroll_fraction: None,
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
        use terminator_core::async_service::{CancellationToken, OperationContext, Policy};
        let (requests, incoming) = tokio::sync::watch::channel(Watch {
            generation: 0,
            sources: Vec::new(),
            retained: HashSet::new(),
        });
        let (outgoing, results) = tokio::sync::mpsc::channel(8);
        let cancellation = CancellationToken::new();
        let watcher = loader::WatchLoop {
            incoming,
            loaded: outgoing.clone(),
            repaint: ctx.clone(),
            service: services.clone(),
            token: cancellation.clone(),
        };
        let mut context = OperationContext::new(
            "markdown",
            "visible-editors".into(),
            Policy::ServiceLifetime,
        );
        context.deadline = None;
        let _ = services.handle().submit(context, cancellation, async move {
            watcher.run().await;
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
                        .run(&token, move || anyhow::Ok(prepare(snapshot)))
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
