//! I/O-free LSP stdio core: framing, message types, request builders,
//! response parsers, UTF-16 position math, and the server command table.
//!
//! Spawning servers, owning pipes, and polling belong to the host (the
//! app manager): this module never creates processes, threads, or
//! sockets, so the crate stays standalone-publishable. The one exception
//! is [`find_server`], which stats candidate paths while probing `PATH`
//! — a pure lookup with no protocol or daemon coupling.
//!
//! Outgoing messages are built with `serde_json` (full control over the
//! minimal capability set); incoming payloads deserialize through
//! `lsp-types` and convert into the small native structs below, so the
//! view never depends on protocol types.
use std::path::{Path, PathBuf};

use serde_json::Value;
use url::Url;

use crate::highlight::Language;

/// Zero-based line/character position. Characters are UTF-16 code units
/// on the wire (see [`char_col_to_utf16`]); lines are always `\n`-based.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

/// Half-open range of [`Position`]s.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

/// Diagnostic severity, strongest first.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Severity {
    Error,
    Warning,
    Information,
    Hint,
}

/// One published diagnostic.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    pub range: Range,
    pub severity: Option<Severity>,
    pub message: String,
}

/// Hover text with its presentation format preserved.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Hover {
    pub text: String,
    pub markdown: bool,
}

/// One go-to-definition target: server URI plus range.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GotoTarget {
    pub uri: String,
    pub range: Range,
}

/// One replacement proposed by a language server during a symbol rename.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RenameTextEdit {
    pub range: Range,
    pub new_text: String,
}

/// All edits for one file. A server version, when present, must still
/// match before the host applies the rename.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RenameFileEdit {
    pub uri: String,
    pub version: Option<i32>,
    pub edits: Vec<RenameTextEdit>,
}

/// One document symbol, flattened from the server's tree: `depth` is the
/// nesting level (0 for top-level items) for outline indentation. `range`
/// spans the whole item (doc comments and attributes included) for
/// containment; `selection` is the name, where a jump should land.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Symbol {
    pub name: String,
    pub kind: String,
    pub range: Range,
    pub selection: Range,
    pub depth: usize,
    /// Server detail (a signature or type), shown dimmed beside the name.
    pub detail: Option<String>,
    pub deprecated: bool,
}

/// Index of the deepest symbol whose range contains `line`: the caret's
/// enclosing item, for breadcrumbs and Structure autoscroll.
#[must_use]
pub fn innermost_symbol(symbols: &[Symbol], line: u32) -> Option<usize> {
    symbols
        .iter()
        .enumerate()
        .filter(|(_, symbol)| symbol.range.start.line <= line && line <= symbol.range.end.line)
        .max_by_key(|(_, symbol)| (symbol.depth, symbol.range.start.line))
        .map(|(index, _)| index)
}

/// Markdown outline: one symbol per heading, nested by level. A heading's
/// range runs to the line before the next heading at its level or above,
/// so the caret's section is its innermost symbol.
#[must_use]
pub fn markdown_outline<B: crate::doc::Buffer>(doc: &B) -> Vec<Symbol> {
    use crate::blocks::{BlockKind, parse_blocks};
    let headings: Vec<(u8, usize)> = parse_blocks(doc)
        .into_iter()
        .filter_map(|block| match block.kind {
            BlockKind::Heading(level) => Some((level, block.start_line)),
            _ => None,
        })
        .collect();
    let last_line = u32::try_from(doc.line_count().saturating_sub(1)).unwrap_or(u32::MAX);
    let mut levels: Vec<u8> = Vec::new();
    let mut out = Vec::with_capacity(headings.len());
    for (index, (level, line)) in headings.iter().enumerate() {
        while levels.last().is_some_and(|open| open >= level) {
            levels.pop();
        }
        let depth = levels.len();
        levels.push(*level);
        let end = headings
            .iter()
            .skip(index.saturating_add(1))
            .find(|(next, _)| next <= level)
            .and_then(|(_, next_line)| u32::try_from(*next_line).ok())
            .map_or(last_line, |next| next.saturating_sub(1));
        let text = doc.line_text(*line);
        let name = text.trim_start_matches('#').trim().to_string();
        let start = u32::try_from(*line).unwrap_or(u32::MAX);
        let range = Range {
            start: Position {
                line: start,
                character: 0,
            },
            end: Position {
                line: end.max(start),
                character: 0,
            },
        };
        out.push(Symbol {
            name,
            kind: format!("H{level}"),
            range,
            selection: Range {
                start: range.start,
                end: range.start,
            },
            depth,
            detail: None,
            deprecated: false,
        });
    }
    out
}

/// One workspace-wide symbol match from an LSP server.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WorkspaceSymbolHit {
    pub name: String,
    pub kind: String,
    pub container: Option<String>,
    pub uri: String,
    pub range: Range,
}

/// JSON-RPC request id: integer or string, echoed back by the server.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RequestId {
    Number(i64),
    Text(String),
}

/// Monotonic request ids; wraps instead of overflowing.
#[derive(Default, Debug)]
pub struct IdGen {
    next: i64,
}

impl IdGen {
    /// Fresh id for one outgoing request.
    pub fn next_id(&mut self) -> RequestId {
        let id = self.next;
        self.next = self.next.wrapping_add(1);
        RequestId::Number(id)
    }
}

/// One decoded JSON-RPC message.
#[derive(Clone, PartialEq, Debug)]
pub enum LspMessage {
    Request {
        id: RequestId,
        method: String,
        params: Value,
    },
    Notification {
        method: String,
        params: Value,
    },
    Response {
        id: RequestId,
        result: Option<Value>,
        error: Option<ResponseError>,
    },
}

/// JSON-RPC error payload on a response.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ResponseError {
    pub code: i64,
    pub message: String,
}

/// Frame one JSON body with its `Content-Length` header.
#[must_use]
pub fn frame(body: &[u8]) -> Vec<u8> {
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body);
    out
}

/// Incremental `Content-Length` framing decoder: feed arbitrary stdout
/// chunks, drain complete bodies. Unknown headers are skipped; a header
/// block without `Content-Length` is discarded through its terminator.
#[derive(Default, Debug)]
pub struct FrameDecoder {
    buf: Vec<u8>,
}

impl FrameDecoder {
    /// Feed bytes; returns every complete message body now available.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        loop {
            match take_frame(&mut self.buf) {
                Frame::Body(body) => out.push(body),
                // Drained a header without a body claim: resync and keep
                // scanning; every drain makes progress, so this ends.
                Frame::Drained => continue,
                Frame::Incomplete => break,
            }
        }
        out
    }
}

enum Frame {
    Body(Vec<u8>),
    Drained,
    Incomplete,
}

/// Split one complete frame off the front of `buf`, if present. A
/// header block without `Content-Length` is discarded so the stream
/// resyncs on the next frame instead of stalling.
fn take_frame(buf: &mut Vec<u8>) -> Frame {
    let Some(end) = find_header_end(buf) else {
        return Frame::Incomplete;
    };
    // Only scan the header block; the body may contain anything.
    let header = buf.get(..end).unwrap_or_default().to_vec();
    let mut length: Option<usize> = None;
    for line in header.split(|b| *b == b'\n') {
        if let Some(value) = line
            .strip_prefix(b"Content-Length:")
            .or_else(|| line.strip_prefix(b"content-length:"))
        {
            let text = std::str::from_utf8(value).unwrap_or("").trim();
            length = text.parse::<usize>().ok();
        }
    }
    let Some(length) = length else {
        // Explicit consume: `drain` is lazy, dropping it unfinished still
        // removes the range, but spelling the consume out leaves no doubt.
        buf.drain(..end).for_each(drop);
        return Frame::Drained;
    };
    let total = end.saturating_add(length);
    if buf.len() < total {
        return Frame::Incomplete;
    }
    // Drop the header first: draining `end..total` would leave it
    // prepended and stall every later frame on a bogus huge length.
    buf.drain(..end).for_each(drop);
    Frame::Body(buf.drain(..length).collect())
}

/// Byte offset just past the `\r\n\r\n` terminating the header block.
fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|at| at.saturating_add(4))
}

/// Classify one complete message body. `None` means not valid
/// JSON-RPC (the caller logs and drops it).
#[must_use]
pub fn parse_message(body: &[u8]) -> Option<LspMessage> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let object = value.as_object()?;
    let id = object.get("id").and_then(parse_id);
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .map(String::from);
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    match (id, method) {
        (Some(id), Some(method)) => Some(LspMessage::Request { id, method, params }),
        (None, Some(method)) => Some(LspMessage::Notification { method, params }),
        (Some(id), None) => {
            let result = object.get("result").cloned();
            let error = object.get("error").and_then(parse_error);
            Some(LspMessage::Response { id, result, error })
        }
        (None, None) => None,
    }
}

fn parse_id(value: &Value) -> Option<RequestId> {
    match value {
        Value::Number(number) => number.as_i64().map(RequestId::Number),
        Value::String(text) => Some(RequestId::Text(text.clone())),
        _ => None,
    }
}

fn parse_error(value: &Value) -> Option<ResponseError> {
    let object = value.as_object()?;
    Some(ResponseError {
        code: object.get("code")?.as_i64()?,
        message: object
            .get("message")
            .and_then(Value::as_str)
            .map_or(String::new(), String::from),
    })
}

fn id_json(id: &RequestId) -> Value {
    match id {
        RequestId::Number(number) => Value::from(*number),
        RequestId::Text(text) => Value::from(text.clone()),
    }
}

fn request(id: &RequestId, method: &str, params: Value) -> Vec<u8> {
    frame(
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id_json(id),
            "method": method,
            "params": params,
        })
        .to_string()
        .as_bytes(),
    )
}

