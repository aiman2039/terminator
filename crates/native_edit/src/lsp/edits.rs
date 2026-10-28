//! Formatting and code actions: requests and their edit results.
use super::*;

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
