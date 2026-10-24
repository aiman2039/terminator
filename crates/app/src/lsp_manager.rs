//! GUI-owned language servers for native editor tabs.
//!
//! The engine stays I/O-free: this manager tracks one server per
//! (language, project root), while the actual child processes and their
//! stdio pumps run as actors on the GUI service supervisor. Nothing here
//! needs a Tokio runtime on the calling thread, so per-frame sync from
//! the GUI thread can never panic it into existence. Diagnostics plus
//! pending hover/goto answers stay here for the views to poll. Servers
//! die with the GUI (inbox drop plus actor cancellation on prune); a
//! missing binary or a crash loop surfaces as a hover message, never a
//! modal error. Only Rust and Python have servers (see
//! `terminator_native_edit::lsp::server_candidates`); every other
//! language edits exactly as before, without LSP.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use terminator_core::async_service::{CancellationToken, OperationContext, Policy};
use terminator_native_edit::highlight::Language;
use terminator_native_edit::lsp as L;
use terminator_native_edit::view::DiagnosticMark;

use crate::gui_services::Services;

/// Full-text sync at most this often after the last edit: diagnostics
/// lag typing by ~1s instead of hammering the server per keystroke.
const SYNC_DEBOUNCE: Duration = Duration::from_secs(1);
/// Cooldown between spawn attempts for one server (crash loops degrade
/// to retries, then to unavailable).
const SPAWN_COOLDOWN: Duration = Duration::from_secs(10);
/// Consecutive crashes before a server counts as unavailable.
const MAX_CRASHES: u32 = 3;
/// Delay before re-asking for an empty outline: an indexing server
/// answers `documentSymbol` with nothing, and caching that would hide
/// symbols until the next edit.
const SYMBOLS_RETRY: Duration = Duration::from_secs(2);

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct ServerKey {
    language: &'static str,
    root: PathBuf,
}

/// Supervisor resource for one server: language plus project root, so a
/// second project's server admits alongside the first instead of
/// queueing behind its never-ending lifetime.
fn server_resource(key: &ServerKey) -> String {
    format!("{}:{}", key.language, key.root.display())
}

struct OpenDoc {
    uri: url::Url,
    version: i32,
    server: ServerKey,
    /// Protocol `languageId` for `didOpen` (`.tsx` differs from `.ts`).
    language_id: &'static str,
    /// Buffer revision of the last submitted text.
    submitted: u64,
    /// Buffer revision last seen by [`LspManager::sync_doc`]. The debounce
    /// timer restarts only when this changes: while an edit waits out the
    /// delay, `sync_doc` runs every frame with the same revision and must
    /// not push `last_change` forward, or the delay never ends and edited
    /// text never reaches the server.
    observed: u64,
    last_change: Instant,
    opened: bool,
    /// Latest text submitted while the server was still starting. The
    /// handshake flushes it as `didOpen` on completion, so a buffer
    /// submitted once (no further frames) still opens. Latest wins:
    /// every pre-ready submit overwrites it.
    pending_text: Option<String>,
}

enum Pending {
    Hover {
        path: PathBuf,
        cursor: (usize, usize),
    },
    Definition {
        path: PathBuf,
    },
    References {
        path: PathBuf,
    },
    Rename {
        path: PathBuf,
        new_name: String,
        revision: u64,
    },
    Symbols {
        path: PathBuf,
        revision: u64,
    },
    WorkspaceSymbols {
        generation: u64,
    },
}

struct Server {
    inbox: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    /// Directly spawned child (test path without services): the GUI
    /// kills it on prune. Supervised servers leave this empty; their
    /// child is owned by the supervisor actor, killed via `cancel`.
    child: Option<tokio::process::Child>,
    /// Supervisor actor kill handle. Cancelling it stops the pumps and
    /// reaps the child without a `Down` event, so deliberate shutdowns
    /// never count as crashes.
    cancel: Option<CancellationToken>,
    ids: L::IdGen,
    pending: HashMap<i64, Pending>,
    init_id: Option<i64>,
    ready: bool,
}

pub struct HoverPopup {
    path: PathBuf,
    cursor_line: usize,
    cursor_col: usize,
    pub text: String,
    pub markdown: bool,
    pub open: bool,
    /// A status note ("starting…", "not found"), not a server answer:
    /// a null hover clears these, never real answers.
    status: bool,
}

/// A finished goto request, ready for the workspace to open.
pub struct GotoJump {
    pub from: PathBuf,
    pub targets: Vec<L::GotoTarget>,
}

/// Reference locations returned for one native editor request.
pub struct ReferenceResults {
    pub from: PathBuf,
    pub targets: Vec<L::GotoTarget>,
}

pub struct RenameResults {
    pub from: PathBuf,
    pub new_name: String,
    pub edits: Result<Vec<L::RenameFileEdit>, String>,
}

enum PumpEvent {
    Body { key: ServerKey, body: Vec<u8> },
    Down { key: ServerKey },
    SpawnFailed { key: ServerKey, note: String },
}

pub struct LspManager {
    servers: HashMap<ServerKey, Server>,
    docs: HashMap<PathBuf, OpenDoc>,
    diagnostics: HashMap<PathBuf, Vec<L::Diagnostic>>,
    hover: Option<HoverPopup>,
    goto: Option<GotoJump>,
    references: Option<ReferenceResults>,
    rename: Option<RenameResults>,
    symbols: std::collections::HashMap<PathBuf, Vec<L::Symbol>>,
    symbols_rev: std::collections::HashMap<PathBuf, u64>,
    /// Last empty outline answer per path (indexing retry gate).
    symbols_retry: HashMap<PathBuf, Instant>,
    workspace_symbol_query: String,
    /// `Some` limits `workspace/symbol` to servers for that project.
    /// `None` asks every ready server.
    workspace_symbol_root: Option<PathBuf>,
    workspace_symbol_generation: u64,
    workspace_symbols: HashMap<ServerKey, Vec<L::WorkspaceSymbolHit>>,
    search_revision: u64,
    unavailable: HashMap<&'static str, String>,
    /// Consecutive crashes per language (reset on a good handshake).
    failures: HashMap<&'static str, u32>,
    /// Last crash/spawn-failure time per language (respawn cooldown).
    cooled: HashMap<&'static str, Instant>,
    pump_tx: tokio::sync::mpsc::UnboundedSender<PumpEvent>,
    pump_rx: tokio::sync::mpsc::UnboundedReceiver<PumpEvent>,
    /// GUI service supervisor: server processes and stdio pumps run as
    /// its actors, never on the calling thread. `None` keeps the direct
    /// spawn path for tests that own their Tokio runtime.
    services: Option<Services>,
    /// Settings → Terminal & Editor server executables by language id.
    /// Empty values fall back to the default candidates on `PATH`.
    overrides: std::collections::BTreeMap<String, String>,
}

impl LspManager {
    pub fn new() -> Self {
        let (pump_tx, pump_rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            servers: HashMap::new(),
            docs: HashMap::new(),
            diagnostics: HashMap::new(),
            hover: None,
            goto: None,
            references: None,
            rename: None,
            symbols: std::collections::HashMap::new(),
            symbols_rev: std::collections::HashMap::new(),
            symbols_retry: HashMap::new(),
            workspace_symbol_query: String::new(),
            workspace_symbol_root: None,
            workspace_symbol_generation: 0,
            workspace_symbols: HashMap::new(),
            search_revision: 0,
            unavailable: HashMap::new(),
            failures: HashMap::new(),
            cooled: HashMap::new(),
            pump_tx,
            pump_rx,
            services: None,
            overrides: std::collections::BTreeMap::new(),
        }
    }

    /// Apply Settings → Terminal & Editor server paths. Saving restarts
    /// only the servers whose path changed, so the new binary is used
    /// without restarting the app. Unchanged settings do nothing.
    pub fn set_server_overrides(&mut self, overrides: &std::collections::BTreeMap<String, String>) {
        let normalized: std::collections::BTreeMap<String, String> = overrides
            .iter()
            .map(|(language, value)| (language.clone(), value.trim().to_owned()))
            .filter(|(_, value)| !value.is_empty())
            .collect();
        if normalized == self.overrides {
            return;
        }
        let mut changed: Vec<&'static str> = Vec::new();
        for language in ["rust", "python", "typescript"] {
            let before = self.overrides.get(language);
            let after = normalized.get(language);
            if before != after {
                changed.push(language);
            }
        }
        self.overrides = normalized;
        for &language in &changed {
            self.unavailable.remove(language);
            self.failures.remove(language);
            self.cooled.remove(language);
            let keys: Vec<ServerKey> = self
                .servers
                .keys()
                .filter(|key| key.language == language)
                .cloned()
                .collect();
            for key in keys {
                self.shutdown_server(&key);
            }
        }
        if !changed.is_empty() {
            self.symbols.clear();
            self.symbols_rev.clear();
            self.symbols_retry.clear();
            self.search_revision = self.search_revision.wrapping_add(1);
        }
    }

    /// Attach the GUI service supervisor. After this, server startup and
    /// stdio run as supervised actors; the calling thread only passes
    /// channels, so sync from the GUI thread needs no Tokio runtime.
    pub fn set_services(&mut self, services: Services) {
        self.services = Some(services);
    }