fn notification(method: &str, params: Value) -> Vec<u8> {
    frame(
        serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        })
        .to_string()
        .as_bytes(),
    )
}

/// Minimal `initialize`: single `rootUri`, hover/definition/diagnostics
/// capability, explicit UTF-16 positions (matches [`char_col_to_utf16`]).
#[must_use]
pub fn initialize(id: &RequestId, process_id: u32, root: &Url) -> Vec<u8> {
    request(
        id,
        "initialize",
        serde_json::json!({
            "processId": process_id,
            "rootUri": root.as_str(),
            "capabilities": {
                "general": { "positionEncodings": ["utf-16"] },
                "workspace": { "workspaceEdit": { "documentChanges": true } },
                "textDocument": {
                    "synchronization": { "didSave": false },
                    "hover": { "contentFormat": ["markdown", "plaintext"] },
                    "definition": { "linkSupport": true },
                    "rename": {},
                    "formatting": {},
                    "inlayHint": {},
                    "callHierarchy": {},
                    "typeHierarchy": {},
                    "codeAction": {
                        "codeActionLiteralSupport": {
                            "codeActionKind": {
                                "valueSet": [
                                    "", "quickfix", "refactor", "refactor.extract",
                                    "refactor.inline", "refactor.rewrite", "source",
                                    "source.organizeImports",
                                ],
                            },
                        },
                        "resolveSupport": { "properties": ["edit"] },
                        "dataSupport": true,
                    },
                    "signatureHelp": {
                        "signatureInformation": {
                            "parameterInformation": { "labelOffsetSupport": true },
                        },
                    },
                    "completion": {
                        "completionItem": {
                            "snippetSupport": true,
                            "documentationFormat": ["markdown", "plaintext"],
                        },
                    },
                    "publishDiagnostics": { "relatedInformation": true },
                },
            },
        }),
    )
}

/// `initialized` handshake after a good `initialize` response.
#[must_use]
pub fn initialized() -> Vec<u8> {
    notification("initialized", serde_json::json!({}))
}

/// `shutdown` request before `exit`.
#[must_use]
pub fn shutdown(id: &RequestId) -> Vec<u8> {
    request(id, "shutdown", Value::Null)
}

/// `exit` notification; the server should quit after `shutdown`.
#[must_use]
pub fn exit() -> Vec<u8> {
    notification("exit", Value::Null)
}

/// Full-text `didOpen` for one buffer.
#[must_use]
pub fn did_open(uri: &Url, language_id: &str, version: i32, text: &str) -> Vec<u8> {
    notification(
        "textDocument/didOpen",
        serde_json::json!({
            "textDocument": {
                "uri": uri.as_str(),
                "languageId": language_id,
                "version": version,
                "text": text,
            },
        }),
    )
}

/// Full-text `didChange`: the whole buffer, every edit (simple, correct;
/// incremental sync can come later if profiles demand it).
#[must_use]
pub fn did_change(uri: &Url, version: i32, text: &str) -> Vec<u8> {
    notification(
        "textDocument/didChange",
        serde_json::json!({
            "textDocument": { "uri": uri.as_str(), "version": version },
            "contentChanges": [{ "text": text }],
        }),
    )
}

/// `didClose` for one buffer.
#[must_use]
pub fn did_close(uri: &Url) -> Vec<u8> {
    notification(
        "textDocument/didClose",
        serde_json::json!({ "textDocument": { "uri": uri.as_str() } }),
    )
}

/// `textDocument/hover` at a UTF-16 position.
#[must_use]
pub fn hover(id: &RequestId, uri: &Url, position: Position) -> Vec<u8> {
    request(
        id,
        "textDocument/hover",
        serde_json::json!({
            "textDocument": { "uri": uri.as_str() },
            "position": { "line": position.line, "character": position.character },
        }),
    )
}

/// `textDocument/definition` at a UTF-16 position.
#[must_use]
pub fn definition(id: &RequestId, uri: &Url, position: Position) -> Vec<u8> {
    request(
        id,
        "textDocument/definition",
        serde_json::json!({
            "textDocument": { "uri": uri.as_str() },
            "position": { "line": position.line, "character": position.character },
        }),
    )
}

/// `textDocument/completion` at a UTF-16 position (invoked, not triggered
/// by a character: servers then return their full list for the prefix).
#[must_use]
pub fn completion(id: &RequestId, uri: &Url, position: Position) -> Vec<u8> {
    request(
        id,
        "textDocument/completion",
        serde_json::json!({
            "textDocument": { "uri": uri.as_str() },
            "position": { "line": position.line, "character": position.character },
            "context": { "triggerKind": 1 },
        }),
    )
}

/// `textDocument/formatting` for a whole document (4-space indents).
#[must_use]
pub fn formatting(id: &RequestId, uri: &Url) -> Vec<u8> {
    request(
        id,
        "textDocument/formatting",
        serde_json::json!({
            "textDocument": { "uri": uri.as_str() },
            "options": { "tabSize": 4, "insertSpaces": true },
        }),
    )
}

/// One inlay hint: `label` is painted before the char at `position`
/// (UTF-16, like every protocol position). Padding is folded into the
/// label as spaces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlayHint {
    pub position: Position,
    pub label: String,
}

/// `textDocument/inlayHint` for lines `0..lines` (the whole document).
#[must_use]
pub fn inlay_hint(id: &RequestId, uri: &Url, lines: u32) -> Vec<u8> {
    request(
        id,
        "textDocument/inlayHint",
        serde_json::json!({
            "textDocument": { "uri": uri.as_str() },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": lines, "character": 0 },
            },
        }),
    )
}

/// Parse an `InlayHint[]` result, sorted by position. Labels may be a
/// string or label parts; null or unrecognized shapes yield no hints.
#[must_use]
pub fn parse_inlay_hints(result: &Value) -> Vec<InlayHint> {
    let Some(items) = result.as_array() else {
        return Vec::new();
    };
    let mut hints: Vec<InlayHint> = items
        .iter()
        .filter_map(|item| {
            let position = item.get("position")?;
            let position = Position {
                line: u32::try_from(position.get("line")?.as_u64()?).ok()?,
                character: u32::try_from(position.get("character")?.as_u64()?).ok()?,
            };
            let text = match item.get("label")? {
                Value::String(text) => text.clone(),
                Value::Array(parts) => parts
                    .iter()
                    .filter_map(|part| part.get("value").and_then(Value::as_str))
                    .collect(),
                _ => return None,
            };
            let text = text.trim();
            if text.is_empty() {
                return None;
            }
            let pad = |key: &str| item.get(key).and_then(Value::as_bool) == Some(true);
            let mut label = String::with_capacity(text.len().saturating_add(2));
            if pad("paddingLeft") {
                label.push(' ');
            }
            label.push_str(text);
            if pad("paddingRight") {
                label.push(' ');
            }
            Some(InlayHint { position, label })
        })
        .collect();
    hints.sort_by_key(|hint| (hint.position.line, hint.position.character));
    hints
}

/// Parse a `TextEdit[]` result (formatting); null means no changes.
#[must_use]
pub fn parse_text_edits(result: &Value) -> Vec<RenameTextEdit> {
    serde_json::from_value::<Vec<lsp_types::TextEdit>>(result.clone())
        .unwrap_or_default()
        .into_iter()
        .map(|edit| RenameTextEdit {
            range: native_range(&edit.range),
            new_text: edit.new_text,
        })
        .collect()
}

/// `textDocument/codeAction` for a range, with the diagnostics touching it.
#[must_use]
pub fn code_action(id: &RequestId, uri: &Url, range: Range, diagnostics: &[Diagnostic]) -> Vec<u8> {
    let position = |p: Position| serde_json::json!({ "line": p.line, "character": p.character });
    let diagnostics: Vec<Value> = diagnostics
        .iter()
        .map(|item| {
            let severity = match item.severity {
                Some(Severity::Error) => 1,
                Some(Severity::Warning) => 2,
                Some(Severity::Information) => 3,
                Some(Severity::Hint) | None => 4,
            };
            serde_json::json!({
                "range": { "start": position(item.range.start), "end": position(item.range.end) },
                "severity": severity,
                "message": item.message,
            })
        })
        .collect();
    request(
        id,
        "textDocument/codeAction",
        serde_json::json!({
            "textDocument": { "uri": uri.as_str() },
            "range": { "start": position(range.start), "end": position(range.end) },
            "context": { "diagnostics": diagnostics },
        }),
    )
}

/// `codeAction/resolve`: fill in a lazily computed action's edit.
#[must_use]
pub fn code_action_resolve(id: &RequestId, action: &Value) -> Vec<u8> {
    request(id, "codeAction/resolve", action.clone())
}

/// One proposed code action. `raw` is the server's object, sent back for
/// `codeAction/resolve` when `edit` is absent.
#[derive(Clone, Debug, PartialEq)]
pub struct CodeActionItem {
    pub title: String,
    pub kind: Option<String>,
    pub preferred: bool,
    pub raw: Value,
}

/// Parse `textDocument/codeAction`: actions that carry an edit or can be
/// resolved to one. Bare commands need `workspace/executeCommand`, which
/// this client does not run, so they are left out.
#[must_use]
pub fn parse_code_actions(result: &Value) -> Vec<CodeActionItem> {
    let Some(items) = result.as_array() else {
        return Vec::new();
    };
    let mut out: Vec<CodeActionItem> = items
        .iter()
        .filter(|item| item.get("edit").is_some() || item.get("data").is_some())
        .filter_map(|item| {
            Some(CodeActionItem {
                title: item.get("title")?.as_str()?.to_owned(),
                kind: item.get("kind").and_then(Value::as_str).map(str::to_owned),
                preferred: item.get("isPreferred").and_then(Value::as_bool) == Some(true),
                raw: item.clone(),
            })
        })
        .collect();
    // Preferred fixes first, then quick fixes, then the rest in order.
    out.sort_by_key(|item| {
        (
            !item.preferred,
            !item
                .kind
                .as_deref()
                .is_some_and(|kind| kind.starts_with("quickfix")),
        )
    });
    out
}

