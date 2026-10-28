use super::*;

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

/// Inputs for [`did_open`].
pub struct DidOpenInput<'a> {
    pub uri: &'a Url,
    pub language_id: &'a str,
    pub version: i32,
    pub text: &'a str,
}

/// Full-text `didOpen` for one buffer.
#[must_use]
pub fn did_open(input: DidOpenInput<'_>) -> Vec<u8> {
    let DidOpenInput {
        uri,
        language_id,
        version,
        text,
    } = input;
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