    /// Drain pump events: route diagnostics, answers, and server deaths.
    /// Call once per frame before any per-doc sync.
    pub fn poll(&mut self) {
        while let Ok(event) = self.pump_rx.try_recv() {
            match event {
                PumpEvent::Body { key, body } => self.on_body(&key, &body),
                PumpEvent::Down { key } => self.on_server_down(&key),
                PumpEvent::SpawnFailed { key, note } => self.on_spawn_failed(&key, &note),
            }
        }
    }

    /// Per-frame sync for one loaded buffer. Returns true when the
    /// caller should clone the text and hand it to [`submit_text`]:
    /// unknown buffers, unopened buffers with a ready (or missing)
    /// server, and edits older than the debounce. Unsupported languages
    /// untrack (and close) silently.
    pub fn sync_doc(&mut self, path: &Path, language: Language, revision: u64) -> bool {
        if L::server_language(language).is_none() {
            self.close_path(path);
            return false;
        }
        match self.docs.get_mut(path) {
            None => true,
            Some(doc) => {
                if doc.observed != revision {
                    doc.observed = revision;
                    doc.last_change = Instant::now();
                }
                if !doc.opened {
                    // Open as soon as a server can take it; a starting
                    // server waits, a dead one respawns in submit.
                    return self.servers.get(&doc.server).is_none_or(|s| s.ready);
                }
                doc.submitted != revision && doc.last_change.elapsed() >= SYNC_DEBOUNCE
            }
        }
    }

    /// Open or refresh one buffer on its server. Always records the
    /// revision (latest text wins). Text submitted while the server is
    /// still starting parks on the doc and flushes as `didOpen` when the
    /// handshake completes, so a slow start neither blocks typing nor
    /// drops the open.
    pub fn submit_text(&mut self, path: &Path, language: Language, revision: u64, text: &str) {
        let Some(language_id) = L::server_language(language) else {
            return;
        };
        let Some(protocol_id) = L::language_id_for_path(path, language) else {
            return;
        };
        let root = project_root(path, language);
        let key = ServerKey {
            language: language_id,
            root,
        };
        let Some(uri) = L::file_uri(path) else {
            return;
        };
        let now = Instant::now();
        let doc = self.docs.entry(path.to_owned()).or_insert_with(|| OpenDoc {
            uri,
            version: 0,
            server: key.clone(),
            language_id: protocol_id,
            submitted: revision,
            observed: revision,
            last_change: now,
            opened: false,
            pending_text: None,
        });
        if doc.server != key {
            doc.server = key.clone();
            doc.opened = false;
        }
        doc.language_id = protocol_id;
        let was_submitted = doc.submitted;
        doc.submitted = revision;
        doc.observed = revision;
        if !self.servers.get(&key).is_some_and(|server| server.ready) {
            let _ = self.ensure_server(&key);
            // Park the latest text for the handshake flush, but only
            // while a server is actually starting: an unavailable server
            // would pile a copy per frame with nobody to flush it.
            if self.servers.contains_key(&key)
                && let Some(doc) = self.docs.get_mut(path)
            {
                doc.pending_text = Some(text.to_owned());
            }
            return;
        }
        let inbox = self.servers.get(&key).map(|server| server.inbox.clone());
        let Some(inbox) = inbox else {
            return;
        };
        let Some(doc) = self.docs.get_mut(path) else {
            return;
        };
        // A parked open is already flushed or superseded; either way the
        // live text below is what the server must hold.
        doc.pending_text = None;
        if !doc.opened {
            doc.version = doc.version.saturating_add(1);
            let bytes = L::did_open(&doc.uri, doc.language_id, doc.version, text);
            let _ = inbox.send(bytes);
            doc.opened = true;
        } else if was_submitted != revision {
            doc.version = doc.version.saturating_add(1);
            let bytes = L::did_change(&doc.uri, doc.version, text);
            let _ = inbox.send(bytes);
        }
    }

    /// Ask for hover at a UTF-16 position; the answer lands in
    /// [`hover_popup`]. Without a server the popup explains why.
    pub fn request_hover(&mut self, path: &Path, position: L::Position, cursor: (usize, usize)) {
        let path = path.to_owned();
        if let Some(note) = self.server_status(&path) {
            self.hover = Some(HoverPopup {
                path,
                cursor_line: cursor.0,
                cursor_col: cursor.1,
                text: note,
                markdown: false,
                open: true,
                status: true,
            });
            return;
        }
        let key = self.docs.get(&path).map(|doc| doc.server.clone());
        let uri = self.docs.get(&path).map(|doc| doc.uri.clone());
        let (Some(key), Some(uri)) = (key, uri) else {
            return;
        };
        let Some(server) = self.servers.get_mut(&key) else {
            return;
        };
        if !server.ready {
            self.hover = Some(HoverPopup {
                path,
                cursor_line: cursor.0,
                cursor_col: cursor.1,
                text: "Language server is starting…".into(),
                markdown: false,
                open: true,
                status: true,
            });
            return;
        }
        let L::RequestId::Number(number) = server.ids.next_id() else {
            return;
        };
        let _ = server
            .inbox
            .send(L::hover(&L::RequestId::Number(number), &uri, position));
        server.pending.insert(
            number,
            Pending::Hover {
                path,
                cursor: (cursor.0, cursor.1),
            },
        );
    }

    /// Ask for the definition of the symbol at a UTF-16 position; the
    /// answer lands in [`take_goto`]. Silent without a ready server
    /// (nothing to jump to, same as an empty answer).
    pub fn request_definition(&mut self, path: &Path, position: L::Position) {
        let path = path.to_owned();
        let key = self.docs.get(&path).map(|doc| doc.server.clone());
        let uri = self.docs.get(&path).map(|doc| doc.uri.clone());
        let (Some(key), Some(uri)) = (key, uri) else {
            return;
        };
        let Some(server) = self.servers.get_mut(&key) else {
            return;
        };
        if !server.ready {
            return;
        }
        let L::RequestId::Number(number) = server.ids.next_id() else {
            return;
        };
        let _ = server
            .inbox
            .send(L::definition(&L::RequestId::Number(number), &uri, position));
        server.pending.insert(number, Pending::Definition { path });
    }

    /// Ask for references at a UTF-16 position. The response is available
    /// through [`take_references`] and never changes files.
    pub fn request_references(&mut self, path: &Path, position: L::Position) {
        let Some(doc) = self.docs.get(path) else {
            return;
        };
        let (key, uri) = (doc.server.clone(), doc.uri.clone());
        let Some(server) = self.servers.get_mut(&key).filter(|server| server.ready) else {
            return;
        };
        let L::RequestId::Number(number) = server.ids.next_id() else {
            return;
        };
        if server
            .inbox
            .send(L::references(&L::RequestId::Number(number), &uri, position))
            .is_ok()
        {
            server.pending.insert(
                number,
                Pending::References {
                    path: path.to_owned(),
                },
            );
        }
    }

    /// Request a preview of a symbol rename. No file is changed here.
    pub fn request_rename(
        &mut self,
        path: &Path,
        position: L::Position,
        new_name: &str,
    ) -> Result<(), String> {
        if let Some(note) = self.server_status(path) {
            return Err(note);
        }
        let doc = self
            .docs
            .get(path)
            .ok_or("No language server tracks this file")?;
        let (key, uri, revision) = (doc.server.clone(), doc.uri.clone(), doc.submitted);
        let server = self
            .servers
            .get_mut(&key)
            .filter(|server| server.ready)
            .ok_or("Language server is not ready")?;
        let L::RequestId::Number(number) = server.ids.next_id() else {
            return Err("Could not allocate a language server request".into());
        };
        server
            .inbox
            .send(L::rename(
                &L::RequestId::Number(number),
                &uri,
                position,
                new_name,
            ))
            .map_err(|_| "Language server connection closed".to_string())?;
        server.pending.insert(
            number,
            Pending::Rename {
                path: path.to_owned(),
                new_name: new_name.to_owned(),
                revision,
            },
        );
        Ok(())
    }

    pub fn document_version(&self, path: &Path) -> Option<i32> {
        self.docs
            .get(path)
            .filter(|doc| doc.opened)
            .map(|doc| doc.version)
    }

    pub fn invalidate_paths(&mut self, paths: &[PathBuf]) {
        for path in paths {
            self.close_path(path);
        }
    }