/// The text edits of a (resolved) code action, per file.
pub fn code_action_edits(action: &Value) -> Result<Vec<RenameFileEdit>, String> {
    let edit = action
        .get("edit")
        .ok_or_else(|| "The language server did not provide an edit for this action".to_owned())?;
    parse_rename(edit)
}

/// `textDocument/signatureHelp` at a UTF-16 position.
#[must_use]
pub fn signature_help(id: &RequestId, uri: &Url, position: Position) -> Vec<u8> {
    request(
        id,
        "textDocument/signatureHelp",
        serde_json::json!({
            "textDocument": { "uri": uri.as_str() },
            "position": { "line": position.line, "character": position.character },
        }),
    )
}

/// The active signature, with the active parameter's char range inside
/// `label` for highlighting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignatureHelp {
    pub label: String,
    pub active: Option<std::ops::Range<usize>>,
    pub documentation: Option<String>,
}

/// Parse a `textDocument/signatureHelp` result; `None` when there is no
/// signature to show.
#[must_use]
pub fn parse_signature_help(result: &Value) -> Option<SignatureHelp> {
    let help: lsp_types::SignatureHelp = serde_json::from_value(result.clone()).ok()?;
    let index = help
        .active_signature
        .and_then(|index| usize::try_from(index).ok())
        .unwrap_or(0);
    let signature = help
        .signatures
        .get(index)
        .or_else(|| help.signatures.first())?;
    let parameter = signature
        .active_parameter
        .or(help.active_parameter)
        .and_then(|index| usize::try_from(index).ok());
    let active = parameter
        .and_then(|index| signature.parameters.as_ref()?.get(index))
        .and_then(|parameter| match &parameter.label {
            lsp_types::ParameterLabel::Simple(text) => {
                let start = signature.label.find(text.as_str())?;
                let head = signature.label.get(..start)?.chars().count();
                Some(head..head.saturating_add(text.chars().count()))
            }
            lsp_types::ParameterLabel::LabelOffsets([start, end]) => {
                // UTF-16 offsets into the label.
                let start = utf16_to_char_col(&signature.label, *start);
                let end = utf16_to_char_col(&signature.label, *end);
                Some(start..end)
            }
        });
    let documentation = signature.documentation.as_ref().map(|doc| match doc {
        lsp_types::Documentation::String(text) => text.clone(),
        lsp_types::Documentation::MarkupContent(markup) => markup.value.clone(),
    });
    Some(SignatureHelp {
        label: signature.label.clone(),
        active,
        documentation: documentation.filter(|text| !text.trim().is_empty()),
    })
}

/// One completion proposal, reduced to what the popup shows and inserts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub kind: Option<String>,
    pub detail: Option<String>,
    /// Text to insert (snippet syntax when `snippet`).
    pub insert: String,
    pub snippet: bool,
    /// Text the typed prefix is matched against.
    pub filter: String,
    /// Server ordering key.
    pub sort: String,
    /// Range the insert replaces, when the server names one.
    pub edit: Option<Range>,
}

/// Parse a `textDocument/completion` result (item array or list).
#[must_use]
pub fn parse_completion(result: &Value) -> Vec<CompletionItem> {
    let Ok(response) = serde_json::from_value::<lsp_types::CompletionResponse>(result.clone())
    else {
        return Vec::new();
    };
    let items = match response {
        lsp_types::CompletionResponse::Array(items) => items,
        lsp_types::CompletionResponse::List(list) => list.items,
    };
    items
        .into_iter()
        .map(|item| {
            let (insert, edit) = match &item.text_edit {
                Some(lsp_types::CompletionTextEdit::Edit(edit)) => {
                    (edit.new_text.clone(), Some(native_range(&edit.range)))
                }
                Some(lsp_types::CompletionTextEdit::InsertAndReplace(edit)) => {
                    (edit.new_text.clone(), Some(native_range(&edit.insert)))
                }
                None => (
                    item.insert_text
                        .clone()
                        .unwrap_or_else(|| item.label.clone()),
                    None,
                ),
            };
            CompletionItem {
                kind: item.kind.map(completion_kind_label),
                detail: item.detail.clone().filter(|detail| !detail.is_empty()),
                snippet: item.insert_text_format == Some(lsp_types::InsertTextFormat::SNIPPET),
                filter: item
                    .filter_text
                    .clone()
                    .unwrap_or_else(|| item.label.clone()),
                sort: item.sort_text.clone().unwrap_or_else(|| item.label.clone()),
                label: item.label,
                insert,
                edit,
            }
        })
        .collect()
}

fn completion_kind_label(kind: lsp_types::CompletionItemKind) -> String {
    use lsp_types::CompletionItemKind as Kind;
    match kind {
        Kind::METHOD => "Method",
        Kind::FUNCTION => "Function",
        Kind::CONSTRUCTOR => "Constructor",
        Kind::FIELD => "Field",
        Kind::VARIABLE => "Variable",
        Kind::CLASS => "Class",
        Kind::INTERFACE => "Interface",
        Kind::MODULE => "Module",
        Kind::PROPERTY => "Property",
        Kind::ENUM => "Enum",
        Kind::KEYWORD => "Keyword",
        Kind::SNIPPET => "Snippet",
        Kind::CONSTANT => "Constant",
        Kind::STRUCT => "Struct",
        Kind::ENUM_MEMBER => "Enum member",
        Kind::TYPE_PARAMETER => "Type parameter",
        _ => "Symbol",
    }
    .to_owned()
}

/// Expand LSP snippet syntax to plain text: `${1:name}` keeps `name`,
/// `$1`/`$0` vanish, `\$` is a literal. Returns the text and the char
/// offset where the caret goes: the first tab stop, else `$0`, else end.
#[must_use]
pub fn expand_snippet(snippet: &str) -> (String, usize) {
    let mut out = String::new();
    let mut first: Option<usize> = None;
    let mut last: Option<usize> = None;
    let mut chars = snippet.chars().peekable();
    let mut depth = 0usize;
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            }
            '$' => {
                let at = out.chars().count();
                if chars.peek() == Some(&'{') {
                    chars.next();
                    let mut number = String::new();
                    while chars.peek().is_some_and(char::is_ascii_digit) {
                        number.extend(chars.next());
                    }
                    if chars.peek() == Some(&':') {
                        chars.next();
                    } else if chars.peek() == Some(&'|') {
                        // Choice: keep the first option.
                        chars.next();
                        for choice in chars.by_ref() {
                            if choice == ',' || choice == '|' {
                                break;
                            }
                            out.push(choice);
                        }
                        for skip in chars.by_ref() {
                            if skip == '}' {
                                break;
                            }
                        }
                        continue;
                    }
                    depth = depth.saturating_add(1);
                    if number == "0" {
                        last = Some(at);
                    } else if first.is_none() {
                        first = Some(at);
                    }
                } else {
                    let mut number = String::new();
                    while chars.peek().is_some_and(char::is_ascii_digit) {
                        number.extend(chars.next());
                    }
                    if number == "0" {
                        last = Some(at);
                    } else if !number.is_empty() && first.is_none() {
                        first = Some(at);
                    }
                }
            }
            '}' if depth > 0 => depth = depth.saturating_sub(1),
            other => out.push(other),
        }
    }
    let end = out.chars().count();
    (out, first.or(last).unwrap_or(end))
}

/// `textDocument/references` at a UTF-16 position, including the declaration.
#[must_use]
pub fn references(id: &RequestId, uri: &Url, position: Position) -> Vec<u8> {
    request(
        id,
        "textDocument/references",
        serde_json::json!({
            "textDocument": { "uri": uri.as_str() },
            "position": { "line": position.line, "character": position.character },
            "context": { "includeDeclaration": true },
        }),
    )
}

/// Which hierarchy a request walks (`IntelliJ` Call / Type Hierarchy).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HierarchyKind {
    Call,
    Type,
}

/// Which way a hierarchy expands: callers / supertypes (`Up`), or
/// callees / subtypes (`Down`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HierarchyDirection {
    Up,
    Down,
}

/// One call or type hierarchy item. `raw` is the server's own object,
/// sent back verbatim to expand it.
#[derive(Clone, Debug, PartialEq)]
pub struct HierarchyItem {
    pub name: String,
    pub kind: String,
    pub detail: Option<String>,
    pub uri: String,
    pub selection: Range,
    pub raw: Value,
}

/// Whether a server's `initialize` capabilities offer `kind`.
#[must_use]
pub fn supports_hierarchy(capabilities: &Value, kind: HierarchyKind) -> bool {
    let key = match kind {
        HierarchyKind::Call => "callHierarchyProvider",
        HierarchyKind::Type => "typeHierarchyProvider",
    };
    match capabilities.get(key) {
        Some(Value::Bool(on)) => *on,
        Some(Value::Object(_)) => true,
        _ => false,
    }
}

/// `textDocument/prepareCallHierarchy` / `prepareTypeHierarchy` at a
/// UTF-16 position.
#[must_use]
pub fn prepare_hierarchy(
    id: &RequestId,
    uri: &Url,
    position: Position,
    kind: HierarchyKind,
) -> Vec<u8> {
    let method = match kind {
        HierarchyKind::Call => "textDocument/prepareCallHierarchy",
        HierarchyKind::Type => "textDocument/prepareTypeHierarchy",
    };
    request(
        id,
        method,
        serde_json::json!({
            "textDocument": { "uri": uri.as_str() },
            "position": { "line": position.line, "character": position.character },
        }),
    )
}

