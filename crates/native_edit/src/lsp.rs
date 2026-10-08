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

/// Flattened hover text (markdown or plaintext, markup stripped).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Hover {
    pub text: String,
}

/// One go-to-definition target: server URI plus range.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct GotoTarget {
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
                "textDocument": {
                    "synchronization": { "didSave": false },
                    "hover": { "contentFormat": ["markdown", "plaintext"] },
                    "definition": { "linkSupport": true },
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

/// Parse a `textDocument/hover` result into flat text. `None` means null
/// or an unrecognized shape (no hover available).
#[must_use]
pub fn parse_hover(result: &Value) -> Option<Hover> {
    if result.is_null() {
        return None;
    }
    let parsed: lsp_types::Hover = serde_json::from_value(result.clone()).ok()?;
    let text = match parsed.contents {
        lsp_types::HoverContents::Scalar(marked) => marked_text(&marked),
        lsp_types::HoverContents::Array(marked) => marked
            .iter()
            .map(marked_text)
            .collect::<Vec<_>>()
            .join("\n"),
        lsp_types::HoverContents::Markup(content) => content.value,
    };
    Some(Hover { text })
}

fn marked_text(value: &lsp_types::MarkedString) -> String {
    match value {
        lsp_types::MarkedString::String(text) => text.clone(),
        lsp_types::MarkedString::LanguageString(content) => content.value.clone(),
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

fn native_location(uri: &lsp_types::Uri, range: &lsp_types::Range) -> GotoTarget {
    GotoTarget {
        uri: uri.as_str().to_owned(),
        range: native_range(range),
    }
}

/// Protocol language id for `didOpen`, if the language has a server.
#[must_use]
pub fn language_id(language: Language) -> Option<&'static str> {
    match language {
        Language::Rust => Some("rust"),
        Language::Python => Some("python"),
        _ => None,
    }
}

/// Server binary candidates for a language, in preference order. Only
/// languages here get a server; everything else edits without LSP.
#[must_use]
pub fn server_candidates(language: Language) -> &'static [&'static str] {
    match language {
        Language::Rust => &["rust-analyzer"],
        Language::Python => &["pylsp"],
        _ => &[],
    }
}

/// First candidate found on `PATH`: `path_env` is the raw `PATH` value so
/// tests can pass a fake one; the host passes its real environment.
#[must_use]
pub fn find_server(candidates: &[&str], path_env: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    const SEPARATOR: char = ';';
    #[cfg(not(windows))]
    const SEPARATOR: char = ':';
    for dir in path_env.split(SEPARATOR) {
        if dir.is_empty() {
            continue;
        }
        for candidate in candidates {
            let full = Path::new(dir).join(candidate);
            if full.is_file() {
                return Some(full);
            }
        }
    }
    None
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
            "fn f()\ndocs here"
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
    fn server_table_lists_rust_and_python_only() {
        assert_eq!(server_candidates(Language::Rust), &["rust-analyzer"]);
        assert_eq!(server_candidates(Language::Python), &["pylsp"]);
        assert!(server_candidates(Language::TypeScript).is_empty());
        assert_eq!(language_id(Language::Rust), Some("rust"));
        assert_eq!(language_id(Language::Markdown), None);
    }

    #[test]
    fn path_probe_finds_fake_server() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fake = dir.path().join("rust-analyzer");
        std::fs::write(&fake, "#!/bin/sh\n").expect("write fake");
        let found = find_server(&["rust-analyzer"], &dir.path().to_string_lossy());
        assert_eq!(found, Some(fake));
        assert!(find_server(&["rust-analyzer"], "/nonexistent-dir-xyz").is_none());
        assert!(find_server(&["rust-analyzer"], "").is_none());
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
        let path = Path::new("/tmp/some dir/f Gö.rs");
        let uri = file_uri(path).expect("uri");
        assert_eq!(uri.as_str(), "file:///tmp/some%20dir/f%20G%C3%B6.rs");
        assert_eq!(uri_to_path(uri.as_str()), Some(path.to_path_buf()));
        assert!(uri_to_path("https://example.com/x").is_none());
        assert!(uri_to_path(":::").is_none());
    }
}