    /// Ask for the document outline of a tracked buffer. Cheap to call
    /// every frame: it only sends when the submitted revision moved past
    /// the cached answer and no request for the file is in flight.
    pub fn request_symbols(&mut self, path: &Path) {
        let Some(doc) = self.docs.get(path) else {
            return;
        };
        let (key, uri, revision) = (doc.server.clone(), doc.uri.clone(), doc.submitted);
        if self.symbols_rev.get(path) == Some(&revision) {
            // An empty outline usually means the server is still indexing.
            // Keep asking on a slow cadence until symbols arrive instead
            // of caching the empty answer until the next edit.
            let empty = self.symbols.get(path).is_none_or(Vec::is_empty);
            if !empty
                || self
                    .symbols_retry
                    .get(path)
                    .is_some_and(|at| at.elapsed() < SYMBOLS_RETRY)
            {
                return;
            }
        }
        let Some(server) = self.servers.get_mut(&key) else {
            return;
        };
        if !server.ready
            || server
                .pending
                .values()
                .any(|pending| matches!(pending, Pending::Symbols { path: p, .. } if p == path))
        {
            return;
        }
        let L::RequestId::Number(number) = server.ids.next_id() else {
            return;
        };
        let _ = server
            .inbox
            .send(L::document_symbol(&L::RequestId::Number(number), &uri));
        server.pending.insert(
            number,
            Pending::Symbols {
                path: path.to_owned(),
                revision,
            },
        );
    }

    /// Cached outline for `path`, in document order (depth-tagged).
    #[must_use]
    pub fn symbols_for(&self, path: &Path) -> Vec<L::Symbol> {
        self.symbols.get(path).cloned().unwrap_or_default()
    }

    /// Request fresh document outlines for every open native editor. The
    /// per-document cache and pending guard keep repeated calls cheap.
    pub fn request_open_document_symbols(&mut self) {
        let paths: Vec<PathBuf> = self.docs.keys().cloned().collect();
        for path in paths {
            self.request_symbols(&path);
        }
    }

    /// Manual recovery for one buffer: forget crash backoff and cached
    /// answers, then stop every server for its language. The next frame
    /// re-submits the open text, which respawns the server and makes it
    /// index again. A binary installed after startup is found this time.
    pub fn restart(&mut self, path: &Path) {
        let Some(language) = self.docs.get(path).map(|doc| doc.server.language) else {
            return;
        };
        self.restart_servers(Some(language));
    }

    /// [`restart`](Self::restart) for every language, used by the
    /// Problems panel where no single editor is implied.
    pub fn restart_all(&mut self) {
        self.restart_servers(None);
    }

    fn restart_servers(&mut self, language: Option<&'static str>) {
        if let Some(language) = language {
            self.unavailable.remove(language);
            self.failures.remove(language);
            self.cooled.remove(language);
        } else {
            self.unavailable.clear();
            self.failures.clear();
            self.cooled.clear();
        }
        let keys: Vec<ServerKey> = self
            .servers
            .keys()
            .filter(|key| language.is_none_or(|wanted| key.language == wanted))
            .cloned()
            .collect();
        for key in keys {
            self.shutdown_server(&key);
        }
        self.symbols.clear();
        self.symbols_rev.clear();
        self.symbols_retry.clear();
        self.search_revision = self.search_revision.wrapping_add(1);
    }

    /// Cached document symbols from all open native buffers.
    pub fn all_document_symbols(&self) -> Vec<(PathBuf, L::Symbol)> {
        self.symbols
            .iter()
            .flat_map(|(path, symbols)| {
                symbols.iter().cloned().map(|symbol| (path.clone(), symbol))
            })
            .collect()
    }

    pub fn clear_workspace_symbols(&mut self) {
        if self.workspace_symbol_query.is_empty()
            && self.workspace_symbols.is_empty()
            && self.workspace_symbol_root.is_none()
        {
            return;
        }
        self.workspace_symbol_query.clear();
        self.workspace_symbol_root = None;
        self.workspace_symbol_generation = self.workspace_symbol_generation.wrapping_add(1);
        self.workspace_symbols.clear();
        self.search_revision = self.search_revision.wrapping_add(1);
        for server in self.servers.values_mut() {
            server
                .pending
                .retain(|_, pending| !matches!(pending, Pending::WorkspaceSymbols { .. }));
        }
    }

    /// Ask each ready project server for matching symbols once per query.
    /// `root` keeps the request on servers for that project. A later query or
    /// a different root discards old pending replies by generation.
    pub fn request_scoped_symbols(&mut self, query: &str, root: Option<&Path>) {
        let query = query.trim();
        if query.is_empty() {
            self.clear_workspace_symbols();
            return;
        }
        let root_changed = match (&self.workspace_symbol_root, root) {
            (None, None) => false,
            (Some(current), Some(next)) => current.as_path() != next,
            _ => true,
        };
        if self.workspace_symbol_query != query || root_changed {
            self.clear_workspace_symbols();
            query.clone_into(&mut self.workspace_symbol_query);
            self.workspace_symbol_root = root.map(Path::to_path_buf);
        }
        let generation = self.workspace_symbol_generation;
        for (key, server) in &mut self.servers {
            if self.workspace_symbol_root.as_ref().is_some_and(|root| {
                key.root != *root && !key.root.starts_with(root) && !root.starts_with(&key.root)
            }) {
                continue;
            }
            if !server.ready || self.workspace_symbols.contains_key(key)
                || server.pending.values().any(|pending| matches!(pending, Pending::WorkspaceSymbols { generation: pending_generation } if *pending_generation == generation))
            {
                continue;
            }
            let L::RequestId::Number(number) = server.ids.next_id() else {
                continue;
            };
            if server
                .inbox
                .send(L::workspace_symbol(&L::RequestId::Number(number), query))
                .is_ok()
            {
                server
                    .pending
                    .insert(number, Pending::WorkspaceSymbols { generation });
            }
        }
    }

    pub fn workspace_symbols(&self) -> Vec<L::WorkspaceSymbolHit> {
        self.workspace_symbols
            .values()
            .flat_map(|hits| hits.iter().cloned())
            .collect()
    }

    pub fn workspace_symbols_pending(&self) -> bool {
        self.servers.values().any(|server| server.pending.values().any(|pending| matches!(pending, Pending::WorkspaceSymbols { generation } if *generation == self.workspace_symbol_generation)))
    }

    pub fn search_revision(&self) -> u64 {
        self.search_revision
    }

    /// Current hover popup for `path`, dismissed when the cursor moved
    /// on. Returns the text to show, if any.
    pub fn hover_popup(&mut self, path: &Path, cursor: (usize, usize)) -> Option<(String, bool)> {
        let popup = self.hover.as_mut()?;
        if popup.path != path || (popup.cursor_line, popup.cursor_col) != cursor || !popup.open {
            if popup.path == path {
                self.hover = None;
            }
            return None;
        }
        Some((popup.text.clone(), popup.markdown))
    }

    pub fn close_hover(&mut self) {
        if let Some(popup) = self.hover.as_mut() {
            popup.open = false;
        }
    }

    /// Finished goto jump, if any.
    pub fn take_goto(&mut self) -> Option<GotoJump> {
        self.goto.take()
    }

    pub fn take_references(&mut self) -> Option<ReferenceResults> {
        self.references.take()
    }

    pub fn take_rename(&mut self) -> Option<RenameResults> {
        self.rename.take()
    }

    /// Worst severity per buffer row, sorted for the view's binary
    /// search. Lines beyond `u32` never occur; they are skipped.
    pub fn row_marks(&self, path: &Path) -> Vec<DiagnosticMark> {
        let mut worst: HashMap<usize, L::Severity> = HashMap::new();
        if let Some(items) = self.diagnostics.get(path) {
            for item in items {
                let Ok(line) = usize::try_from(item.range.start.line) else {
                    continue;
                };
                worst
                    .entry(line)
                    .and_modify(|known| {
                        if let Some(severity) = item.severity
                            && severity < *known
                        {
                            *known = severity;
                        }
                    })
                    .or_insert_with(|| item.severity.unwrap_or(L::Severity::Hint));
            }
        }
        let mut marks: Vec<DiagnosticMark> = worst
            .into_iter()
            .map(|(line, severity)| DiagnosticMark { line, severity })
            .collect();
        marks.sort_by_key(|mark| mark.line);
        marks
    }

    /// All diagnostics for one path, for the problems panel.
    pub fn problems_for(&self, path: &Path) -> Vec<L::Diagnostic> {
        self.diagnostics.get(path).cloned().unwrap_or_default()
    }

    /// Every tracked file's diagnostics, sorted by path, for the
    /// global Problems panel.
    #[must_use]
    pub fn all_problems(&self) -> Vec<(PathBuf, Vec<L::Diagnostic>)> {
        let mut files: Vec<(PathBuf, Vec<L::Diagnostic>)> = self
            .diagnostics
            .iter()
            .filter(|(_, items)| !items.is_empty())
            .map(|(path, items)| (path.clone(), items.clone()))
            .collect();
        files.sort_by(|a, b| a.0.cmp(&b.0));
        files
    }

    /// `(errors, warnings, total)` across every tracked file, for the
    /// status bar. Missing severity counts toward the total only.
    #[must_use]
    pub fn problem_counts(&self) -> (usize, usize, usize) {
        let mut errors: usize = 0;
        let mut warnings: usize = 0;
        let mut total: usize = 0;
        for items in self.diagnostics.values() {
            for item in items {
                total = total.saturating_add(1);
                match item.severity {
                    Some(L::Severity::Error) => errors = errors.saturating_add(1),
                    Some(L::Severity::Warning) => warnings = warnings.saturating_add(1),
                    _ => {}
                }
            }
        }
        (errors, warnings, total)
    }