/// The children of a hierarchy item: incoming / outgoing calls, or
/// super / sub types.
#[must_use]
pub fn hierarchy_children(
    id: &RequestId,
    item: &Value,
    kind: HierarchyKind,
    direction: HierarchyDirection,
) -> Vec<u8> {
    let method = match (kind, direction) {
        (HierarchyKind::Call, HierarchyDirection::Up) => "callHierarchy/incomingCalls",
        (HierarchyKind::Call, HierarchyDirection::Down) => "callHierarchy/outgoingCalls",
        (HierarchyKind::Type, HierarchyDirection::Up) => "typeHierarchy/supertypes",
        (HierarchyKind::Type, HierarchyDirection::Down) => "typeHierarchy/subtypes",
    };
    request(id, method, serde_json::json!({ "item": item }))
}

/// Parse hierarchy items from a prepare or children answer. Call answers
/// wrap each item as `{ from | to, fromRanges }`; those are unwrapped.
#[must_use]
pub fn parse_hierarchy_items(result: &Value) -> Vec<HierarchyItem> {
    let Some(items) = result.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let item = item.get("from").or_else(|| item.get("to")).unwrap_or(item);
            let range = |key: &str| -> Option<Range> {
                let range: lsp_types::Range =
                    serde_json::from_value(item.get(key)?.clone()).ok()?;
                Some(native_range(&range))
            };
            let kind = item
                .get("kind")
                .and_then(|kind| serde_json::from_value::<lsp_types::SymbolKind>(kind.clone()).ok())
                .map_or_else(String::new, symbol_kind_label);
            Some(HierarchyItem {
                name: item.get("name")?.as_str()?.to_owned(),
                kind,
                detail: item
                    .get("detail")
                    .and_then(Value::as_str)
                    .filter(|detail| !detail.is_empty())
                    .map(str::to_owned),
                uri: item.get("uri")?.as_str()?.to_owned(),
                selection: range("selectionRange").or_else(|| range("range"))?,
                raw: item.clone(),
            })
        })
        .collect()
}

/// `textDocument/rename` at a UTF-16 position.
#[must_use]
pub fn rename(id: &RequestId, uri: &Url, position: Position, new_name: &str) -> Vec<u8> {
    request(
        id,
        "textDocument/rename",
        serde_json::json!({
            "textDocument": { "uri": uri.as_str() },
            "position": { "line": position.line, "character": position.character },
            "newName": new_name,
        }),
    )
}

/// `textDocument/documentSymbol` for one document.
#[must_use]
pub fn document_symbol(id: &RequestId, uri: &Url) -> Vec<u8> {
    request(
        id,
        "textDocument/documentSymbol",
        serde_json::json!({
            "textDocument": { "uri": uri.as_str() },
        }),
    )
}

/// `workspace/symbol` for project-wide symbol search.
#[must_use]
pub fn workspace_symbol(id: &RequestId, query: &str) -> Vec<u8> {
    request(
        id,
        "workspace/symbol",
        serde_json::json!({ "query": query }),
    )
}

fn native_position(value: lsp_types::Position) -> Position {
    Position {
        line: value.line,
        character: value.character,
    }
}

fn native_range(value: &lsp_types::Range) -> Range {
    Range {
        start: native_position(value.start),
        end: native_position(value.end),
    }
}

fn native_severity(value: lsp_types::DiagnosticSeverity) -> Severity {
    if value == lsp_types::DiagnosticSeverity::ERROR {
        Severity::Error
    } else if value == lsp_types::DiagnosticSeverity::WARNING {
        Severity::Warning
    } else if value == lsp_types::DiagnosticSeverity::INFORMATION {
        Severity::Information
    } else {
        Severity::Hint
    }
}

/// Parse `textDocument/publishDiagnostics` params into the document URI
/// plus native diagnostics. `None` means the payload is not diagnostics.
#[must_use]
pub fn parse_diagnostics(params: &Value) -> Option<(String, Vec<Diagnostic>)> {
    let parsed: lsp_types::PublishDiagnosticsParams =
        serde_json::from_value(params.clone()).ok()?;
    let items = parsed
        .diagnostics
        .iter()
        .map(|item| Diagnostic {
            range: native_range(&item.range),
            severity: item.severity.map(native_severity),
            message: item.message.clone(),
        })
        .collect();
    Some((parsed.uri.as_str().to_owned(), items))
}

/// Parse a `textDocument/hover` result. `None` means null
/// or an unrecognized shape (no hover available).
#[must_use]
pub fn parse_hover(result: &Value) -> Option<Hover> {
    if result.is_null() {
        return None;
    }
    let parsed: lsp_types::Hover = serde_json::from_value(result.clone()).ok()?;
    let (text, markdown) = match parsed.contents {
        lsp_types::HoverContents::Scalar(marked) => (marked_text(&marked), true),
        lsp_types::HoverContents::Array(marked) => (
            marked
                .iter()
                .map(marked_text)
                .collect::<Vec<_>>()
                .join("\n\n"),
            true,
        ),
        lsp_types::HoverContents::Markup(content) => (
            content.value,
            content.kind == lsp_types::MarkupKind::Markdown,
        ),
    };
    Some(Hover { text, markdown })
}

fn marked_text(value: &lsp_types::MarkedString) -> String {
    match value {
        lsp_types::MarkedString::String(text) => text.clone(),
        lsp_types::MarkedString::LanguageString(content) => {
            format!("```{}\n{}\n```", content.language, content.value)
        }
    }
}

