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

/// Inputs for [`code_action`].
pub struct CodeActionInput<'a> {
    pub id: &'a RequestId,
    pub uri: &'a Url,
    pub range: Range,
    pub diagnostics: &'a [Diagnostic],
}

/// `textDocument/codeAction` for a range, with the diagnostics touching it.
#[must_use]
pub fn code_action(input: CodeActionInput<'_>) -> Vec<u8> {
    let CodeActionInput {
        id,
        uri,
        range,
        diagnostics,
    } = input;
    let position = |p: Position| serde_json::json!({ "line": p.line, "character": p.character });
    let diagnostics: Vec<Value> = diagnostics
        .iter()
        .map(|item| {
            let severity = match item.severity {
                Some(Severity::Error) => 1,
                Some(Severity::Warning) => 2,
                Some(Severity::Information) => 3,
                Some(Severity::Hint | Severity::Spelling) | None => 4,
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
    /// Offered by the spell-check server: commands run there, not on the
    /// file's language server.
    pub spell: bool,
}

/// Parse `textDocument/codeAction`: actions that carry an edit or can be
/// resolved to one. Bare commands need `workspace/executeCommand`, which
/// this client does not run, so they are left out.
#[must_use]
pub fn parse_code_actions(result: &Value) -> Vec<CodeActionItem> {
    let mut out = action_items(result, false, |item| item.get("data").is_some());
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

/// Parse the spell server's `textDocument/codeAction`: replacement fixes
/// (with an edit) plus bare commands such as "add to dictionary", which
/// [`action_command`] turns into `workspace/executeCommand`. Fixes keep
/// the server's order, so the best suggestion stays first.
#[must_use]
pub fn parse_spell_actions(result: &Value) -> Vec<CodeActionItem> {
    action_items(result, true, |item| action_command(item).is_some())
}

/// Actions with an edit, or that `runnable` accepts, as items.
fn action_items(
    result: &Value,
    spell: bool,
    runnable: impl Fn(&Value) -> bool,
) -> Vec<CodeActionItem> {
    let Some(items) = result.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .filter(|item| item.get("edit").is_some() || runnable(item))
        .filter_map(|item| {
            Some(CodeActionItem {
                title: item.get("title")?.as_str()?.to_owned(),
                kind: item.get("kind").and_then(Value::as_str).map(str::to_owned),
                preferred: item.get("isPreferred").and_then(Value::as_bool) == Some(true),
                raw: item.clone(),
                spell,
            })
        })
        .collect()
}

/// `{command, arguments}` of a bare `Command` or of a code action that
/// carries only a command. `None` for actions with an edit.
#[must_use]
pub fn action_command(action: &Value) -> Option<Value> {
    if action.get("edit").is_some() {
        return None;
    }
    let (name, arguments) = match action.get("command")? {
        Value::String(name) => (name.clone(), action.get("arguments")),
        command => (
            command.get("command")?.as_str()?.to_owned(),
            command.get("arguments"),
        ),
    };
    Some(serde_json::json!({
        "command": name,
        "arguments": arguments.cloned().unwrap_or_else(|| Value::Array(Vec::new())),
    }))
}

/// `workspace/executeCommand` for a value from [`action_command`].
#[must_use]
pub fn execute_command(id: &RequestId, command: &Value) -> Vec<u8> {
    request(id, "workspace/executeCommand", command.clone())
}
