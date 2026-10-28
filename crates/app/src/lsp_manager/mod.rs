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

pub(crate) struct RequestHierarchyChildrenInput<'a> {
    pub path: &'a Path,
    pub item: &'a serde_json::Value,
    pub kind: L::HierarchyKind,
    pub direction: L::HierarchyDirection,
    pub generation: u64,
    pub node: usize,
}

mod docs;
mod edits;
mod navigation;
mod problems;
mod symbols;
pub(crate) use docs::SubmitTextInput;
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
    InlayHints {
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
    Signature {
        path: PathBuf,
        generation: u64,
    },
    Format {
        path: PathBuf,
        revision: u64,
    },
    CodeActions {
        path: PathBuf,
        generation: u64,
    },
    Resolve {
        path: PathBuf,
    },
    /// Hierarchy roots (`node: None`) or one node's children.
    Hierarchy {
        generation: u64,
        node: Option<usize>,
    },
}

/// A call / type hierarchy answer, taken by the hierarchy window.
#[derive(Debug)]
pub enum HierarchyAnswer {
    Roots {
        generation: u64,
        items: Vec<L::HierarchyItem>,
    },
    Children {
        generation: u64,
        node: usize,
        items: Vec<L::HierarchyItem>,
    },
}

/// Answers for formatting and code actions, taken by the editor.
#[derive(Default)]
pub struct EditAnswers {
    /// `(path, buffer revision formatted, edits)`.
    pub format: Option<(PathBuf, u64, Vec<L::RenameTextEdit>)>,
    /// `(path, generation, actions)`.
    pub actions: Option<(PathBuf, u64, Vec<L::CodeActionItem>)>,
    /// `(path, resolved action)` for an action picked without an edit.
    pub resolved: Option<(PathBuf, serde_json::Value)>,
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
    /// The server's `initialize` capabilities (`Null` until it answers).
    capabilities: serde_json::Value,
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
    pub edit_answers: EditAnswers,
    /// Hierarchy answers in arrival order (see [`HierarchyAnswer`]).
    pub hierarchy_answers: Vec<HierarchyAnswer>,
    hierarchy_generation: u64,
    /// Latest signature-help answer: `(path, generation, signature)`.
    signature: Option<(PathBuf, u64, Option<L::SignatureHelp>)>,
    rename: Option<RenameResults>,
    symbols: std::collections::HashMap<PathBuf, std::sync::Arc<[L::Symbol]>>,
    symbols_rev: std::collections::HashMap<PathBuf, u64>,
    /// Last empty outline answer per path (indexing retry gate).
    symbols_retry: HashMap<PathBuf, Instant>,
    /// Inlay hints per path, with the buffer revision they describe.
    inlay: HashMap<PathBuf, (u64, std::sync::Arc<[L::InlayHint]>)>,
    /// Last empty or failed hints answer per path (indexing retry gate).
    inlay_retry: HashMap<PathBuf, Instant>,
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
            signature: None,
            edit_answers: EditAnswers::default(),
            hierarchy_answers: Vec::new(),
            hierarchy_generation: 0,
            rename: None,
            symbols: std::collections::HashMap::new(),
            symbols_rev: std::collections::HashMap::new(),
            symbols_retry: HashMap::new(),
            inlay: HashMap::new(),
            inlay_retry: HashMap::new(),
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
            self.inlay.clear();
            self.inlay_retry.clear();
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
