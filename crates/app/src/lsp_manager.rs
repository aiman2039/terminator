//! GUI-owned language servers for native editor tabs.
//!
//! The engine stays I/O-free: this manager spawns one server per
//! (language, project root), pumps its stdio on tokio tasks, and keeps
//! diagnostics plus pending hover/goto answers for the views to poll.
//! Servers die with the GUI (stdin EOF on task drop, `start_kill` on
//! prune); a missing binary or a crash loop surfaces as a hover message,
//! never a modal error. Only Rust and Python have servers (see
//! `terminator_native_edit::lsp::server_candidates`); every other
//! language edits exactly as before, without LSP.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use terminator_native_edit::highlight::Language;
use terminator_native_edit::lsp as L;
use terminator_native_edit::view::DiagnosticMark;

/// Full-text sync at most this often after the last edit: diagnostics
/// lag typing by ~1s instead of hammering the server per keystroke.
const SYNC_DEBOUNCE: Duration = Duration::from_secs(1);
/// Cooldown between spawn attempts for one server (crash loops degrade
/// to retries, then to unavailable).
const SPAWN_COOLDOWN: Duration = Duration::from_secs(10);
/// Consecutive crashes before a server counts as unavailable.
const MAX_CRASHES: u32 = 3;

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct ServerKey {
    language: &'static str,
    root: PathBuf,
}

struct OpenDoc {
    uri: url::Url,
    version: i32,
    server: ServerKey,
    /// Buffer revision of the last submitted text.
    submitted: u64,
    last_change: Instant,
    opened: bool,
}

enum Pending {
    Hover {
        path: PathBuf,
        cursor: (usize, usize),
    },
    Definition {
        path: PathBuf,
    },
}