/// Parse a `textDocument/definition` result. Null or unrecognized shapes
/// yield no targets.
#[must_use]
pub fn parse_definition(result: &Value) -> Vec<GotoTarget> {
    if result.is_null() {
        return Vec::new();
    }
    let parsed: Result<lsp_types::GotoDefinitionResponse, _> =
        serde_json::from_value(result.clone());
    match parsed {
        Ok(lsp_types::GotoDefinitionResponse::Scalar(location)) => {
            vec![native_location(&location.uri, &location.range)]
        }
        Ok(lsp_types::GotoDefinitionResponse::Array(locations)) => locations
            .iter()
            .map(|location| native_location(&location.uri, &location.range))
            .collect(),
        Ok(lsp_types::GotoDefinitionResponse::Link(links)) => links
            .iter()
            .map(|link| native_location(&link.target_uri, &link.target_range))
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Parse a `textDocument/references` location list. Invalid entries are
/// ignored; an unsupported or null result produces an empty list.
#[must_use]
pub fn parse_references(result: &Value) -> Vec<GotoTarget> {
    let Ok(locations) = serde_json::from_value::<Vec<lsp_types::Location>>(result.clone()) else {
        return Vec::new();
    };
    locations
        .iter()
        .map(|location| native_location(&location.uri, &location.range))
        .collect()
}

/// Parse only text replacements. Resource operations and change annotations
/// require a different preview/apply flow and are rejected as a whole.
pub fn parse_rename(result: &Value) -> Result<Vec<RenameFileEdit>, String> {
    let edit: lsp_types::WorkspaceEdit = serde_json::from_value(result.clone())
        .map_err(|error| format!("Invalid rename response: {error}"))?;
    if edit
        .change_annotations
        .as_ref()
        .is_some_and(|items| !items.is_empty())
    {
        return Err("Rename response contains unsupported change annotations".into());
    }
    let mut files = Vec::new();
    if let Some(changes) = edit.document_changes {
        let documents = match changes {
            lsp_types::DocumentChanges::Edits(documents) => documents,
            lsp_types::DocumentChanges::Operations(operations) => operations
                .into_iter()
                .map(|operation| match operation {
                    lsp_types::DocumentChangeOperation::Edit(document) => Ok(document),
                    lsp_types::DocumentChangeOperation::Op(_) => {
                        Err("Rename response contains a file create, move, or delete".to_string())
                    }
                })
                .collect::<Result<Vec<_>, _>>()?,
        };
        for document in documents {
            let mut edits = Vec::new();
            for edit in document.edits {
                let edit = match edit {
                    lsp_types::OneOf::Left(edit) => edit,
                    lsp_types::OneOf::Right(_) => {
                        return Err("Rename response contains an annotated edit".into());
                    }
                };
                edits.push(RenameTextEdit {
                    range: native_range(&edit.range),
                    new_text: edit.new_text,
                });
            }
            files.push(RenameFileEdit {
                uri: document.text_document.uri.as_str().to_owned(),
                version: document.text_document.version,
                edits,
            });
        }
    } else if let Some(changes) = edit.changes {
        for (uri, edits) in changes {
            files.push(RenameFileEdit {
                uri: uri.as_str().to_owned(),
                version: None,
                edits: edits
                    .into_iter()
                    .map(|edit| RenameTextEdit {
                        range: native_range(&edit.range),
                        new_text: edit.new_text,
                    })
                    .collect(),
            });
        }
    }
    files.sort_by(|a, b| a.uri.cmp(&b.uri));
    Ok(files)
}

/// Parse a `textDocument/documentSymbol` result (hierarchical
/// `DocumentSymbol[]` or flat `SymbolInformation[]`) into a flat,
/// depth-tagged outline. Null or unrecognized shapes yield no symbols.
#[must_use]
pub fn parse_document_symbols(result: &Value) -> Vec<Symbol> {
    if result.is_null() {
        return Vec::new();
    }
    let parsed: Result<lsp_types::DocumentSymbolResponse, _> =
        serde_json::from_value(result.clone());
    let mut out = Vec::new();
    match parsed {
        Ok(lsp_types::DocumentSymbolResponse::Nested(items)) => {
            fn walk(items: &[lsp_types::DocumentSymbol], depth: usize, out: &mut Vec<Symbol>) {
                for item in items {
                    out.push(Symbol {
                        name: item.name.clone(),
                        kind: symbol_kind_label(item.kind),
                        range: native_range(&item.range),
                        selection: native_range(&item.selection_range),
                        depth,
                        detail: item.detail.clone().filter(|detail| !detail.is_empty()),
                        #[allow(deprecated)] // older servers only set the flag
                        deprecated: item.deprecated == Some(true)
                            || item
                                .tags
                                .as_ref()
                                .is_some_and(|tags| tags.contains(&lsp_types::SymbolTag::DEPRECATED)),
                    });
                    if let Some(children) = &item.children {
                        walk(children, depth.saturating_add(1), out);
                    }
                }
            }
            walk(&items, 0, &mut out);
        }
        Ok(lsp_types::DocumentSymbolResponse::Flat(items)) => {
            for item in items {
                out.push(Symbol {
                    name: item.name.clone(),
                    kind: symbol_kind_label(item.kind),
                    range: native_range(&item.location.range),
                    selection: native_range(&item.location.range),
                    depth: 0,
                    detail: item.container_name.clone(),
                    #[allow(deprecated)] // older servers only set the flag
                    deprecated: item.deprecated == Some(true)
                        || item
                            .tags
                            .as_ref()
                            .is_some_and(|tags| tags.contains(&lsp_types::SymbolTag::DEPRECATED)),
                });
            }
        }
        Err(_) => {}
    }
    out
}

/// Parse both LSP workspace-symbol response shapes. Unresolved locations
/// have no range and cannot be opened without a follow-up capability, so
/// they are skipped.
#[must_use]
pub fn parse_workspace_symbols(result: &Value) -> Vec<WorkspaceSymbolHit> {
    if result.is_null() {
        return Vec::new();
    }
    let parsed: Result<lsp_types::WorkspaceSymbolResponse, _> =
        serde_json::from_value(result.clone());
    match parsed {
        Ok(lsp_types::WorkspaceSymbolResponse::Flat(items)) => items
            .into_iter()
            .map(|item| WorkspaceSymbolHit {
                name: item.name,
                kind: symbol_kind_label(item.kind),
                container: item.container_name,
                uri: item.location.uri.as_str().to_owned(),
                range: native_range(&item.location.range),
            })
            .collect(),
        Ok(lsp_types::WorkspaceSymbolResponse::Nested(items)) => items
            .into_iter()
            .filter_map(|item| {
                let lsp_types::OneOf::Left(location) = item.location else {
                    return None;
                };
                Some(WorkspaceSymbolHit {
                    name: item.name,
                    kind: symbol_kind_label(item.kind),
                    container: item.container_name,
                    uri: location.uri.as_str().to_owned(),
                    range: native_range(&location.range),
                })
            })
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn symbol_kind_label(kind: lsp_types::SymbolKind) -> String {
    use lsp_types::SymbolKind as Kind;
    match kind {
        Kind::FILE => "File",
        Kind::MODULE => "Module",
        Kind::NAMESPACE => "Namespace",
        Kind::PACKAGE => "Package",
        Kind::CLASS => "Class",
        Kind::METHOD => "Method",
        Kind::PROPERTY => "Property",
        Kind::FIELD => "Field",
        Kind::CONSTRUCTOR => "Constructor",
        Kind::ENUM => "Enum",
        Kind::INTERFACE => "Interface",
        Kind::FUNCTION => "Function",
        Kind::VARIABLE => "Variable",
        Kind::CONSTANT => "Constant",
        Kind::STRING => "String",
        Kind::NUMBER => "Number",
        Kind::BOOLEAN => "Boolean",
        Kind::ARRAY => "Array",
        Kind::OBJECT => "Object",
        Kind::KEY => "Key",
        Kind::NULL => "Null",
        Kind::ENUM_MEMBER => "Enum member",
        Kind::STRUCT => "Struct",
        Kind::EVENT => "Event",
        Kind::OPERATOR => "Operator",
        Kind::TYPE_PARAMETER => "Type parameter",
        _ => "Symbol",
    }
    .to_string()
}

fn native_location(uri: &lsp_types::Uri, range: &lsp_types::Range) -> GotoTarget {
    GotoTarget {
        uri: uri.as_str().to_owned(),
        range: native_range(range),
    }
}

/// Server family id for a language, if it has a server. TypeScript and
/// JavaScript share one family, because one `typescript-language-server`
/// serves both; the per-file `didOpen` id comes from
/// [`language_id_for_path`].
#[must_use]
pub fn server_language(language: Language) -> Option<&'static str> {
    match language {
        Language::Rust => Some("rust"),
        Language::Python => Some("python"),
        Language::TypeScript | Language::JavaScript => Some("typescript"),
        _ => None,
    }
}

/// Protocol `languageId` for `didOpen`: TypeScript and JavaScript have
/// several (`.tsx` is `typescriptreact`, `.jsx` is `javascriptreact`).
#[must_use]
pub fn language_id_for_path(path: &Path, language: Language) -> Option<&'static str> {
    match language {
        Language::Rust => Some("rust"),
        Language::Python => Some("python"),
        Language::TypeScript => Some(if extension(path) == "tsx" {
            "typescriptreact"
        } else {
            "typescript"
        }),
        Language::JavaScript => Some(javascript_id(path)),
        _ => None,
    }
}

fn javascript_id(path: &Path) -> &'static str {
    if extension(path) == "jsx" {
        "javascriptreact"
    } else {
        "javascript"
    }
}

fn extension(path: &Path) -> String {
    path.extension()
        .map(|value| value.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

/// Server binary candidates for a language, in preference order. Only
/// languages here get a server; everything else edits without LSP.
#[must_use]
pub fn server_candidates(language: Language) -> &'static [&'static str] {
    match language {
        Language::Rust => &["rust-analyzer"],
        Language::Python => &["pylsp"],
        Language::TypeScript | Language::JavaScript => &["typescript-language-server"],
        _ => &[],
    }
}

/// Extra command-line arguments the server needs. TypeScript's server
/// speaks LSP only on stdio, which it selects with `--stdio`.
#[must_use]
pub fn server_args(language: Language) -> &'static [&'static str] {
    match language {
        Language::TypeScript | Language::JavaScript => &["--stdio"],
        _ => &[],
    }
}

/// First candidate found on `PATH`: `path_env` is the raw `PATH` value so
/// tests can pass a fake one; the host passes its real environment. On
/// Windows each stem is also probed with every `PATHEXT` extension, since
/// `PATH` entries hold `rust-analyzer.exe` rather than `rust-analyzer`.
#[must_use]
pub fn find_server(candidates: &[&str], path_env: &str) -> Option<PathBuf> {
    find_server_with(candidates, path_env, &platform_extensions())
}

/// [`find_server`] with explicit executable extensions: the seam that
/// lets non-Windows tests prove `.exe` discovery end to end.
fn find_server_with(candidates: &[&str], path_env: &str, extensions: &[String]) -> Option<PathBuf> {
    #[cfg(windows)]
    const SEPARATOR: char = ';';
    #[cfg(not(windows))]
    const SEPARATOR: char = ':';
    for dir in path_env.split(SEPARATOR) {
        if dir.is_empty() {
            continue;
        }
        for candidate in candidates {
            for name in candidate_file_names(candidate, extensions) {
                let full = Path::new(dir).join(name);
                if full.is_file() {
                    return Some(full);
                }
            }
        }
    }
    None
}

/// File names to probe for one server stem: the stem itself, plus the
/// stem with each executable extension. A stem that already carries an
/// extension is used as-is, so explicit `rust-analyzer.exe` candidates
/// never gain a doubled suffix.
fn candidate_file_names(candidate: &str, extensions: &[String]) -> Vec<String> {
    if extensions.is_empty() || Path::new(candidate).extension().is_some() {
        return vec![candidate.to_owned()];
    }
    extensions
        .iter()
        .map(|extension| format!("{candidate}{extension}"))
        .collect()
}

/// Executable extensions to probe alongside the bare stem: `PATHEXT` on
/// Windows (mirroring the host resolver), none elsewhere. Empty means
/// the bare stem is the only name.
fn platform_extensions() -> Vec<String> {
    #[cfg(windows)]
    {
        std::env::var_os("PATHEXT")
            .map(|value| {
                std::env::split_paths(&value)
                    .filter_map(|path| {
                        let extension = path.to_str()?.trim().to_owned();
                        (!extension.is_empty()).then_some(extension)
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|extensions| !extensions.is_empty())
            .unwrap_or_else(|| {
                [".COM", ".EXE", ".BAT", ".CMD"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect()
            })
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// `file://` URI for a path, percent-encoding special characters.
/// `None` when the path cannot be represented as a file URI.
#[must_use]
pub fn file_uri(path: &Path) -> Option<Url> {
    Url::from_file_path(path).ok()
}

/// Local path for a `file://` URI string. `None` for non-file URIs or
/// unparsable input (the caller keeps the URI for display).
#[must_use]
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    Url::parse(uri).ok()?.to_file_path().ok()
}

/// Buffer char column to LSP UTF-16 units for one line. Astral chars
/// count 2, everything else 1; clamps at the line end.
#[must_use]
pub fn char_col_to_utf16(line: &str, char_col: usize) -> u32 {
    let mut units = 0usize;
    let mut seen = 0usize;
    for c in line.chars() {
        if seen >= char_col {
            break;
        }
        units = units.saturating_add(c.len_utf16());
        seen = seen.saturating_add(1);
    }
    u32::try_from(units).unwrap_or(u32::MAX)
}

/// LSP UTF-16 units back to a buffer char column for one line. Splits
/// inside a surrogate pair snap to the pair start; clamps at line end.
#[must_use]
pub fn utf16_to_char_col(line: &str, units: u32) -> usize {
    let mut used = 0u32;
    let mut col = 0usize;
    for c in line.chars() {
        let width = u32::try_from(c.len_utf16()).unwrap_or(u32::MAX);
        if used.saturating_add(width) > units {
            break;
        }
        used = used.saturating_add(width);
        col = col.saturating_add(1);
    }
    col
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body_of(framed: &[u8]) -> Value {
        let mut decoder = FrameDecoder::default();
        let bodies = decoder.feed(framed);
        assert_eq!(bodies.len(), 1);
        let body = bodies.into_iter().next().unwrap_or_default();
        serde_json::from_slice(&body).expect("valid json")
    }

    static NULL: Value = Value::Null;

    fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
        value.get(key).unwrap_or(&NULL)
    }

    fn text<'a>(value: &'a Value, key: &str) -> &'a str {
        field(value, key).as_str().unwrap_or("")
    }

    fn number(value: &Value, key: &str) -> i64 {
        field(value, key).as_i64().unwrap_or(-1)
    }

    #[test]
    fn framing_round_trips_one_byte_chunks() {
        let body = br#"{"jsonrpc":"2.0","method":"exit","params":{}}"#;
        let framed = frame(body);
        let mut decoder = FrameDecoder::default();
        let mut bodies = Vec::new();
        for chunk in framed.chunks(1) {
            bodies.extend(decoder.feed(chunk));
        }
        assert_eq!(bodies, vec![body.to_vec()]);
    }

    #[test]
    fn decoder_waits_for_full_body() {
        let framed = frame(b"{}");
        let mut decoder = FrameDecoder::default();
        let split = framed.len().saturating_sub(1);
        let (head, tail) = framed.split_at(split);
        assert!(decoder.feed(head).is_empty());
        assert_eq!(decoder.feed(tail).len(), 1);
    }

    #[test]
    fn decoder_drains_concatenated_frames() {
        let first = frame(br#"{"jsonrpc":"2.0","id":0,"result":{}}"#);
        let second = frame(br#"{"jsonrpc":"2.0","method":"m","params":{}}"#);
        let mut stream = first.clone();
        stream.extend_from_slice(&second);
        // One feed with both frames glued together …
        let mut decoder = FrameDecoder::default();
        let bodies = decoder.feed(&stream);
        assert_eq!(bodies.len(), 2);
        // … and split at every possible byte offset.
        for at in 0..stream.len() {
            let mut decoder = FrameDecoder::default();
            let (head, tail) = stream.split_at(at);
            let mut bodies = decoder.feed(head);
            bodies.extend(decoder.feed(tail));
            assert_eq!(bodies.len(), 2, "split at {at}");
        }
    }

    #[test]
    fn decoder_skips_header_without_length() {
        let mut stream = b"X-Noise: yes\r\n\r\n".to_vec();
        stream.extend_from_slice(&frame(b"{}"));
        let mut decoder = FrameDecoder::default();
        assert_eq!(decoder.feed(&stream), vec![b"{}".to_vec()]);
    }

    #[test]
    fn classifies_request_notification_response() {
        let request = parse_message(br#"{"jsonrpc":"2.0","id":1,"method":"m","params":{"a":1}}"#);
        assert!(matches!(
            request,
            Some(LspMessage::Request { method, .. }) if method == "m"
        ));
        let notification = parse_message(br#"{"jsonrpc":"2.0","method":"m","params":null}"#);
        assert!(matches!(
            notification,
            Some(LspMessage::Notification { method, .. }) if method == "m"
        ));
        let response = parse_message(br#"{"jsonrpc":"2.0","id":"x","result":1,"error":null}"#);
        assert!(matches!(
            response,
            Some(LspMessage::Response {
                id: RequestId::Text(_),
                ..
            })
        ));
        assert!(parse_message(b"{}").is_none());
        assert!(parse_message(b"not json").is_none());
    }

    #[test]
    fn error_response_carries_code_and_message() {
        let message = parse_message(
            br#"{"jsonrpc":"2.0","id":2,"result":null,
                "error":{"code":-32601,"message":"gone"}}"#,
        );
        match message {
            Some(LspMessage::Response {
                error: Some(error), ..
            }) => {
                assert_eq!(error.code, -32601);
                assert_eq!(error.message, "gone");
            }
            other => panic!("expected error response, got {other:?}"),
        }
    }

    #[test]
    fn initialize_carries_root_and_utf16() {
        let root = Url::parse("file:///repo").expect("root");
        let mut ids = IdGen::default();
        let value = body_of(&initialize(&ids.next_id(), 7, &root));
        assert_eq!(text(&value, "method"), "initialize");
        let params = field(&value, "params");
        assert_eq!(text(params, "rootUri"), "file:///repo");
        assert_eq!(number(params, "processId"), 7);
        let general = field(field(params, "capabilities"), "general");
        let encodings = field(general, "positionEncodings");
        let first = encodings.get(0).and_then(Value::as_str).unwrap_or("");
        assert_eq!(first, "utf-16");
    }

    #[test]
    fn sync_notifications_carry_uri_version_text() {
        let uri = Url::parse("file:///repo/main.rs").expect("uri");
        let opened = body_of(&did_open(&uri, "rust", 3, "fn f() {}"));
        assert_eq!(text(&opened, "method"), "textDocument/didOpen");
        let document = field(field(&opened, "params"), "textDocument");
        assert_eq!(number(document, "version"), 3);
        assert_eq!(text(document, "text"), "fn f() {}");

        let changed = body_of(&did_change(&uri, 4, "fn g() {}"));
        assert_eq!(text(&changed, "method"), "textDocument/didChange");
        let changes = field(field(&changed, "params"), "contentChanges");
        let first = changes.get(0).unwrap_or(&NULL);
        assert_eq!(text(first, "text"), "fn g() {}");

        let closed = body_of(&did_close(&uri));
        assert_eq!(text(&closed, "method"), "textDocument/didClose");
    }

    #[test]
    fn hover_and_definition_carry_utf16_position() {
        let uri = Url::parse("file:///repo/main.rs").expect("uri");
        let mut ids = IdGen::default();
        let at = Position {
            line: 4,
            character: 9,
        };
        let asking = body_of(&hover(&ids.next_id(), &uri, at));
        assert_eq!(text(&asking, "method"), "textDocument/hover");
        let position = field(field(&asking, "params"), "position");
        assert_eq!(number(position, "line"), 4);
        assert_eq!(number(position, "character"), 9);
        let jumping = body_of(&definition(&ids.next_id(), &uri, at));
        assert_eq!(text(&jumping, "method"), "textDocument/definition");
        assert_eq!(number(&jumping, "id"), 1);
    }

    #[test]
    fn references_request_and_locations_keep_utf16_positions() {
        let uri = Url::parse("file:///repo/main.rs").expect("uri");
        let at = Position {
            line: 4,
            character: 9,
        };
        let request = body_of(&references(&RequestId::Number(7), &uri, at));
        assert_eq!(text(&request, "method"), "textDocument/references");
        let params = field(&request, "params");
        assert_eq!(number(field(params, "position"), "character"), 9);
        assert_eq!(
            field(field(params, "context"), "includeDeclaration"),
            &Value::Bool(true)
        );

        let locations = serde_json::json!([{
            "uri": "file:///repo/lib.rs",
            "range": {
                "start": { "line": 2, "character": 4 },
                "end": { "line": 2, "character": 8 }
            }
        }]);
        let parsed = parse_references(&locations);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].uri, "file:///repo/lib.rs");
        assert_eq!(
            parsed[0].range.start,
            Position {
                line: 2,
                character: 4
            }
        );
        assert!(parse_references(&Value::Null).is_empty());
    }

    #[test]
    fn rename_parses_multifile_text_edits_and_rejects_file_operations() {
        let uri = Url::parse("file:///repo/main.rs").unwrap();
        let request = body_of(&rename(
            &RequestId::Number(4),
            &uri,
            Position {
                line: 1,
                character: 5,
            },
            "new_name",
        ));
        assert_eq!(text(&request, "method"), "textDocument/rename");
        assert_eq!(text(field(&request, "params"), "newName"), "new_name");

        let edit = serde_json::json!({
            "documentChanges": [{
                "textDocument": { "uri": "file:///repo/main.rs", "version": 3 },
                "edits": [{
                    "range": {
                        "start": { "line": 1, "character": 5 },
                        "end": { "line": 1, "character": 9 }
                    },
                    "newText": "new_name"
                }]
            }]
        });
        let files = parse_rename(&edit).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].version, Some(3));
        assert_eq!(files[0].edits[0].new_text, "new_name");

        let operation = serde_json::json!({
            "documentChanges": [{
                "kind": "rename",
                "oldUri": "file:///repo/a.rs",
                "newUri": "file:///repo/b.rs"
            }]
        });
        assert!(parse_rename(&operation).is_err());
    }

    #[test]
    fn workspace_symbol_request_and_responses_keep_file_targets() {
        let request = body_of(&workspace_symbol(&RequestId::Number(9), "helper"));
        assert_eq!(text(&request, "method"), "workspace/symbol");
        assert_eq!(text(field(&request, "params"), "query"), "helper");
        let flat = serde_json::json!([{
            "name": "helper",
            "kind": 12,
            "containerName": "module",
            "location": {
                "uri": "file:///repo/src/lib.rs",
                "range": {
                    "start": { "line": 3, "character": 4 },
                    "end": { "line": 3, "character": 10 }
                }
            }
        }]);
        let hits = parse_workspace_symbols(&flat);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "helper");
        assert_eq!(hits[0].container.as_deref(), Some("module"));
        assert_eq!(hits[0].range.start.line, 3);
        assert_eq!(hits[0].uri, "file:///repo/src/lib.rs");

        let unresolved = serde_json::json!([{
            "name": "unresolved",
            "kind": 12,
            "location": { "uri": "file:///repo/src/lib.rs" }
        }]);
        assert!(parse_workspace_symbols(&unresolved).is_empty());
    }

    #[test]
    fn publishes_diagnostics_with_severity() {
        let params = serde_json::json!({
            "uri": "file:///repo/main.rs",
            "diagnostics": [
                {
                    "range": {"start": {"line": 1, "character": 2},
                              "end": {"line": 1, "character": 9}},
                    "severity": 1,
                    "message": "cannot find value `x`",
                },
                {
                    "range": {"start": {"line": 4, "character": 0},
                              "end": {"line": 4, "character": 1}},
                    "message": "unused, no severity",
                },
            ],
        });
        let (uri, items) = parse_diagnostics(&params).expect("diagnostics");
        assert_eq!(uri, "file:///repo/main.rs");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].severity, Some(Severity::Error));
        assert_eq!(
            items[0].range.start,
            Position {
                line: 1,
                character: 2
            }
        );
        assert_eq!(items[1].severity, None);
    }

    #[test]
    fn hover_shapes_flatten_to_text() {
        assert!(parse_hover(&Value::Null).is_none());
        let markup = serde_json::json!({
            "contents": {"kind": "markdown", "value": "# f\n```rust\nfn f()\n```"}
        });
        assert_eq!(
            parse_hover(&markup).expect("markup").text,
            "# f\n```rust\nfn f()\n```"
        );
        assert!(parse_hover(&markup).expect("markup").markdown);
        let plaintext = serde_json::json!({
            "contents": {"kind": "plaintext", "value": "# f"}
        });
        assert!(!parse_hover(&plaintext).expect("plaintext").markdown);
        let scalar = serde_json::json!({"contents": "plain hover"});
        assert_eq!(parse_hover(&scalar).expect("scalar").text, "plain hover");
        let array = serde_json::json!({
            "contents": [
                {"language": "rust", "value": "fn f()"},
                "docs here",
            ]
        });
        assert_eq!(
            parse_hover(&array).expect("array").text,
            "```rust\nfn f()\n```\n\ndocs here"
        );
    }

    #[test]
    fn definition_shapes_land_on_targets() {
        assert!(parse_definition(&Value::Null).is_empty());
        let single = serde_json::json!({
            "uri": "file:///repo/a.rs",
            "range": {"start": {"line": 0, "character": 0},
                      "end": {"line": 0, "character": 5}},
        });
        let targets = parse_definition(&single);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].uri, "file:///repo/a.rs");

        let link = serde_json::json!([{
            "originSelectionRange": {
                "start": {"line": 3, "character": 1},
                "end": {"line": 3, "character": 2}},
            "targetUri": "file:///repo/b.rs",
            "targetRange": {"start": {"line": 9, "character": 0},
                            "end": {"line": 9, "character": 4}},
            "targetSelectionRange": {
                "start": {"line": 9, "character": 0},
                "end": {"line": 9, "character": 4}},
        }]);
        let targets = parse_definition(&link);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].uri, "file:///repo/b.rs");
        assert_eq!(targets[0].range.start.line, 9);
    }

    #[test]
    fn document_symbols_flatten_nested_and_flat_shapes() {
        assert!(parse_document_symbols(&Value::Null).is_empty());
        let nested = serde_json::json!([{
            "name": "main",
            "kind": 12,
            "range": {"start": {"line": 0, "character": 0},
                      "end": {"line": 9, "character": 1}},
            "selectionRange": {"start": {"line": 0, "character": 3},
                               "end": {"line": 0, "character": 7}},
            "children": [{
                "name": "helper",
                "kind": 12,
                "range": {"start": {"line": 2, "character": 1},
                          "end": {"line": 4, "character": 2}},
                "selectionRange": {"start": {"line": 2, "character": 5},
                                   "end": {"line": 2, "character": 11}},
            }],
        }]);
        let symbols = parse_document_symbols(&nested);
        assert_eq!(symbols.len(), 2);
        assert_eq!(symbols[0].name, "main");
        assert_eq!(symbols[0].depth, 0);
        assert_eq!(symbols[1].name, "helper");
        assert_eq!(symbols[1].depth, 1);
        assert_eq!(symbols[1].range.start.line, 2);
        // Jumps land on the name, not the item's first line.
        assert_eq!(symbols[1].selection.start.character, 5);

        let flat = serde_json::json!([{
            "name": "Thing",
            "kind": 5,
            "location": {
                "uri": "file:///repo/a.rs",
                "range": {"start": {"line": 3, "character": 0},
                          "end": {"line": 5, "character": 1}},
            },
        }]);
        let symbols = parse_document_symbols(&flat);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].kind, "Class");
        assert_eq!(symbols[0].depth, 0);
    }

    #[test]
    fn code_actions_keep_editable_ones_preferred_first() {
        let actions = serde_json::json!([
            { "title": "Run command", "command": { "title": "x", "command": "x" } },
            { "title": "Extract variable", "kind": "refactor.extract", "data": 1 },
            { "title": "Add missing import", "kind": "quickfix", "isPreferred": true,
              "edit": { "changes": { "file:///a.rs": [
                  { "range": { "start": { "line": 0, "character": 0 },
                               "end": { "line": 0, "character": 0 } },
                    "newText": "use x;\n" } ] } } },
        ]);
        let items = parse_code_actions(&actions);
        assert_eq!(
            items
                .iter()
                .map(|item| item.title.as_str())
                .collect::<Vec<_>>(),
            ["Add missing import", "Extract variable"]
        );
        let edits = code_action_edits(&items[0].raw).unwrap();
        assert_eq!(edits[0].edits[0].new_text, "use x;\n");
        assert!(code_action_edits(&items[1].raw).is_err());
        let formatted = parse_text_edits(&serde_json::json!([
            { "range": { "start": { "line": 1, "character": 0 },
                         "end": { "line": 1, "character": 2 } }, "newText": "    " }
        ]));
        assert_eq!(formatted.len(), 1);
        assert!(parse_text_edits(&Value::Null).is_empty());
    }

    #[test]
    fn inlay_hints_parse_string_and_part_labels_in_position_order() {
        let request = body_of(&inlay_hint(
            &RequestId::Number(4),
            &Url::parse("file:///tmp/a.rs").unwrap(),
            12,
        ));
        assert_eq!(text(&request, "method"), "textDocument/inlayHint");
        assert_eq!(request["params"]["range"]["end"]["line"], 12);
        let result = serde_json::json!([
            {
                "position": { "line": 3, "character": 9 },
                "label": [{ "value": "a" }, { "value": ":" }],
                "paddingRight": true,
            },
            { "position": { "line": 1, "character": 5 }, "label": ": i32", "kind": 1 },
            { "position": { "line": 2, "character": 0 }, "label": "  " },
            { "position": { "line": 2 }, "label": "broken" },
        ]);
        assert_eq!(
            parse_inlay_hints(&result),
            [
                InlayHint {
                    position: Position {
                        line: 1,
                        character: 5
                    },
                    label: ": i32".into(),
                },
                InlayHint {
                    position: Position {
                        line: 3,
                        character: 9
                    },
                    label: "a: ".into(),
                },
            ]
        );
        assert!(parse_inlay_hints(&Value::Null).is_empty());
    }

    #[test]
    fn hierarchy_requests_and_answers_round_trip_items() {
        let uri = Url::parse("file:///tmp/a.rs").unwrap();
        let at = Position {
            line: 3,
            character: 4,
        };
        let prepare = body_of(&prepare_hierarchy(
            &RequestId::Number(1),
            &uri,
            at,
            HierarchyKind::Call,
        ));
        assert_eq!(
            text(&prepare, "method"),
            "textDocument/prepareCallHierarchy"
        );
        let item = serde_json::json!({
            "name": "helper",
            "kind": 12,
            "detail": "fn helper()",
            "uri": "file:///tmp/a.rs",
            "range": { "start": { "line": 1, "character": 0 }, "end": { "line": 3, "character": 1 } },
            "selectionRange": { "start": { "line": 1, "character": 3 }, "end": { "line": 1, "character": 9 } },
            "data": 7,
        });
        let prepared = parse_hierarchy_items(&serde_json::json!([item.clone()]));
        assert_eq!(prepared.len(), 1);
        assert_eq!(prepared[0].name, "helper");
        assert_eq!(prepared[0].kind, "Function");
        assert_eq!(prepared[0].selection.start.character, 3);
        // Children send the server's own item back.
        for (kind, direction, method) in [
            (
                HierarchyKind::Call,
                HierarchyDirection::Up,
                "callHierarchy/incomingCalls",
            ),
            (
                HierarchyKind::Call,
                HierarchyDirection::Down,
                "callHierarchy/outgoingCalls",
            ),
            (
                HierarchyKind::Type,
                HierarchyDirection::Up,
                "typeHierarchy/supertypes",
            ),
            (
                HierarchyKind::Type,
                HierarchyDirection::Down,
                "typeHierarchy/subtypes",
            ),
        ] {
            let body = body_of(&hierarchy_children(
                &RequestId::Number(2),
                &prepared[0].raw,
                kind,
                direction,
            ));
            assert_eq!(text(&body, "method"), method);
            assert_eq!(body["params"]["item"]["data"], 7);
        }
        // Calls wrap items in `from` / `to`.
        let incoming = serde_json::json!([{ "from": item, "fromRanges": [] }]);
        assert_eq!(parse_hierarchy_items(&incoming)[0].name, "helper");
        assert!(parse_hierarchy_items(&Value::Null).is_empty());
        let caps =
            serde_json::json!({ "callHierarchyProvider": true, "typeHierarchyProvider": {} });
        assert!(supports_hierarchy(&caps, HierarchyKind::Call));
        assert!(supports_hierarchy(&caps, HierarchyKind::Type));
        assert!(!supports_hierarchy(
            &serde_json::json!({}),
            HierarchyKind::Call
        ));
    }

    #[test]
    fn signature_help_marks_the_active_parameter() {
        let help = serde_json::json!({
            "signatures": [{
                "label": "fn push(&mut self, value: T)",
                "parameters": [{"label": "&mut self"}, {"label": [19, 27]}],
            }],
            "activeSignature": 0,
            "activeParameter": 1,
        });
        let parsed = parse_signature_help(&help).expect("signature");
        assert_eq!(parsed.label.get(19..27), Some("value: T"));
        assert_eq!(parsed.active, Some(19..27));
        let simple = serde_json::json!({
            "signatures": [{"label": "f(a, b)", "parameters": [{"label": "a"}, {"label": "b"}]}],
            "activeParameter": 1,
        });
        assert_eq!(parse_signature_help(&simple).unwrap().active, Some(5..6));
        assert!(parse_signature_help(&serde_json::json!({"signatures": []})).is_none());
    }

    #[test]
    fn completion_lists_keep_edits_snippets_and_sort_keys() {
        let list = serde_json::json!({
            "isIncomplete": false,
            "items": [
                {
                    "label": "push",
                    "kind": 2,
                    "detail": "fn(&mut self, T)",
                    "insertTextFormat": 2,
                    "sortText": "0001",
                    "textEdit": {
                        "range": {"start": {"line": 3, "character": 6},
                                  "end": {"line": 3, "character": 8}},
                        "newText": "push(${1:value})$0",
                    },
                },
                { "label": "len" },
            ],
        });
        let items = parse_completion(&list);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].kind.as_deref(), Some("Method"));
        assert!(items[0].snippet);
        assert_eq!(items[0].insert, "push(${1:value})$0");
        assert_eq!(items[0].edit.map(|range| range.start.character), Some(6));
        assert_eq!(items[0].sort, "0001");
        // Bare items fall back to their label.
        assert_eq!(items[1].insert, "len");
        assert_eq!(items[1].filter, "len");
        assert!(parse_completion(&Value::Null).is_empty());
    }

    #[test]
    fn snippets_expand_to_text_with_the_caret_on_the_first_stop() {
        assert_eq!(
            expand_snippet("push(${1:value})$0"),
            ("push(value)".into(), 5)
        );
        assert_eq!(expand_snippet("foo($0)"), ("foo()".into(), 4));
        assert_eq!(expand_snippet("plain"), ("plain".into(), 5));
        assert_eq!(expand_snippet("\\$x ${1|a,b|}"), ("$x a".into(), 4));
    }

    #[test]
    fn document_symbols_keep_detail_and_deprecation() {
        let nested = serde_json::json!([{
            "name": "old",
            "detail": "fn() -> u8",
            "kind": 12,
            "tags": [1],
            "range": {"start": {"line": 0, "character": 0},
                      "end": {"line": 0, "character": 9}},
            "selectionRange": {"start": {"line": 0, "character": 3},
                               "end": {"line": 0, "character": 6}},
        }]);
        let symbols = parse_document_symbols(&nested);
        assert_eq!(symbols[0].detail.as_deref(), Some("fn() -> u8"));
        assert!(symbols[0].deprecated);
    }

    #[test]
    fn markdown_outline_nests_headings_and_spans_sections() {
        let doc = crate::doc::Doc::new("# A\ntext\n## B\nmore\n### C\n## D\n# E\n");
        let outline = markdown_outline(&doc);
        let shape: Vec<(&str, usize, u32, u32)> = outline
            .iter()
            .map(|s| {
                (
                    s.name.as_str(),
                    s.depth,
                    s.range.start.line,
                    s.range.end.line,
                )
            })
            .collect();
        assert_eq!(
            shape,
            [
                ("A", 0, 0, 5),
                ("B", 1, 2, 4),
                ("C", 2, 4, 4),
                ("D", 1, 5, 5),
                ("E", 0, 6, 7),
            ]
        );
        // The caret in B's body is inside B, not A.
        assert_eq!(innermost_symbol(&outline, 3), Some(1));
        assert_eq!(innermost_symbol(&outline, 4), Some(2));
    }

    #[test]
    fn server_table_covers_rust_python_and_typescript() {
        assert_eq!(server_candidates(Language::Rust), &["rust-analyzer"]);
        assert_eq!(server_candidates(Language::Python), &["pylsp"]);
        assert_eq!(
            server_candidates(Language::TypeScript),
            &["typescript-language-server"]
        );
        assert_eq!(
            server_candidates(Language::JavaScript),
            &["typescript-language-server"]
        );
        assert!(server_candidates(Language::Go).is_empty());
        assert_eq!(server_language(Language::Rust), Some("rust"));
        assert_eq!(server_language(Language::TypeScript), Some("typescript"));
        assert_eq!(server_language(Language::JavaScript), Some("typescript"));
        assert_eq!(server_language(Language::Markdown), None);
        assert_eq!(server_args(Language::TypeScript), &["--stdio"]);
        assert!(server_args(Language::Rust).is_empty());
    }

    #[test]
    fn typescript_file_ids_follow_the_extension() {
        let id = |name: &str| language_id_for_path(Path::new(name), Language::TypeScript);
        assert_eq!(id("app.ts"), Some("typescript"));
        assert_eq!(id("view.tsx"), Some("typescriptreact"));
        assert_eq!(id("config.mts"), Some("typescript"));
        assert_eq!(
            language_id_for_path(Path::new("main.js"), Language::JavaScript),
            Some("javascript")
        );
        assert_eq!(
            language_id_for_path(Path::new("view.jsx"), Language::JavaScript),
            Some("javascriptreact")
        );
        assert_eq!(
            language_id_for_path(Path::new("notes.md"), Language::Markdown),
            None
        );
    }

    #[test]
    fn path_probe_finds_fake_server() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fake = dir
            .path()
            .join(format!("rust-analyzer{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&fake, "#!/bin/sh\n").expect("write fake");
        let found = find_server(&["rust-analyzer"], &dir.path().to_string_lossy());
        // PATHEXT can spell .EXE differently from the file's .exe suffix.
        assert_eq!(
            found
                .expect("fake server")
                .canonicalize()
                .expect("found path"),
            fake.canonicalize().expect("fixture path")
        );
        assert!(find_server(&["rust-analyzer"], "/nonexistent-dir-xyz").is_none());
        assert!(find_server(&["rust-analyzer"], "").is_none());
    }

    #[test]
    fn exe_stems_resolve_through_extension_probing() {
        // Windows `PATH` entries hold `rust-analyzer.exe`, never the bare
        // stem: prove the probing finds it using explicit extensions so
        // the test holds on every platform.
        let extensions = [".EXE", ".BAT"].map(str::to_owned);
        let dir = tempfile::tempdir().expect("tempdir");
        let exe = dir.path().join("rust-analyzer.EXE");
        std::fs::write(&exe, "fake").expect("write fake");
        let path_env = dir.path().to_string_lossy().into_owned();
        assert_eq!(
            find_server_with(&["rust-analyzer"], &path_env, &extensions),
            Some(exe)
        );
        // A stem that already carries an extension is used as-is: no
        // doubled suffix, no probing.
        assert_eq!(
            candidate_file_names("rust-analyzer.EXE", &extensions),
            vec!["rust-analyzer.EXE".to_owned()]
        );
        // Unknown stems still miss, even with extensions configured.
        assert!(find_server_with(&["pylsp"], &path_env, &extensions).is_none());
        assert!(find_server_with(&["rust-analyzer"], "", &extensions).is_none());
    }

    #[test]
    fn utf16_math_round_trips_astral_chars() {
        // `a` + crab (2 units) + `b`: char cols 0..3, unit cols 0..4.
        let line = "a🦀b";
        assert_eq!(char_col_to_utf16(line, 0), 0);
        assert_eq!(char_col_to_utf16(line, 1), 1);
        assert_eq!(char_col_to_utf16(line, 2), 3);
        assert_eq!(char_col_to_utf16(line, 3), 4);
        assert_eq!(char_col_to_utf16(line, 99), 4);
        // Splitting inside the surrogate pair snaps to the pair start.
        assert_eq!(utf16_to_char_col(line, 0), 0);
        assert_eq!(utf16_to_char_col(line, 1), 1);
        assert_eq!(utf16_to_char_col(line, 2), 1);
        assert_eq!(utf16_to_char_col(line, 3), 2);
        assert_eq!(utf16_to_char_col(line, 99), 3);
    }

    #[test]
    fn file_uri_round_trips_special_chars() {
        let path = std::env::temp_dir().join("some dir/f Gö.rs");
        let uri = file_uri(&path).expect("uri");
        assert_eq!(uri.scheme(), "file");
        assert!(uri.as_str().ends_with("/some%20dir/f%20G%C3%B6.rs"));
        assert_eq!(uri_to_path(uri.as_str()), Some(path));
        assert!(file_uri(Path::new("relative/file.rs")).is_none());
        assert!(uri_to_path("https://example.com/x").is_none());
        assert!(uri_to_path(":::").is_none());
    }
}
