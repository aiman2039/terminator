//! Native Markdown presentation over the existing daemon-owned editor session.
use anyhow::{Context, Result, ensure};
use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use terminator_core::{Paths, Session, SessionKind};
use url::Url;

pub const MAX_DOCUMENT: usize = 1024 * 1024;
const INTERVAL: Duration = Duration::from_millis(350);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Edit,
    #[default]
    Preview,
    Split,
}
impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Edit => "Edit",
            Self::Preview => "Preview",
            Self::Split => "Split",
        }
    }
}
pub fn supported(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "md" | "markdown" | "mdown" | "mkd" | "mkdn"
        )
    })
}
pub fn available(session: &Session) -> bool {
    session.kind == SessionKind::Editor
        && !session.review
        && session.lifecycle.live()
        && session.file.as_deref().is_some_and(supported)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    session: String,
    path: PathBuf,
    socket: PathBuf,
}
impl Source {
    pub fn new(paths: &Paths, session: &Session) -> Self {
        Self {
            session: session.id.clone(),
            path: session.file.clone().expect("Markdown editor file"),
            socket: paths.editor_socket(&session.id),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
struct Revision {
    buffer: u64,
    tick: u64,
    modified: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
struct Snapshot {
    path: PathBuf,
    text: String,
    revision: Option<Revision>,
    #[serde(default)]
    paused: bool,
}
#[cfg(test)]
fn read_source(source: &Source, previous: Option<&Snapshot>) -> Result<Option<Snapshot>> {
    if source.socket.exists() {
        let response = (|| -> Result<Option<serde_json::Value>> {
            let mut rpc =
                crate::nvim_rpc::Connection::connect(&source.socket, Duration::from_millis(400))?;
            // Fast requests still work at swap-file, hit-enter and input() prompts.
            // Ordinary evaluation would be deferred until the user answers them.
            let mode = rpc.call("nvim_get_mode", serde_json::json!([]), 4096)?;
            if mode["blocking"].as_bool().context("Invalid Neovim mode")? {
                return Ok(None);
            }
            let revision = previous
                .filter(|s| !s.paused)
                .and_then(|s| s.revision.as_ref());
            let arguments =
                serde_json::json!({"path":source.path, "previous":revision, "limit":MAX_DOCUMENT});
            let code = format!("local _A = ...\n{}", include_str!("markdown_snapshot.lua"));
            Ok(Some(rpc.call(
                "nvim_exec_lua",
                serde_json::json!([code, [arguments]]),
                MAX_DOCUMENT * 6 + 16384,
            )?))
        })();
        let value = match response {
            Ok(Some(value)) => value,
            Ok(None) => return paused_snapshot(source, previous).map(Some),
            Err(error) if crate::nvim_rpc::timed_out(&error) => {
                return paused_snapshot(source, previous).map(Some);
            }
            Err(error) => return Err(error.context("Could not read the live editor buffer")),
        };
        let value: serde_json::Value =
            serde_json::from_str(value.as_str().context("Invalid editor preview response")?)?;
        if let Some(error) = value["error"].as_str() {
            anyhow::bail!("{error}");
        }
        if value["unchanged"].as_bool() == Some(true) {
            return Ok(None);
        }
        let snapshot: Snapshot = serde_json::from_value(value)?;
        ensure!(
            snapshot.text.len() <= MAX_DOCUMENT,
            "Markdown preview is limited to 1 MiB"
        );
        return Ok(Some(snapshot));
    }
    read_saved(source).map(Some)
}
#[cfg(test)]
fn paused_snapshot(source: &Source, previous: Option<&Snapshot>) -> Result<Snapshot> {
    // Never replace an observed unsaved buffer with older disk contents.
    let mut snapshot = match previous.filter(|s| s.revision.is_some()) {
        Some(snapshot) => snapshot.clone(),
        None => read_saved(source)?,
    };
    snapshot.paused = true;
    Ok(snapshot)
}
#[cfg(test)]
fn read_saved(source: &Source) -> Result<Snapshot> {
    read_saved_cancel(
        source,
        &terminator_core::async_service::CancellationToken::new(),
    )
}
fn read_saved_cancel(
    source: &Source,
    cancel: &terminator_core::async_service::CancellationToken,
) -> Result<Snapshot> {
    // Custom terminal editors have no Neovim RPC. Clearly label their saved-file view.
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&source.path)
        .context("Open Markdown file")?;
    ensure!(
        file.metadata()?.is_file(),
        "Markdown preview requires a regular file"
    );
    let bytes = terminator_core::async_service::read_chunks(file, MAX_DOCUMENT + 1, cancel)?;
    ensure!(
        bytes.len() <= MAX_DOCUMENT,
        "Markdown preview is limited to 1 MiB"
    );
    Ok(Snapshot {
        path: source.path.clone(),
        text: String::from_utf8(bytes).context("Markdown file is not UTF-8")?,
        revision: None,
        paused: false,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    File(PathBuf),
    Web(String),
}
fn resolve_link(path: &Path, target: &str) -> Option<Link> {
    if target.starts_with('#') {
        return None;
    }
    let base = Url::from_file_path(path).ok()?;
    let url = base.join(target).ok()?;
    match url.scheme() {
        "file" => url.to_file_path().ok().map(Link::File),
        "https" | "http" => Some(Link::Web(url.into())),
        _ => None,
    }
}
struct Document {
    text: String,
    status: &'static str,
    links: HashMap<String, Option<Link>>,
    images: HashSet<String>,
}
fn prepare(snapshot: Snapshot) -> Document {
    let mut links = HashMap::new();
    let mut images = HashSet::new();
    let mut replacements = Vec::new();
    let mut events = Parser::new_ext(&snapshot.text, Options::all()).into_offset_iter();
    while let Some((event, range)) = events.next() {
        match event {
            Event::Start(Tag::Link { dest_url, .. }) if !dest_url.starts_with('#') => {
                links.insert(
                    dest_url.to_string(),
                    resolve_link(&snapshot.path, &dest_url),
                );
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                let mut alt = String::new();
                for (event, _) in events.by_ref() {
                    match event {
                        Event::End(TagEnd::Image) => break,
                        Event::Text(text) | Event::Code(text) => alt.push_str(&text),
                        _ => {}
                    }
                }
                let alt = alt.replace(['[', ']'], "");
                let replacement = match resolve_link(&snapshot.path, &dest_url) {
                    Some(Link::File(path)) => match Url::from_file_path(path) {
                        Ok(url) => {
                            let uri = format!("markdown-image:{url}");
                            images.insert(uri.clone());
                            format!("![{alt}](<{uri}>)")
                        }
                        Err(_) => format!("Image: {alt}"),
                    },
                    _ => format!("Image: {alt}"),
                };
                replacements.push((range, replacement));
            }
            _ => {}
        }
    }
    let mut text = snapshot.text;
    for (range, replacement) in replacements.into_iter().rev() {
        text.replace_range(range, &replacement);
    }
    Document {
        text,
        status: match snapshot.revision {
            Some(revision) if snapshot.paused && revision.modified => {
                "Unsaved preview · live updates paused"
            }
            Some(_) if snapshot.paused => "Last live preview · live updates paused",
            None if snapshot.paused => "Saved file · live preview paused",
            Some(revision) if revision.modified => "Unsaved changes",
            Some(_) => "Live preview",
            None => "Saved file",
        },
        links,
        images,
    }
}

pub struct Preview {
    document: Option<Document>,
    snapshot_source: Option<(PathBuf, String)>,
    snapshot_generation: u64,
    applied_generation: u64,
    error: Option<String>,
    cache: CommonMarkCache,
    pub editor_focused: bool,
    preview_rect: egui::Rect,
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
    fn apply(&mut self, result: Result<Document, String>) {
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
                        .max_image_width(Some(ui.available_width().max(1.0) as usize))
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
struct Watch {
    generation: u64,
    sources: Vec<Source>,
    retained: HashSet<String>,
}
type Loaded = (u64, String, Result<Document, String>);
pub struct Previews {
    pub entries: HashMap<String, Preview>,
    requests: tokio::sync::watch::Sender<Watch>,
    results: tokio::sync::mpsc::Receiver<Loaded>,
    outgoing: tokio::sync::mpsc::Sender<Loaded>,
    services: crate::gui_services::Services,
    next_document: u64,
    #[cfg(test)]
    _owner: Option<crate::gui_services::Owner>,
    visible: Vec<Source>,
    watching: Vec<Source>,
    retained: HashSet<String>,
    generation: u64,
    refresh: bool,
    images: std::sync::Arc<crate::markdown_images::Images>,
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
                snapshots.lock().unwrap().retain(|sid, _| watch.retained.contains(sid));
                let changed = watched_generation != Some(watch.generation);
                watched_generation = Some(watch.generation);
                failed.lock().unwrap().retain(|sid| watch.retained.contains(sid));
                let operation = token.child_token();
                let _guard = operation.clone().drop_guard();
                let work = async {
                    let mut jobs = stream::iter(watch.sources.iter().cloned().map(|source| {
                        let previous = snapshots.lock().unwrap().get(&source.session).cloned();
                        let service = service.clone(); let operation = operation.clone(); let snapshots = snapshots.clone();
                        let failed = failed.clone();
                        let force = changed || failed.lock().unwrap().contains(&source.session);
                        async move {
                            let result = match read_source_async(&service, &source, previous.clone(), &operation, force).await {
                                Ok(None) => return None,
                                Ok(Some(snapshot)) if !force && previous.as_deref() == Some(&snapshot) => return None,
                                Ok(Some(snapshot)) => {
                                    let retained = Arc::new(snapshot.clone());
                                    let result = service.cpu().run(&operation, move || Ok(prepare(snapshot))).await.map_err(|e| format!("{e:#}"));
                                    if result.is_ok() { snapshots.lock().unwrap().insert(source.session.clone(), retained); failed.lock().unwrap().remove(&source.session); }
                                    else { failed.lock().unwrap().insert(source.session.clone()); }
                                    result
                                }
                                Err(error) => {
                                    failed.lock().unwrap().insert(source.session.clone());
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
                    _ = token.cancelled() => break,
                    result = incoming.changed() => { if result.is_err() { break; } continue; }
                    _ = work => {},
                }
                tokio::select! { _ = token.cancelled() => break, result = incoming.changed() => if result.is_err() { break }, _ = tokio::time::sleep(INTERVAL) => {} }
            }
            Ok(Vec::new())
        });
        let images = crate::markdown_images::Images::with_services(ctx, services.clone());
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
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
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
    if service
        .fs()
        .run(cancel, move || Ok(socket.exists()))
        .await?
    {
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
            if mode["blocking"].as_bool().context("Invalid Neovim mode")? {
                return Ok(None);
            }
            let revision = previous
                .as_deref()
                .filter(|s| !force && !s.paused)
                .and_then(|s| s.revision.as_ref());
            let arguments =
                serde_json::json!({"path":source.path, "previous":revision, "limit":MAX_DOCUMENT});
            let code = format!("local _A = ...\n{}", include_str!("markdown_snapshot.lua"));
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
        if let Some(error) = value["error"].as_str() {
            anyhow::bail!("{error}");
        }
        if value["unchanged"].as_bool() == Some(true) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn snapshot(text: &str) -> Snapshot {
        Snapshot {
            path: "/docs/read me.md".into(),
            text: text.into(),
            revision: None,
            paused: false,
        }
    }

    #[test]
    fn diff_snapshots_resolve_links_refresh_and_release_resources() {
        let ctx = egui::Context::default();
        let mut previews = Previews::new(&ctx);
        let path = Path::new("/repo/docs/readme.md");
        previews.begin_frame();
        previews.snapshot(
            "diff:left",
            path,
            "[Next](../next.md) ![pic](images/a.png) [bad](command:run)",
        );
        previews.wait_prepared();
        let preview = &previews.entries["diff:left"];
        let doc = preview.document.as_ref().unwrap();
        assert_eq!(
            doc.links["../next.md"],
            Some(Link::File("/repo/next.md".into()))
        );
        assert_eq!(doc.links["command:run"], None);
        assert!(
            doc.images
                .contains("markdown-image:file:///repo/docs/images/a.png")
        );
        assert_eq!(preview.cache.get_link_hook("../next.md"), Some(false));
        previews.snapshot("diff:right", path, "# Right snapshot");
        previews.end_frame(&ctx);
        assert_eq!(previews.entries.len(), 2);
        previews.begin_frame();
        previews.snapshot("diff:left", path, "[New](new.md)");
        previews.wait_prepared();
        let preview = &previews.entries["diff:left"];
        assert!(
            !preview
                .document
                .as_ref()
                .unwrap()
                .links
                .contains_key("../next.md")
        );
        assert_eq!(preview.cache.get_link_hook("../next.md"), None);
        assert_eq!(preview.cache.get_link_hook("new.md"), Some(false));
        previews.end_frame(&ctx);
        assert_eq!(previews.entries.len(), 1);
        previews.begin_frame();
        previews.end_frame(&ctx);
        assert!(previews.entries.is_empty());
    }

    #[test]
    fn blocking_editor_uses_saved_file_and_preserves_the_last_unsaved_preview() {
        use std::{io::Write, os::unix::net::UnixListener};
        let dir = tempfile::tempdir().unwrap();
        let source = Source {
            session: "editor".into(),
            path: dir.path().join("README.md"),
            socket: dir.path().join("nvim.sock"),
        };
        std::fs::write(&source.path, "# Saved file").unwrap();
        let listener = UnixListener::bind(&source.socket).unwrap();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut peer, _) = listener.accept().unwrap();
                let request: serde_json::Value = rmp_serde::from_read(&mut peer).unwrap();
                assert_eq!(request[2], "nvim_get_mode");
                peer.write_all(
                    &rmp_serde::to_vec(&(
                        1,
                        1,
                        serde_json::Value::Null,
                        serde_json::json!({"mode":"rm","blocking":true}),
                    ))
                    .unwrap(),
                )
                .unwrap();
                // The client must close without queuing an evaluation behind the prompt.
                let mut byte = [0];
                assert_eq!(peer.read(&mut byte).unwrap(), 0);
            }
        });
        let saved = read_source(&source, None).unwrap().unwrap();
        assert_eq!(saved.text, "# Saved file");
        assert_eq!(prepare(saved).status, "Saved file · live preview paused");
        let mut dirty = snapshot("# Unsaved edits");
        dirty.revision = Some(Revision {
            buffer: 1,
            tick: 2,
            modified: true,
        });
        let preserved = read_source(&source, Some(&dirty)).unwrap().unwrap();
        assert_eq!(preserved.text, "# Unsaved edits");
        assert_eq!(
            prepare(preserved).status,
            "Unsaved preview · live updates paused"
        );
        assert_eq!(
            std::fs::read_to_string(&source.path).unwrap(),
            "# Saved file"
        );
        server.join().unwrap();
    }

    #[test]
    fn markdown_defaults_to_preview_and_explicit_edit_mode_is_persisted() {
        let mut preferences = crate::preferences::UiPreferences::default();
        assert_eq!(
            preferences
                .markdown_modes
                .get("new")
                .copied()
                .unwrap_or_default(),
            Mode::Preview
        );
        preferences
            .markdown_modes
            .insert("editor".into(), Mode::Edit);
        let restored: crate::preferences::UiPreferences =
            serde_json::from_str(&serde_json::to_string(&preferences).unwrap()).unwrap();
        assert_eq!(restored.markdown_modes["editor"], Mode::Edit);
    }

    #[test]
    fn refresh_during_a_prompt_keeps_the_cached_unsaved_document() {
        use std::{io::Write, os::unix::net::UnixListener, time::Instant};
        let dir = tempfile::tempdir().unwrap();
        let source = Source {
            session: "editor".into(),
            path: dir.path().join("README.md"),
            socket: dir.path().join("nvim.sock"),
        };
        std::fs::write(&source.path, "# Saved").unwrap();
        let listener = UnixListener::bind(&source.socket).unwrap();
        let path = source.path.clone();
        let server = std::thread::spawn(move || {
            for blocked in [false, true] {
                let (mut peer, _) = listener.accept().unwrap();
                peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let request: serde_json::Value = rmp_serde::from_read(&mut peer).unwrap();
                assert_eq!(request[2], "nvim_get_mode");
                peer.write_all(
                    &rmp_serde::to_vec(&(
                        1,
                        1,
                        serde_json::Value::Null,
                        serde_json::json!({"mode":"n","blocking":blocked}),
                    ))
                    .unwrap(),
                )
                .unwrap();
                if !blocked {
                    let request: serde_json::Value = rmp_serde::from_read(&mut peer).unwrap();
                    assert_eq!(request[2], "nvim_exec_lua");
                    let response = serde_json::json!({"path":path,"text":"# Unsaved","revision":{"buffer":1,"tick":2,"modified":true}}).to_string();
                    peer.write_all(
                        &rmp_serde::to_vec(&(1, 2, serde_json::Value::Null, response)).unwrap(),
                    )
                    .unwrap();
                }
            }
        });
        let ctx = egui::Context::default();
        let mut previews = Previews::new(&ctx);
        for paused in [false, true] {
            if paused {
                previews.refresh(&ctx);
            }
            let started = Instant::now();
            loop {
                previews.begin_frame();
                previews.retain("editor");
                previews.watch(source.clone());
                previews.end_frame(&ctx);
                if previews.entries["editor"]
                    .document
                    .as_ref()
                    .is_some_and(|d| d.status.contains("paused") == paused)
                {
                    break;
                }
                assert!(
                    started.elapsed() < Duration::from_secs(2),
                    "Preview error: {:?}",
                    previews.entries["editor"].error
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(
                previews.entries["editor"].document.as_ref().unwrap().text,
                "# Unsaved"
            );
        }
        server.join().unwrap();
    }

    #[test]
    fn relative_links_and_images_resolve_against_the_document() {
        let doc = prepare(snapshot(
            "[Next](nested/next%20file.md)\n\n![a **picture**](<images/a ) b.png>)\n\n![Reference][photo]\n\n[photo]: ../photo.png\n\n`![literal](no.png)`",
        ));
        assert_eq!(
            doc.links["nested/next%20file.md"],
            Some(Link::File("/docs/nested/next file.md".into()))
        );
        assert!(
            doc.images
                .contains("markdown-image:file:///docs/images/a%20)%20b.png"),
            "{:?}",
            doc.images
        );
        assert!(doc.images.contains("markdown-image:file:///photo.png"));
        let rendered_images = Parser::new(&doc.text)
            .filter(|e| matches!(e, Event::Start(Tag::Image { .. })))
            .count();
        assert_eq!(rendered_images, 2);
        assert!(doc.text.contains("`![literal](no.png)`"));
        assert!(doc.text.contains("![a picture](<markdown-image:"));
    }

    #[test]
    fn preview_links_do_not_execute_arbitrary_url_schemes_or_fetch_remote_images() {
        let doc = prepare(snapshot(
            "[bad](javascript:alert) [shell](command:run) [web](https://example.com) [section](#title) ![remote](https://example.com/a.png)",
        ));
        assert_eq!(doc.links["javascript:alert"], None);
        assert_eq!(doc.links["command:run"], None);
        assert_eq!(
            doc.links["https://example.com"],
            Some(Link::Web("https://example.com/".into()))
        );
        assert!(!doc.links.contains_key("#title"));
        assert!(doc.images.is_empty());
        assert!(doc.text.contains("Image: remote"));
    }

    #[test]
    fn saved_file_preview_reports_changes_missing_files_and_size_limits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.MD");
        let source = Source {
            session: "fixture".into(),
            path: path.clone(),
            socket: dir.path().join("missing.sock"),
        };
        std::fs::write(&path, "# First").unwrap();
        let first = read_source(&source, None).unwrap().unwrap();
        assert_eq!(first.text, "# First");
        assert_eq!(prepare(first).status, "Saved file");
        std::fs::write(&path, "# Changed").unwrap();
        assert_eq!(
            read_source(&source, None).unwrap().unwrap().text,
            "# Changed"
        );
        std::fs::File::create(&path)
            .unwrap()
            .set_len(MAX_DOCUMENT as u64 + 1)
            .unwrap();
        assert!(
            read_source(&source, None)
                .unwrap_err()
                .to_string()
                .contains("1 MiB")
        );
        std::fs::write(&path, [255]).unwrap();
        assert!(
            read_source(&source, None)
                .unwrap_err()
                .to_string()
                .contains("UTF-8")
        );
        std::fs::remove_file(&path).unwrap();
        assert!(read_source(&source, None).is_err());
        assert!(supported(&path));
        assert!(!supported(Path::new("file.mdx")));
        assert!(!supported(Path::new("file.rs")));
    }

    #[test]
    fn stale_preview_updates_cannot_replace_a_newer_watch() {
        let ctx = egui::Context::default();
        let mut previews = Previews::new(&ctx);
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        previews.results = rx;
        previews.generation = 2;
        previews.entries.insert("editor".into(), Preview::default());
        tx.try_send((1, "editor".into(), Ok(prepare(snapshot("old")))))
            .unwrap();
        previews.begin_frame();
        assert!(previews.entries["editor"].document.is_none());
        tx.try_send((2, "editor".into(), Ok(prepare(snapshot("new")))))
            .unwrap();
        previews.begin_frame();
        assert_eq!(
            previews.entries["editor"].document.as_ref().unwrap().text,
            "new"
        );
        previews.end_frame(&ctx);
        assert!(previews.entries.is_empty());
    }

    #[test]
    fn refresh_failure_keeps_last_preview_and_recovery_clears_the_error() {
        let mut preview = Preview::default();
        let mut dirty = snapshot("# Unsaved");
        dirty.revision = Some(Revision {
            buffer: 1,
            tick: 3,
            modified: true,
        });
        preview.apply(Ok(prepare(dirty)));
        assert_eq!(preview.document.as_ref().unwrap().status, "Unsaved changes");
        preview.apply(Err("Editor temporarily unavailable".into()));
        assert_eq!(preview.document.as_ref().unwrap().text, "# Unsaved");
        preview.apply(Ok(prepare(snapshot("# Recovered"))));
        assert!(preview.error.is_none());
    }

    #[test]
    fn clicking_preview_suspends_editor_keyboard_focus() {
        let ctx = egui::Context::default();
        let mut preview = Preview {
            preview_rect: egui::Rect::from_min_max(
                egui::pos2(100.0, 0.0),
                egui::pos2(200.0, 100.0),
            ),
            ..Default::default()
        };
        let mut output = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::PointerButton {
                    pos: egui::pos2(150.0, 40.0),
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                }],
                ..Default::default()
            },
            |ui| preview.pointer_focus(ui),
        );
        output.textures_delta.clear();
        assert!(!preview.editor_focused);
    }
}