    /// Whether a document is tracked for `path` (a server was consulted).
    #[must_use]
    pub fn tracked(&self, path: &Path) -> bool {
        self.docs.contains_key(path)
    }

    /// Why `path` has no ready server, if tracked and not ready.
    #[must_use]
    pub fn server_note(&self, path: &Path) -> Option<String> {
        self.server_status(path)
    }

    /// Why `path` has no working server, if that is the case: missing
    /// binary, repeated crashes, or still starting. `None` means ready.
    fn server_status(&self, path: &Path) -> Option<String> {
        let doc = self.docs.get(path)?;
        if let Some(note) = self.unavailable.get(doc.server.language) {
            return Some(note.clone());
        }
        let server = self.servers.get(&doc.server)?;
        if server.ready {
            return None;
        }
        Some(format!("Starting {}…", doc.server.language))
    }

    fn close_path(&mut self, path: &Path) {
        if let Some(doc) = self.docs.remove(path)
            && doc.opened
            && let Some(server) = self.servers.get(&doc.server)
        {
            let _ = server.inbox.send(L::did_close(&doc.uri));
        }
        self.diagnostics.remove(path);
        let had_symbols = self.symbols.remove(path).is_some();
        self.symbols_rev.remove(path);
        self.symbols_retry.remove(path);
        if had_symbols {
            self.search_revision = self.search_revision.wrapping_add(1);
        }
        if self.hover.as_ref().is_some_and(|popup| popup.path == path) {
            self.hover = None;
        }
        if self
            .references
            .as_ref()
            .is_some_and(|result| result.from == path)
        {
            self.references = None;
        }
        if self
            .rename
            .as_ref()
            .is_some_and(|result| result.from == path)
        {
            self.rename = None;
        }
    }

    /// A running server for `key`, spawning one when needed. `None`
    /// means unavailable (binary missing, spawn failed, crash loop):
    /// the reason lands in `unavailable` for hover feedback.
    fn ensure_server(&mut self, key: &ServerKey) -> Option<&mut Server> {
        if self.servers.contains_key(key) {
            return self.servers.get_mut(key);
        }
        if self.unavailable.contains_key(key.language) {
            return None;
        }
        // Crash loops back off, then give up with a reason.
        if self.failures.get(key.language).copied().unwrap_or(0) >= MAX_CRASHES {
            self.unavailable.insert(
                key.language,
                format!("{} keeps crashing; restart the app to retry", key.language),
            );
            return None;
        }
        if self
            .cooled
            .get(key.language)
            .is_some_and(|at| at.elapsed() < SPAWN_COOLDOWN)
        {
            return None;
        }
        let Some(path) = self.resolve_server(key.language) else {
            self.unavailable
                .insert(key.language, self.missing_server_note(key.language));
            return None;
        };
        if self.services.is_some() {
            return self.spawn_supervised(key, &path);
        }
        match spawn_server_direct(key, &path, self.pump_tx.clone()) {
            Ok(server) => {
                self.servers.insert(key.clone(), server);
                self.servers.get_mut(key)
            }
            Err(note) => {
                self.note_failure(key.language);
                self.unavailable.insert(key.language, note);
                None
            }
        }
    }

    /// Server executable for one language: the Settings override when
    /// set (absolute path or bare name), else the default candidates,
    /// including the usual GUI-launch directories and `~/.cargo/bin`.
    fn resolve_server(&self, language: &'static str) -> Option<PathBuf> {
        if let Some(custom) = self
            .overrides
            .get(language)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            return terminator_core::find_executable(custom);
        }
        terminator_native_edit::lsp::server_candidates(language_of(language))
            .iter()
            .find_map(|candidate| terminator_core::find_executable(candidate))
    }

    /// Why no executable was found, naming the Settings row that fixes it.
    fn missing_server_note(&self, language: &'static str) -> String {
        if let Some(custom) = self
            .overrides
            .get(language)
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            return format!(
                "Language server not found: {custom}. Fix the path in Settings → Terminal & Editor."
            );
        }
        let candidates = terminator_native_edit::lsp::server_candidates(language_of(language));
        format!(
            "No language server on PATH ({}); install it or set a path in Settings → Terminal & Editor.",
            candidates.join(", ")
        )
    }

    /// Insert an optimistic entry for `key` and start its process plus
    /// stdio pumps as a supervisor actor. Needs no Tokio runtime on the
    /// calling thread: channels and the submit are plain sends. The
    /// handshake (`initialize`) queues on the inbox at once, so a slow
    /// start behaves exactly like the direct path; a failed `exec`
    /// reports back as [`PumpEvent::SpawnFailed`].
    fn spawn_supervised(&mut self, key: &ServerKey, binary: &Path) -> Option<&mut Server> {
        let services = self.services.clone()?;
        let (inbox, incoming) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let cancel = CancellationToken::new();
        let actor_cancel = cancel.clone();
        self.servers.insert(
            key.clone(),
            Server {
                inbox,
                child: None,
                cancel: Some(cancel),
                ids: L::IdGen::default(),
                pending: HashMap::new(),
                init_id: None,
                ready: false,
            },
        );
        let actor_key = key.clone();
        let root = key.root.clone();
        let binary = binary.to_owned();
        let pump = self.pump_tx.clone();
        // One supervised task per (language, root): the supervisor
        // admits a single active task per key, so keying by language
        // alone would park every second project's server behind the
        // first forever.
        let context = OperationContext::new("lsp", server_resource(key), Policy::ServiceLifetime);
        if services
            .handle()
            .submit(context, actor_cancel.clone(), async move {
                run_server_actor(actor_key, binary, root, incoming, pump, actor_cancel).await?;
                Ok(Vec::new())
            })
            .is_err()
        {
            self.servers.remove(key);
            self.note_failure(key.language);
            self.unavailable
                .insert(key.language, format!("Could not start {}", key.language));
            return None;
        }
        let server = self.servers.get_mut(key)?;
        let L::RequestId::Number(init) = server.ids.next_id() else {
            self.servers.remove(key);
            self.note_failure(key.language);
            return None;
        };
        let root_uri = L::file_uri(&key.root);
        let Some(root_uri) = root_uri else {
            self.servers.remove(key);
            self.note_failure(key.language);
            return None;
        };
        let bytes = L::initialize(&L::RequestId::Number(init), std::process::id(), &root_uri);
        if server.inbox.send(bytes).is_err() {
            self.servers.remove(key);
            self.note_failure(key.language);
            return None;
        }
        server.init_id = Some(init);
        self.servers.get_mut(key)
    }

    /// A supervised spawn that failed inside the actor: same outcome as a
    /// direct spawn error, reported asynchronously. Parked text survives
    /// for the next respawn's handshake flush.
    fn on_spawn_failed(&mut self, key: &ServerKey, note: &str) {
        if self.servers.remove(key).is_some() {
            self.note_failure(key.language);
            self.unavailable.insert(key.language, note.to_owned());
            for doc in self.docs.values_mut().filter(|doc| doc.server == *key) {
                doc.opened = false;
            }
        }
    }

    fn note_failure(&mut self, language: &'static str) {
        let count = self.failures.get(language).copied().unwrap_or(0);
        self.failures.insert(language, count.saturating_add(1));
        self.cooled.insert(language, Instant::now());
    }

    fn on_body(&mut self, key: &ServerKey, body: &[u8]) {
        let Some(message) = L::parse_message(body) else {
            return;
        };
        match message {
            L::LspMessage::Notification { method, params } => {
                if method == "textDocument/publishDiagnostics"
                    && let Some((uri, items)) = L::parse_diagnostics(&params)
                    && let Some(path) = L::uri_to_path(&uri)
                    && self.docs.contains_key(&path)
                {
                    self.diagnostics.insert(path, items);
                }
            }
            L::LspMessage::Response { id, result, error } => {
                self.on_response(key, id, result, error);
            }
            // Answer server-to-client requests with null (progress and
            // registration prompts), like the probe verified: unanswered
            // requests can stall analysis.
            L::LspMessage::Request { id, .. } => {
                let reply = match &id {
                    L::RequestId::Number(number) => {
                        serde_json::json!({"jsonrpc":"2.0","id":number,"result":null})
                    }
                    L::RequestId::Text(text) => {
                        serde_json::json!({"jsonrpc":"2.0","id":text,"result":null})
                    }
                };
                if let Some(server) = self.servers.get(key) {
                    let _ = server.inbox.send(L::frame(reply.to_string().as_bytes()));
                }
            }
        }
    }

    fn on_response(
        &mut self,
        key: &ServerKey,
        id: L::RequestId,
        result: Option<serde_json::Value>,
        error: Option<L::ResponseError>,
    ) {
        let L::RequestId::Number(number) = id else {
            return;
        };
        // Init handshake first: only the starting server owns init ids.
        if self
            .servers
            .get(key)
            .is_some_and(|server| server.init_id == Some(number))
        {
            // A failed `initialize` (for example TypeScript's server with
            // no usable `typescript` package) must not count as ready.
            if let Some(error) = error {
                self.shutdown_server(key);
                self.note_failure(key.language);
                self.unavailable.insert(
                    key.language,
                    format!("Language server failed to start: {}", error.message),
                );
                return;
            }
            let Some(server) = self.servers.get_mut(key) else {
                return;
            };
            server.init_id = None;
            server.ready = true;
            let _ = server.inbox.send(L::initialized());
            let inbox = server.inbox.clone();
            // Buffers submitted while starting open now, in order behind
            // `initialized`: a single pre-ready submit is enough, and no
            // further frame is needed to get its `didOpen` out.
            for doc in self.docs.values_mut().filter(|doc| doc.server == *key) {
                if let Some(text) = doc.pending_text.take() {
                    doc.version = doc.version.saturating_add(1);
                    let _ = inbox.send(L::did_open(&doc.uri, doc.language_id, doc.version, &text));
                    doc.opened = true;
                } else {
                    doc.opened = false;
                }
            }
            self.failures.remove(key.language);
            return;
        }
        let pending = self
            .servers
            .get_mut(key)
            .and_then(|server| server.pending.remove(&number));
        match pending {
            Some(Pending::Hover { path, cursor }) => {
                let result = result.unwrap_or(serde_json::Value::Null);
                if let Some(hover) = L::parse_hover(&result) {
                    self.hover = Some(HoverPopup {
                        path,
                        cursor_line: cursor.0,
                        cursor_col: cursor.1,
                        text: hover.text,
                        markdown: hover.markdown,
                        open: true,
                        status: false,
                    });
                }
                // A null answer clears a status note (the request it
                // hung off is done); real answers are never cleared.
                else if self
                    .hover
                    .as_ref()
                    .is_some_and(|popup| popup.status && popup.path == path)
                {
                    self.hover = None;
                }
            }
            Some(Pending::Symbols { path, revision }) => {
                let result = result.unwrap_or(serde_json::Value::Null);
                let parsed = L::parse_document_symbols(&result);
                if parsed.is_empty() {
                    self.symbols_retry.insert(path.clone(), Instant::now());
                } else {
                    self.symbols_retry.remove(&path);
                }
                self.symbols.insert(path.clone(), parsed);
                self.symbols_rev.insert(path, revision);
                self.search_revision = self.search_revision.wrapping_add(1);
            }
            Some(Pending::WorkspaceSymbols { generation }) => {
                if generation == self.workspace_symbol_generation {
                    let hits =
                        L::parse_workspace_symbols(&result.unwrap_or(serde_json::Value::Null));
                    self.workspace_symbols
                        .insert(key.clone(), hits.into_iter().take(200).collect());
                    self.search_revision = self.search_revision.wrapping_add(1);
                }
            }
            Some(Pending::Definition { path }) => {
                let result = result.unwrap_or(serde_json::Value::Null);
                let targets = L::parse_definition(&result);
                if !targets.is_empty() {
                    self.goto = Some(GotoJump {
                        from: path,
                        targets,
                    });
                }
            }
            Some(Pending::References { path }) => {
                let result = result.unwrap_or(serde_json::Value::Null);
                self.references = Some(ReferenceResults {
                    from: path,
                    targets: L::parse_references(&result),
                });
            }
            Some(Pending::Rename {
                path,
                new_name,
                revision,
            }) => {
                let stale = self
                    .docs
                    .get(&path)
                    .is_none_or(|doc| doc.observed != revision);
                let edits = if stale {
                    Err("The file changed while the rename was running. Try again.".into())
                } else if let Some(error) = error {
                    Err(error.message)
                } else {
                    L::parse_rename(&result.unwrap_or(serde_json::Value::Null))
                };
                self.rename = Some(RenameResults {
                    from: path,
                    new_name,
                    edits,
                });
            }
            None => {}
        }
    }

    fn on_server_down(&mut self, key: &ServerKey) {
        // Stray exits (a supervised actor reaped just after a deliberate
        // shutdown removed the entry) are not crashes.
        if self.servers.remove(key).is_none() {
            return;
        }
        self.workspace_symbols.remove(key);
        self.search_revision = self.search_revision.wrapping_add(1);
        self.note_failure(key.language);
        // Docs re-open (and the server respawns, cooldown permitting)
        // on the next sync; pending answers are dead with their ids.
        for doc in self.docs.values_mut().filter(|doc| doc.server == *key) {
            doc.opened = false;
        }
    }

    fn shutdown_server(&mut self, key: &ServerKey) {
        if let Some(mut server) = self.servers.remove(key) {
            let id = server.ids.next_id();
            let _ = server.inbox.send(L::shutdown(&id));
            let _ = server.inbox.send(L::exit());
            if let Some(mut child) = server.child {
                let _ = child.start_kill();
            }
            if let Some(cancel) = server.cancel {
                cancel.cancel();
            }
        }
        self.workspace_symbols.remove(key);
        self.search_revision = self.search_revision.wrapping_add(1);
        for doc in self.docs.values_mut().filter(|doc| doc.server == *key) {
            doc.opened = false;
        }
    }

    /// Forget closed tabs: `didClose` open buffers, drop their state,
    /// and shut down servers nobody needs anymore.
    pub fn prune(&mut self, live: &std::collections::HashSet<PathBuf>) {
        let gone: Vec<PathBuf> = self
            .docs
            .keys()
            .filter(|path| !live.contains(*path))
            .cloned()
            .collect();
        for path in &gone {
            self.close_path(path);
        }
        let needed: std::collections::HashSet<ServerKey> =
            self.docs.values().map(|doc| doc.server.clone()).collect();
        let idle: Vec<ServerKey> = self
            .servers
            .keys()
            .filter(|key| !needed.contains(*key))
            .cloned()
            .collect();
        for key in &idle {
            self.shutdown_server(key);
        }
    }
}

