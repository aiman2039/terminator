use super::*;

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
