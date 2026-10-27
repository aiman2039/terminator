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

mod docs;
mod responses;
mod supervisor;

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
    Completion {
        path: PathBuf,
        generation: u64,
    },
}

/// Completion proposals for one request (`generation` drops stale ones).
pub struct CompletionResults {
    pub path: PathBuf,
    pub generation: u64,
    pub items: Vec<L::CompletionItem>,
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
    completion: Option<CompletionResults>,
    completion_generation: u64,
    rename: Option<RenameResults>,
    symbols: std::collections::HashMap<PathBuf, std::sync::Arc<[L::Symbol]>>,
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
            completion: None,
            completion_generation: 0,
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
            let empty = self
                .symbols
                .get(path)
                .is_none_or(|symbols| symbols.is_empty());
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

    /// Seed a cached outline, standing in for a server answer.
    #[cfg(test)]
    pub fn set_symbols_for_test(&mut self, path: &Path, symbols: Vec<L::Symbol>) {
        self.symbols.insert(path.to_owned(), symbols.into());
    }

    /// Cached outline for `path`, in document order (depth-tagged).
    /// Shared, so per-frame callers clone a pointer, not the outline.
    #[must_use]
    pub fn symbols_for(&self, path: &Path) -> std::sync::Arc<[L::Symbol]> {
        self.symbols
            .get(path)
            .cloned()
            .unwrap_or_else(|| std::sync::Arc::from(Vec::new()))
    }

    /// Request fresh document outlines for every open native editor. The
    /// per-document cache and pending guard keep repeated calls cheap.
    pub fn request_open_document_symbols(&mut self) {
        let paths: Vec<PathBuf> = self.docs.keys().cloned().collect();
        for path in paths {
            self.request_symbols(&path);
        }
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

    /// Ask for completions at `position`. Returns the request's generation
    /// (a later request supersedes it), or `None` when no ready server
    /// tracks the file.
    pub fn request_completion(&mut self, path: &Path, position: L::Position) -> Option<u64> {
        let doc = self.docs.get(path)?;
        let (key, uri) = (doc.server.clone(), doc.uri.clone());
        let server = self.servers.get_mut(&key)?;
        if !server.ready {
            return None;
        }
        let L::RequestId::Number(number) = server.ids.next_id() else {
            return None;
        };
        self.completion_generation = self.completion_generation.wrapping_add(1);
        let generation = self.completion_generation;
        let _ = server
            .inbox
            .send(L::completion(&L::RequestId::Number(number), &uri, position));
        server.pending.insert(
            number,
            Pending::Completion {
                path: path.to_owned(),
                generation,
            },
        );
        Some(generation)
    }

    /// Latest completion answer, if one arrived for the newest request.
    pub fn take_completion(&mut self) -> Option<CompletionResults> {
        self.completion.take()
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

#[cfg(test)]
mod tests;