/// Inverse of [`L::server_language`], for the server table.
fn language_of(id: &str) -> Language {
    match id {
        "rust" => Language::Rust,
        "python" => Language::Python,
        "typescript" => Language::TypeScript,
        _ => Language::Plain,
    }
}

/// Workspace root for one file: nearest ancestor holding a language
/// marker (`Cargo.toml`, `pyproject.toml`, …), else nearest with `.git`,
/// else the file's own directory. Language markers win over a nearer
/// `.git` (a nested repo inside a buildable workspace still analyzes
/// with the workspace). Servers use the root for project-wide analysis.
fn project_root(path: &Path, language: Language) -> PathBuf {
    let markers: &[&str] = match language {
        Language::Rust => &["Cargo.toml"],
        Language::Python => &["pyproject.toml", "setup.py", "setup.cfg"],
        Language::TypeScript | Language::JavaScript => {
            &["tsconfig.json", "jsconfig.json", "package.json"]
        }
        _ => &[],
    };
    let mut fallback: Option<PathBuf> = None;
    let mut current = path.parent();
    for _ in 0..20 {
        let Some(dir) = current else {
            break;
        };
        if markers.iter().any(|name| dir.join(name).exists()) {
            return dir.to_path_buf();
        }
        if fallback.is_none() && dir.join(".git").exists() {
            fallback = Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    fallback.unwrap_or_else(|| path.parent().map(Path::to_path_buf).unwrap_or_default())
}

/// Supervisor actor for one server: spawn the child with piped stdio
/// in `root` and pump both directions. Runs on the supervisor's Tokio
/// runtime, so process startup and `tokio::spawn` never touch the GUI
/// thread. A failed `exec` reports [`PumpEvent::SpawnFailed`]; a live
/// child forwards framed bodies and reports [`PumpEvent::Down`] when it
/// exits. Cancellation (deliberate shutdown) kills the child, reaps it,
/// and stays silent, so prunes never count as crashes.
async fn run_server_actor(
    key: ServerKey,
    binary: PathBuf,
    root: PathBuf,
    mut incoming: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    pump: tokio::sync::mpsc::UnboundedSender<PumpEvent>,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let mut child = match tokio::process::Command::new(&binary)
        .args(L::server_args(language_of(key.language)))
        .current_dir(&root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let _ = pump.send(PumpEvent::SpawnFailed {
                key: key.clone(),
                note: format!("Could not start {}: {error:#}", key.language),
            });
            return Ok(());
        }
    };
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let writer = tokio::spawn(async move {
        use tokio::io::AsyncWriteExt as _;
        let mut stdin = stdin;
        while let Some(bytes) = incoming.recv().await {
            let ok = match stdin.as_mut() {
                Some(pipe) => pipe.write_all(&bytes).await.is_ok() && pipe.flush().await.is_ok(),
                None => false,
            };
            if !ok {
                break;
            }
        }
    });
    let reader_pump = pump.clone();
    let reader_key = key.clone();
    let reader = tokio::spawn(async move {
        use tokio::io::AsyncReadExt as _;
        let mut stdout = stdout;
        let mut decoder = L::FrameDecoder::default();
        let mut chunk = [0u8; 8192];
        while let Some(pipe) = stdout.as_mut() {
            match pipe.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut stop = false;
                    for body in decoder.feed(chunk.get(..n).unwrap_or(&[])) {
                        if reader_pump
                            .send(PumpEvent::Body {
                                key: reader_key.clone(),
                                body,
                            })
                            .is_err()
                        {
                            stop = true;
                            break;
                        }
                    }
                    if stop {
                        break;
                    }
                }
            }
        }
    });
    // The child outlives the pumps on crash: wait for the process
    // itself, then tear the pumps down.
    let exited = tokio::select! {
        result = child.wait() => {
            let _ = result;
            true
        }
        () = cancel.cancelled() => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            false
        }
    };
    writer.abort();
    reader.abort();
    if exited && !cancel.is_cancelled() {
        let _ = pump.send(PumpEvent::Down { key });
    }
    Ok(())
}

