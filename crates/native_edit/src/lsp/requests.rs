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

pub(super) fn completion_kind_label(kind: lsp_types::CompletionItemKind) -> String {
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

pub(super) fn push_first_choice(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    out: &mut String,
) {
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
                        push_first_choice(&mut chars, &mut out);
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