struct Server {
    inbox: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    child: tokio::process::Child,
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

enum PumpEvent {
    Body { key: ServerKey, body: Vec<u8> },
    Down { key: ServerKey },
}

pub struct LspManager {
    servers: HashMap<ServerKey, Server>,
    docs: HashMap<PathBuf, OpenDoc>,
    diagnostics: HashMap<PathBuf, Vec<L::Diagnostic>>,
    hover: Option<HoverPopup>,
    goto: Option<GotoJump>,
    unavailable: HashMap<&'static str, String>,
    /// Consecutive crashes per language (reset on a good handshake).
    failures: HashMap<&'static str, u32>,
    /// Last crash/spawn-failure time per language (respawn cooldown).
    cooled: HashMap<&'static str, Instant>,
    pump_tx: tokio::sync::mpsc::UnboundedSender<PumpEvent>,
    pump_rx: tokio::sync::mpsc::UnboundedReceiver<PumpEvent>,
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
            unavailable: HashMap::new(),
            failures: HashMap::new(),
            cooled: HashMap::new(),
            pump_tx,
            pump_rx,
        }
    }

    /// Drain pump events: route diagnostics, answers, and server deaths.
    /// Call once per frame before any per-doc sync.
    pub fn poll(&mut self) {
        while let Ok(event) = self.pump_rx.try_recv() {
            match event {
                PumpEvent::Body { key, body } => self.on_body(&key, &body),
                PumpEvent::Down { key } => self.on_server_down(&key),
            }
        }
    }

    /// Per-frame sync for one loaded buffer. Returns true when the
    /// caller should clone the text and hand it to [`submit_text`]:
    /// unknown buffers, unopened buffers with a ready (or missing)
    /// server, and edits older than the debounce. Unsupported languages
    /// untrack (and close) silently.
    pub fn sync_doc(&mut self, path: &Path, language: Language, revision: u64) -> bool {
        if L::language_id(language).is_none() {
            self.close_path(path);
            return false;
        }
        match self.docs.get_mut(path) {
            None => true,
            Some(doc) => {
                if doc.submitted != revision {
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
    /// revision (latest text wins); sends `didOpen`/`didChange` only
    /// through a ready server, so a slow start never blocks typing.
    pub fn submit_text(&mut self, path: &Path, language: Language, revision: u64, text: &str) {
        let Some(language_id) = L::language_id(language) else {
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
            submitted: revision,
            last_change: now,
            opened: false,
        });
        if doc.server != key {
            doc.server = key.clone();
            doc.opened = false;
        }
        let was_submitted = doc.submitted;
        doc.submitted = revision;
        if !self.servers.get(&key).is_some_and(|server| server.ready) {
            let _ = self.ensure_server(&key);
            return;
        }
        let inbox = self.servers.get(&key).map(|server| server.inbox.clone());
        let Some(inbox) = inbox else {
            return;
        };
        let Some(doc) = self.docs.get_mut(path) else {
            return;
        };
        if !doc.opened {
            doc.version = doc.version.saturating_add(1);
            let bytes = L::did_open(&doc.uri, language_id, doc.version, text);
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

    /// Current hover popup for `path`, dismissed when the cursor moved
    /// on. Returns the text to show, if any.
    pub fn hover_popup(&mut self, path: &Path, cursor: (usize, usize)) -> Option<String> {
        let popup = self.hover.as_mut()?;
        if popup.path != path || (popup.cursor_line, popup.cursor_col) != cursor || !popup.open {
            if popup.path == path {
                self.hover = None;
            }
            return None;
        }
        Some(popup.text.clone())
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
        if self.hover.as_ref().is_some_and(|popup| popup.path == path) {
            self.hover = None;
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
        let candidates = terminator_native_edit::lsp::server_candidates(language_of(key.language));
        let path_env = std::env::var_os("PATH")
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Some(path) = L::find_server(candidates, &path_env) else {
            self.unavailable.insert(
                key.language,
                format!(
                    "No language server on PATH ({}); install it to enable hover, goto, and diagnostics",
                    candidates.join(", ")
                ),
            );
            return None;
        };
        match spawn_server(key, &path, self.pump_tx.clone()) {
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
            L::LspMessage::Response { id, result, .. } => {
                self.on_response(key, id, result);
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
    ) {
        let L::RequestId::Number(number) = id else {
            return;
        };
        // Init handshake first: only the starting server owns init ids.
        if let Some(server) = self.servers.get_mut(key)
            && server.init_id == Some(number)
        {
            server.init_id = None;
            server.ready = true;
            let _ = server.inbox.send(L::initialized());
            // Docs synced while starting open now.
            for doc in self.docs.values_mut().filter(|doc| doc.server == *key) {
                doc.opened = false;
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
            None => {}
        }
    }

    fn on_server_down(&mut self, key: &ServerKey) {
        self.servers.remove(key);
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
            let _ = server.child.start_kill();
        }
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

/// Inverse of [`L::language_id`], for the server table.
fn language_of(id: &str) -> Language {
    match id {
        "rust" => Language::Rust,
        "python" => Language::Python,
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

/// Spawn one server with piped stdio in `root`, handshake it, and pump
/// both directions on background tasks. Needs a running tokio runtime
/// (the GUI always has one; tests use `#[tokio::test]`).
fn spawn_server(
    key: &ServerKey,
    binary: &Path,
    pump: tokio::sync::mpsc::UnboundedSender<PumpEvent>,
) -> Result<Server, String> {
    let mut child = tokio::process::Command::new(binary)
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
        child,
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
        let path = PathBuf::from("/tmp/note.md");
        assert!(!manager.sync_doc(&path, Language::Markdown, 1));
        assert!(manager.row_marks(&path).is_empty());
        assert!(manager.problems_for(&path).is_empty());
    }

    #[test]
    fn marks_keep_worst_severity_per_row_sorted() {
        let mut manager = LspManager::new();
        let path = PathBuf::from("/tmp/main.rs");
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
                submitted: 1,
                last_change: Instant::now(),
                opened: true,
            },
        );
    }

    #[test]
    fn diagnostics_route_to_tracked_docs_only() {
        let mut manager = LspManager::new();
        let path = PathBuf::from("/tmp/routed.rs");
        tracked(&mut manager, &path);
        let key = ServerKey {
            language: "rust",
            root: project_root(&path, Language::Rust),
        };
        let params = serde_json::json!({
            "uri": "file:///tmp/routed.rs",
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
            "params": {"uri": "file:///tmp/ghost.rs", "diagnostics": []},
        })
        .to_string();
        manager.on_body(&key, unknown.as_bytes());
        assert!(
            manager
                .problems_for(&PathBuf::from("/tmp/ghost.rs"))
                .is_empty()
        );
    }

    #[test]
    fn prune_closes_docs_and_clears_state() {
        let mut manager = LspManager::new();
        let path = PathBuf::from("/tmp/gone.rs");
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
        assert!(
            problems
                .iter()
                .any(|item| item.message.contains("mismatch")),
            "type error reported: {:?}",
            problems
                .iter()
                .map(|item| &item.message)
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
            hover.as_ref().is_some_and(|text| text.contains("i32")),
            "hover: {hover:?}"
        );
        manager.prune(&std::collections::HashSet::new());
    }
}