/// Spawn one server with piped stdio in `root`, handshake it, and pump
/// both directions on background tasks. Needs a running tokio runtime;
/// only the no-services test path uses this (tests use `#[tokio::test]`).
/// The GUI path goes through [`LspManager::spawn_supervised`] instead.
fn spawn_server_direct(
    key: &ServerKey,
    binary: &Path,
    pump: tokio::sync::mpsc::UnboundedSender<PumpEvent>,
) -> Result<Server, String> {
    let mut child = tokio::process::Command::new(binary)
        .args(L::server_args(language_of(key.language)))
        .current_dir(&key.root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|error| format!("Could not start {}: {error:#}", key.language))?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let (inbox, mut incoming) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(async move {
        use tokio::io::AsyncWriteExt as _;
        let mut stdin = stdin;
        while let Some(bytes) = incoming.recv().await {
            let ok = match stdin.as_mut() {
                Some(pipe) => pipe.write_all(&bytes).await.is_ok() && pipe.flush().await.is_ok(),
                None => false,
            };
            if !ok {
                break;
            }
        }
    });
    let reader_pump = pump.clone();
    let reader_key = key.clone();
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt as _;
        let mut stdout = stdout;
        let mut decoder = L::FrameDecoder::default();
        let mut chunk = [0u8; 8192];
        while let Some(pipe) = stdout.as_mut() {
            match pipe.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut stop = false;
                    for body in decoder.feed(chunk.get(..n).unwrap_or(&[])) {
                        if reader_pump
                            .send(PumpEvent::Body {
                                key: reader_key.clone(),
                                body,
                            })
                            .is_err()
                        {
                            stop = true;
                            break;
                        }
                    }
                    if stop {
                        break;
                    }
                }
            }
        }
        let _ = pump.send(PumpEvent::Down { key: reader_key });
    });
    let mut server = Server {
        inbox,
        child: Some(child),
        cancel: None,
        ids: L::IdGen::default(),
        pending: HashMap::new(),
        init_id: None,
        ready: false,
    };
    let L::RequestId::Number(init) = server.ids.next_id() else {
        return Err(format!("Could not start {}", key.language));
    };
    let root = L::file_uri(&key.root).ok_or_else(|| format!("No file URI for {}", key.language))?;
    let bytes = L::initialize(&L::RequestId::Number(init), std::process::id(), &root);
    if server.inbox.send(bytes).is_err() {
        return Err(format!("{} exited immediately", key.language));
    }
    server.init_id = Some(init);
    Ok(server)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_prefers_language_marker_over_git() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::create_dir_all(root.join("inner/src")).expect("mkdir");
        std::fs::write(root.join("Cargo.toml"), "[package]").expect("manifest");
        std::fs::create_dir_all(root.join("inner/.git")).expect("git");
        let file = root.join("inner/src/main.rs");
        assert_eq!(project_root(&file, Language::Rust), root);
    }

    #[test]
    fn root_falls_back_to_parent_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("lonely.py");
        assert_eq!(project_root(&file, Language::Python), dir.path());
    }

    #[test]
    fn unsupported_language_never_syncs() {
        let mut manager = LspManager::new();
        let path = crate::test_path("/tmp/note.md");
        assert!(!manager.sync_doc(&path, Language::Markdown, 1));
        assert!(manager.row_marks(&path).is_empty());
        assert!(manager.problems_for(&path).is_empty());
    }

    #[test]
    fn marks_keep_worst_severity_per_row_sorted() {
        let mut manager = LspManager::new();
        let path = crate::test_path("/tmp/main.rs");
        manager.diagnostics.insert(
            path.clone(),
            vec![
                L::Diagnostic {
                    range: L::Range {
                        start: L::Position {
                            line: 9,
                            character: 0,
                        },
                        end: L::Position {
                            line: 9,
                            character: 1,
                        },
                    },
                    severity: Some(L::Severity::Warning),
                    message: "w".into(),
                },
                L::Diagnostic {
                    range: L::Range {
                        start: L::Position {
                            line: 9,
                            character: 5,
                        },
                        end: L::Position {
                            line: 9,
                            character: 6,
                        },
                    },
                    severity: Some(L::Severity::Error),
                    message: "e".into(),
                },
                L::Diagnostic {
                    range: L::Range {
                        start: L::Position {
                            line: 2,
                            character: 0,
                        },
                        end: L::Position {
                            line: 2,
                            character: 1,
                        },
                    },
                    severity: None,
                    message: "plain".into(),
                },
            ],
        );
        let marks = manager.row_marks(&path);
        assert_eq!(marks.len(), 2);
        assert_eq!(marks[0].line, 2);
        assert_eq!(marks[0].severity, L::Severity::Hint);
        assert_eq!(marks[1].line, 9);
        assert_eq!(marks[1].severity, L::Severity::Error);
        // Status-bar counts: one error, one warning, one unseverity'd.
        assert_eq!(manager.problem_counts(), (1, 1, 3));
        assert!(manager.tracked(&path) || !manager.problems_for(&path).is_empty());
    }

    #[test]
    fn problem_counts_start_empty_and_untracked() {
        let manager = LspManager::new();
        assert_eq!(manager.problem_counts(), (0, 0, 0));
        assert!(!manager.tracked(std::path::Path::new("/tmp/never-opened.rs")));
        assert_eq!(
            manager.server_note(std::path::Path::new("/tmp/never-opened.rs")),
            None
        );
    }

    #[test]
    fn debounce_fires_without_further_edits() {
        let mut manager = LspManager::new();
        let path = crate::test_path("/tmp/debounced.rs");
        tracked(&mut manager, &path);
        // Nothing new to send for the already-submitted revision.
        assert!(!manager.sync_doc(&path, Language::Rust, 1));
        // A fresh edit starts the delay...
        assert!(!manager.sync_doc(&path, Language::Rust, 2));
        // ...and frames with no further edits must not extend it:
        // backdated past the delay, the same revision submits.
        manager.docs.get_mut(&path).expect("doc").last_change = Instant::now()
            .checked_sub(SYNC_DEBOUNCE + Duration::from_secs(1))
            .expect("backdate");
        assert!(manager.sync_doc(&path, Language::Rust, 2));
    }

    /// Server startup and stdio must not need a Tokio runtime on the
    /// calling thread: the GUI thread owns none, and spawning from it
    /// panicked. Supervised startup only passes channels, so spawning a
    /// (promptly exiting) server here must complete its lifecycle with
    /// events, never a panic.
    #[cfg(unix)]
    #[test]
    fn supervised_spawn_needs_no_caller_runtime() {
        assert!(
            tokio::runtime::Handle::try_current().is_err(),
            "regression needs a runtime-less thread"
        );
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = eframe::egui::Context::default();
        let (tx, _rx) = std::sync::mpsc::channel::<crate::Update>();
        let paths = terminator_core::Paths::at(dir.path().to_owned());
        let (services, _owner) =
            crate::gui_services::Services::new(paths, ctx, tx).expect("services");
        let mut manager = LspManager::new();
        manager.set_services(services);
        let key = ServerKey {
            language: "rust",
            root: dir.path().to_owned(),
        };
        // /bin/true can exit before initialize is queued. Both an immediate
        // None and a later Down event are valid; neither needs a caller runtime.
        let _ = manager.spawn_supervised(&key, Path::new("/bin/true"));
        let start = Instant::now();
        while manager.servers.contains_key(&key) {
            manager.poll();
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "exited server is reaped"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        manager.poll();
        assert_eq!(
            manager.failures.get(key.language),
            Some(&1),
            "the exited server is recorded exactly once"
        );
    }

    #[test]
    fn server_resources_scope_by_root() {
        let first = ServerKey {
            language: "rust",
            root: PathBuf::from("/proj-a"),
        };
        let second = ServerKey {
            language: "rust",
            root: PathBuf::from("/proj-b"),
        };
        assert_ne!(server_resource(&first), server_resource(&second));
        assert_eq!(server_resource(&first), server_resource(&first.clone()));
    }

    /// Two projects, one language: the second server must admit
    /// alongside the first. The supervisor runs a single active task
    /// per key, so a language-only key parks the second spawn forever.
    #[cfg(unix)]
    #[test]
    fn second_project_server_starts_alongside_first() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ctx = eframe::egui::Context::default();
        let (tx, _rx) = std::sync::mpsc::channel::<crate::Update>();
        let paths = terminator_core::Paths::at(dir.path().to_owned());
        let (services, _owner) =
            crate::gui_services::Services::new(paths, ctx, tx).expect("services");
        let mut manager = LspManager::new();
        manager.set_services(services.clone());
        // `/bin/cat` stays alive on its piped stdin, so both actors are
        // long-lived: without root-scoped keys the second never admits.
        // (The supervisor's own processes actor is already admitted, so
        // the first server brings the count to two, the second to three.)
        let first = ServerKey {
            language: "rust",
            root: dir.path().join("proj-a"),
        };
        std::fs::create_dir_all(&first.root).expect("root");
        manager
            .spawn_supervised(&first, Path::new("/bin/cat"))
            .expect("spawn accepted");
        let start = Instant::now();
        while services.handle().diagnostics().actors < 2 {
            manager.poll();
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "first server admits"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let second = ServerKey {
            language: "rust",
            root: dir.path().join("proj-b"),
        };
        std::fs::create_dir_all(&second.root).expect("root");
        manager
            .spawn_supervised(&second, Path::new("/bin/cat"))
            .expect("spawn accepted");
        let start = Instant::now();
        while services.handle().diagnostics().actors < 3 {
            manager.poll();
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "second project's server admits alongside the first"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        manager.prune(&std::collections::HashSet::new());
        manager.poll();
    }

    /// Text submitted while the server starts must open at handshake
    /// completion: `initialized` first, then the parked `didOpen`, with
    /// no further submit needed.
    #[test]
    fn parked_text_opens_at_handshake_completion() {
        let mut manager = LspManager::new();
        let path = crate::test_path("/tmp/pending.rs");
        tracked(&mut manager, &path);
        let key = manager.docs.get(&path).expect("doc").server.clone();
        let (inbox, mut outbox) = tokio::sync::mpsc::unbounded_channel();
        manager.servers.insert(
            key.clone(),
            Server {
                inbox,
                child: None,
                cancel: None,
                ids: L::IdGen::default(),
                pending: HashMap::new(),
                init_id: Some(7),
                ready: false,
            },
        );
        let doc = manager.docs.get_mut(&path).expect("doc");
        doc.opened = false;
        doc.version = 0;
        doc.pending_text = Some("fn main() {}".into());
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 7,
            "result": {"capabilities": {}},
        })
        .to_string();
        manager.on_body(&key, body.as_bytes());
        let doc = manager.docs.get(&path).expect("doc");
        assert!(doc.opened);
        assert!(doc.pending_text.is_none());
        assert!(manager.servers.get(&key).is_some_and(|server| server.ready));
        // Inbox traffic is framed for the wire: decode before parsing.
        let mut decoder = L::FrameDecoder::default();
        let framed = outbox.try_recv().expect("initialized");
        let bodies = decoder.feed(&framed);
        assert_eq!(bodies.len(), 1);
        let first: serde_json::Value = serde_json::from_slice(&bodies[0]).expect("json");
        assert_eq!(
            first.get("method").and_then(|m| m.as_str()),
            Some("initialized")
        );
        let framed = outbox.try_recv().expect("didOpen");
        let bodies = decoder.feed(&framed);
        assert_eq!(bodies.len(), 1);
        let second: serde_json::Value = serde_json::from_slice(&bodies[0]).expect("json");
        assert_eq!(
            second.get("method").and_then(|m| m.as_str()),
            Some("textDocument/didOpen")
        );
        let document = second
            .get("params")
            .and_then(|params| params.get("textDocument"))
            .expect("document");
        assert_eq!(
            document.get("text").and_then(|text| text.as_str()),
            Some("fn main() {}")
        );
        assert_eq!(document.get("version").and_then(|v| v.as_i64()), Some(1));
        assert!(outbox.try_recv().is_err());
    }

    fn tracked(manager: &mut LspManager, path: &Path) {
        let root = project_root(path, Language::Rust);
        manager.docs.insert(
            path.to_owned(),
            OpenDoc {
                uri: L::file_uri(path).expect("uri"),
                version: 1,
                server: ServerKey {
                    language: "rust",
                    root,
                },
                language_id: "rust",
                submitted: 1,
                observed: 1,
                last_change: Instant::now(),
                opened: true,
                pending_text: None,
            },
        );
    }

    #[test]
    fn rename_response_is_preview_only_and_rejects_stale_buffer() {
        let mut manager = LspManager::new();
        let path = crate::test_path("/tmp/rename.rs");
        tracked(&mut manager, &path);
        let key = manager.docs.get(&path).unwrap().server.clone();
        let (inbox, mut outbox) = tokio::sync::mpsc::unbounded_channel();
        manager.servers.insert(
            key.clone(),
            Server {
                inbox,
                child: None,
                cancel: None,
                ids: L::IdGen::default(),
                pending: HashMap::new(),
                init_id: None,
                ready: true,
            },
        );
        let at = L::Position {
            line: 0,
            character: 4,
        };
        manager.request_rename(&path, at, "bar").unwrap();
        let mut decoder = L::FrameDecoder::default();
        let body = decoder.feed(&outbox.try_recv().unwrap()).remove(0);
        let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(request["method"], "textDocument/rename");
        assert_eq!(request["params"]["newName"], "bar");
        let uri = L::file_uri(&path).unwrap().to_string();
        let response = serde_json::json!({
            "jsonrpc": "2.0", "id": request["id"],
            "result": { "changes": {
                (uri): [{
                    "range": {
                        "start": {"line": 0, "character": 4},
                        "end": {"line": 0, "character": 7}
                    },
                    "newText": "bar"
                }]
            }}
        });
        manager.on_body(&key, response.to_string().as_bytes());
        let result = manager.take_rename().unwrap();
        assert_eq!(result.new_name, "bar");
        assert_eq!(result.edits.unwrap()[0].edits[0].new_text, "bar");

        manager.request_rename(&path, at, "baz").unwrap();
        manager.docs.get_mut(&path).unwrap().observed = 2;
        let body = decoder.feed(&outbox.try_recv().unwrap()).remove(0);
        let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let response = serde_json::json!({"jsonrpc": "2.0", "id": request["id"], "result": null});
        manager.on_body(&key, response.to_string().as_bytes());
        assert!(manager.take_rename().unwrap().edits.is_err());
    }

    #[test]
    fn workspace_symbol_results_follow_the_latest_query() {
        let mut manager = LspManager::new();
        let path = crate::test_path("/tmp/symbols.rs");
        tracked(&mut manager, &path);
        let key = manager.docs.get(&path).unwrap().server.clone();
        let (inbox, mut outbox) = tokio::sync::mpsc::unbounded_channel();
        manager.servers.insert(
            key.clone(),
            Server {
                inbox,
                child: None,
                cancel: None,
                ids: L::IdGen::default(),
                pending: HashMap::new(),
                init_id: None,
                ready: true,
            },
        );
        manager.request_scoped_symbols("old", None);
        assert!(manager.workspace_symbols_pending());
        let mut decoder = L::FrameDecoder::default();
        let request = decoder.feed(&outbox.try_recv().unwrap()).remove(0);
        let request: serde_json::Value = serde_json::from_slice(&request).unwrap();
        assert_eq!(request["method"], "workspace/symbol");
        manager.request_scoped_symbols("new", None);
        let newer = decoder.feed(&outbox.try_recv().unwrap()).remove(0);
        let newer: serde_json::Value = serde_json::from_slice(&newer).unwrap();
        let location = serde_json::json!({
            "uri": L::file_uri(&path).unwrap().as_str(),
            "range": {"start": {"line": 2, "character": 0}, "end": {"line": 2, "character": 3}}
        });
        let old = serde_json::json!({
            "jsonrpc": "2.0", "id": request["id"],
            "result": [{"name": "old", "kind": 12, "location": location.clone()}]
        });
        manager.on_body(&key, old.to_string().as_bytes());
        assert!(manager.workspace_symbols().is_empty());
        let new = serde_json::json!({
            "jsonrpc": "2.0", "id": newer["id"],
            "result": [{"name": "new", "kind": 12, "location": location}]
        });
        manager.on_body(&key, new.to_string().as_bytes());
        assert_eq!(manager.workspace_symbols()[0].name, "new");
        assert!(!manager.workspace_symbols_pending());
    }

    #[test]
    fn diagnostics_route_to_tracked_docs_only() {
        let mut manager = LspManager::new();
        let path = crate::test_path("/tmp/routed.rs");
        tracked(&mut manager, &path);
        let key = ServerKey {
            language: "rust",
            root: project_root(&path, Language::Rust),
        };
        let params = serde_json::json!({
            "uri": L::file_uri(&path).unwrap().as_str(),
            "diagnostics": [{
                "range": {"start": {"line": 0, "character": 0},
                          "end": {"line": 0, "character": 1}},
                "severity": 1,
                "message": "boom",
            }],
        });
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": params,
        })
        .to_string();
        manager.on_body(&key, body.as_bytes());
        assert_eq!(manager.problems_for(&path).len(), 1);
        // Unknown files never accumulate state.
        let unknown = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {"uri": L::file_uri(&crate::test_path("/tmp/ghost.rs")).unwrap().as_str(), "diagnostics": []},
        })
        .to_string();
        manager.on_body(&key, unknown.as_bytes());
        assert!(
            manager
                .problems_for(&crate::test_path("/tmp/ghost.rs"))
                .is_empty()
        );
    }

    #[test]
    fn prune_closes_docs_and_clears_state() {
        let mut manager = LspManager::new();
        let path = crate::test_path("/tmp/gone.rs");
        tracked(&mut manager, &path);
        manager.diagnostics.insert(
            path.clone(),
            vec![L::Diagnostic {
                range: L::Range {
                    start: L::Position {
                        line: 0,
                        character: 0,
                    },
                    end: L::Position {
                        line: 0,
                        character: 1,
                    },
                },
                severity: None,
                message: "x".into(),
            }],
        );
        manager.prune(&std::collections::HashSet::new());
        assert!(manager.problems_for(&path).is_empty());
        assert!(manager.row_marks(&path).is_empty());
    }

    /// Full round trip against the real TypeScript server when
    /// installed; self-skips otherwise.
    #[tokio::test]
    async fn live_typescript_server_diagnostics_and_symbols() {
        if terminator_core::find_executable("typescript-language-server").is_none() {
            eprintln!("SKIP: typescript-language-server not installed");
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("proj");
        std::fs::create_dir_all(&root).expect("mkdir");
        std::fs::write(
            root.join("package.json"),
            "{\"name\":\"proj\",\"version\":\"0.0.0\"}",
        )
        .expect("package");
        std::fs::write(
            root.join("tsconfig.json"),
            "{\"compilerOptions\":{\"strict\":true}}",
        )
        .expect("tsconfig");
        let source = "export const bad: number = \"oops\";\nexport function ok() { return 1; }\n";
        let file = root.join("index.ts");
        std::fs::write(&file, source).expect("source");

        let mut manager = LspManager::new();
        assert!(manager.sync_doc(&file, Language::TypeScript, 1));
        manager.submit_text(&file, Language::TypeScript, 1, source);
        let start = Instant::now();
        while manager.problems_for(&file).is_empty() {
            if let Some(note) = manager.server_note(&file)
                && note.starts_with("Language server failed to start")
            {
                eprintln!("SKIP: {note}");
                return;
            }
            assert!(
                start.elapsed() < Duration::from_mins(1),
                "no diagnostics; note={:?}",
                manager.server_note(&file)
            );
            manager.poll();
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let problems = manager.problems_for(&file);
        assert!(
            problems
                .iter()
                .any(|item| item.severity == Some(L::Severity::Error)),
            "type error reported: {problems:?}"
        );

        let start = Instant::now();
        while manager.symbols_for(&file).is_empty() {
            assert!(start.elapsed() < Duration::from_secs(30), "no symbols");
            manager.request_symbols(&file);
            manager.poll();
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let names: Vec<String> = manager
            .symbols_for(&file)
            .into_iter()
            .map(|symbol| symbol.name)
            .collect();
        assert!(names.iter().any(|name| name == "ok"), "{names:?}");
        manager.prune(&std::collections::HashSet::new());
    }

    /// Full round trip against the real rust-analyzer when installed;
    /// self-skips otherwise (live-provider verification, not CI-gated).
    #[tokio::test]
    async fn live_rust_analyzer_diagnostics() {
        let path_env = std::env::var_os("PATH")
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        if L::find_server(&["rust-analyzer"], &path_env).is_none() {
            eprintln!("SKIP: rust-analyzer not on PATH");
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("proj");
        std::fs::create_dir_all(root.join("src")).expect("mkdir");
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"proj\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
        )
        .expect("manifest");
        let main = "fn main() {\n    let bad: i32 = \"oops\";\n}\n";
        let file = root.join("src/main.rs");
        std::fs::write(&file, main).expect("main");

        let mut manager = LspManager::new();
        assert!(manager.sync_doc(&file, Language::Rust, 1));
        manager.submit_text(&file, Language::Rust, 1, main);
        // Handshake + analysis need seconds; poll until diagnostics land.
        let start = Instant::now();
        while manager.problems_for(&file).is_empty() {
            assert!(start.elapsed() < Duration::from_secs(90), "no diagnostics");
            manager.poll();
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let problems = manager.problems_for(&file);
        // Error severity at the `bad` binding (line 1), not message
        // text: wording varies across server versions, but a type error
        // there is the point of the fixture.
        assert!(
            problems.iter().any(|item| item.severity == Some(L::Severity::Error)
                && item.range.start.line == 1),
            "type error reported at line 1: {:?}",
            problems
                .iter()
                .map(|item| (&item.message, item.range.start.line))
                .collect::<Vec<_>>()
        );
        assert!(!manager.row_marks(&file).is_empty());
        // Hover the `bad` binding (line 1, chars 8..11), retrying the
        // request like a user pressing K again: early answers are null
        // until analysis settles.
        let start = Instant::now();
        let mut hover = None;
        while hover.is_none() {
            assert!(start.elapsed() < Duration::from_mins(1), "no hover");
            manager.request_hover(
                &file,
                L::Position {
                    line: 1,
                    character: 9,
                },
                (1, 8),
            );
            for _ in 0..10 {
                manager.poll();
                hover = manager.hover_popup(&file, (1, 8));
                if hover.is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        assert!(
            hover.as_ref().is_some_and(|(text, _)| text.contains("i32")),
            "hover: {hover:?}"
        );
        manager.prune(&std::collections::HashSet::new());
    }

    /// Fake ready server so symbol requests have an inbox to land in.
    fn ready_server(
        manager: &mut LspManager,
        path: &Path,
    ) -> (ServerKey, tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>) {
        let key = manager.docs.get(path).expect("doc").server.clone();
        let (inbox, outbox) = tokio::sync::mpsc::unbounded_channel();
        manager.servers.insert(
            key.clone(),
            Server {
                inbox,
                child: None,
                cancel: None,
                ids: L::IdGen::default(),
                pending: HashMap::new(),
                init_id: None,
                ready: true,
            },
        );
        (key, outbox)
    }

    #[test]
    fn empty_outline_retries_on_a_delay_instead_of_caching_forever() {
        let mut manager = LspManager::new();
        let path = crate::test_path("/tmp/outline.rs");
        tracked(&mut manager, &path);
        let (key, mut sent) = ready_server(&mut manager, &path);

        manager.request_symbols(&path);
        assert!(sent.try_recv().is_ok(), "first request goes out");
        let id = manager
            .servers
            .get(&key)
            .expect("server")
            .pending
            .keys()
            .copied()
            .next()
            .expect("pending symbols");
        manager.on_response(
            &key,
            L::RequestId::Number(id),
            Some(serde_json::json!([])),
            None,
        );
        assert!(manager.symbols_for(&path).is_empty());

        // Immediately after an empty answer the gate holds...
        manager.request_symbols(&path);
        assert!(sent.try_recv().is_err(), "no per-frame spam while indexing");
        // ...and releases once the delay passed.
        manager.symbols_retry.insert(
            path.clone(),
            Instant::now().checked_sub(SYMBOLS_RETRY).expect("backdate"),
        );
        manager.request_symbols(&path);
        assert!(sent.try_recv().is_ok(), "indexing retry asks again");
    }

    #[test]
    fn restart_clears_unavailable_and_reopens_documents() {
        let mut manager = LspManager::new();
        let path = crate::test_path("/tmp/retry.rs");
        tracked(&mut manager, &path);
        let _ = ready_server(&mut manager, &path);
        manager
            .unavailable
            .insert("rust", "No language server".into());
        manager.failures.insert("rust", MAX_CRASHES);

        manager.restart(&path);

        assert!(manager.unavailable.is_empty());
        assert!(manager.failures.is_empty());
        assert!(!manager.docs.get(&path).expect("doc").opened);
    }

    #[test]
    fn failed_initialize_reports_the_server_message() {
        let mut manager = LspManager::new();
        let path = crate::test_path("/tmp/app.ts");
        tracked(&mut manager, &path);
        let (key, _sent) = ready_server(&mut manager, &path);
        manager.servers.get_mut(&key).expect("server").init_id = Some(7);

        manager.on_response(
            &key,
            L::RequestId::Number(7),
            None,
            Some(L::ResponseError {
                code: -32603,
                message: "Could not find a valid TypeScript installation.".into(),
            }),
        );

        let note = manager.server_note(&path).expect("note");
        assert!(note.contains("Could not find a valid TypeScript"), "{note}");
        assert!(!manager.servers.contains_key(&key));
    }

    #[test]
    fn saved_server_path_restarts_only_that_language() {
        let mut manager = LspManager::new();
        let path = crate::test_path("/tmp/live.rs");
        tracked(&mut manager, &path);
        let (key, _sent) = ready_server(&mut manager, &path);
        let mut overrides = std::collections::BTreeMap::new();
        overrides.insert("rust".to_owned(), " /opt/rust-analyzer ".to_owned());

        manager.set_server_overrides(&overrides);

        assert!(!manager.servers.contains_key(&key), "rust server restarts");
        assert!(!manager.docs.get(&path).expect("doc").opened);
        assert_eq!(
            manager.overrides.get("rust").map(String::as_str),
            Some("/opt/rust-analyzer"),
            "path is trimmed"
        );

        // Unrelated settings saves leave the live server alone.
        let (key, _sent) = ready_server(&mut manager, &path);
        manager.set_server_overrides(&overrides);
        assert!(manager.servers.contains_key(&key));
    }
}
