//! Location requests beyond `definition` (implementation, type
//! definition), document highlights, and range / on-type formatting,
//! plus the server capability checks that gate them.
use super::*;

/// Which location request a goto sends. All three answer with the same
/// `Location | Location[] | LocationLink[]` shape as `definition`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Locate {
    Definition,
    Implementation,
    TypeDefinition,
}

impl Locate {
    /// Protocol method name.
    #[must_use]
    pub fn method(self) -> &'static str {
        match self {
            Locate::Definition => "textDocument/definition",
            Locate::Implementation => "textDocument/implementation",
            Locate::TypeDefinition => "textDocument/typeDefinition",
        }
    }

    /// Server capability that advertises the request.
    #[must_use]
    pub fn provider(self) -> &'static str {
        match self {
            Locate::Definition => "definitionProvider",
            Locate::Implementation => "implementationProvider",
            Locate::TypeDefinition => "typeDefinitionProvider",
        }
    }

    /// Short user-facing name ("implementation").
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Locate::Definition => "definition",
            Locate::Implementation => "implementation",
            Locate::TypeDefinition => "type declaration",
        }
    }
}

/// Whether `initialize` capabilities advertise `provider` (`true` or an
/// options object; `false`, null, or missing mean no).
#[must_use]
pub fn supports(capabilities: &Value, provider: &str) -> bool {
    match capabilities.get(provider) {
        Some(Value::Bool(on)) => *on,
        Some(Value::Object(_)) => true,
        _ => false,
    }
}

/// Characters that trigger `textDocument/onTypeFormatting`, when the
/// server advertises it (first plus "more" trigger characters).
#[must_use]
pub fn on_type_triggers(capabilities: &Value) -> Vec<char> {
    let Some(options) = capabilities.get("documentOnTypeFormattingProvider") else {
        return Vec::new();
    };
    let first = options
        .get("firstTriggerCharacter")
        .and_then(Value::as_str)
        .into_iter();
    let more = options
        .get("moreTriggerCharacter")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str);
    first
        .chain(more)
        .filter_map(|text| {
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Some(c),
                _ => None,
            }
        })
        .collect()
}

fn position_json(position: Position) -> Value {
    serde_json::json!({ "line": position.line, "character": position.character })
}

/// A location request (`definition`, `implementation`, `typeDefinition`)
/// at a UTF-16 position.
#[must_use]
pub fn locate(kind: Locate, id: &RequestId, uri: &Url, position: Position) -> Vec<u8> {
    request(
        id,
        kind.method(),
        serde_json::json!({
            "textDocument": { "uri": uri.as_str() },
            "position": position_json(position),
        }),
    )
}

/// `textDocument/documentHighlight` at a UTF-16 position.
#[must_use]
pub fn document_highlight(id: &RequestId, uri: &Url, position: Position) -> Vec<u8> {
    request(
        id,
        "textDocument/documentHighlight",
        serde_json::json!({
            "textDocument": { "uri": uri.as_str() },
            "position": position_json(position),
        }),
    )
}

/// Parse a `DocumentHighlight[]` answer into ranges, sorted; null or
/// unrecognized shapes yield none.
#[must_use]
pub fn parse_document_highlights(result: &Value) -> Vec<Range> {
    let mut ranges: Vec<Range> =
        serde_json::from_value::<Vec<lsp_types::DocumentHighlight>>(result.clone())
            .unwrap_or_default()
            .iter()
            .map(|item| native_range(&item.range))
            .collect();
    ranges.sort_by_key(|range| (range.start.line, range.start.character));
    ranges
}

/// `textDocument/rangeFormatting` for one range (4-space indents, like
/// [`formatting`]).
#[must_use]
pub fn range_formatting(id: &RequestId, uri: &Url, range: Range) -> Vec<u8> {
    request(
        id,
        "textDocument/rangeFormatting",
        serde_json::json!({
            "textDocument": { "uri": uri.as_str() },
            "range": { "start": position_json(range.start), "end": position_json(range.end) },
            "options": { "tabSize": 4, "insertSpaces": true },
        }),
    )
}

/// `textDocument/onTypeFormatting` after `ch` was typed; `position` is
/// the caret just after it.
#[must_use]
pub fn on_type_formatting(id: &RequestId, uri: &Url, position: Position, ch: char) -> Vec<u8> {
    request(
        id,
        "textDocument/onTypeFormatting",
        serde_json::json!({
            "textDocument": { "uri": uri.as_str() },
            "position": position_json(position),
            "ch": ch.to_string(),
            "options": { "tabSize": 4, "insertSpaces": true },
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(bytes: &[u8]) -> Value {
        let text = String::from_utf8_lossy(bytes);
        let json = text.split("\r\n\r\n").nth(1).unwrap_or_default();
        serde_json::from_str(json).unwrap_or_default()
    }

    #[test]
    fn capabilities_gate_on_true_or_options_only() {
        let caps = serde_json::json!({
            "implementationProvider": true,
            "typeDefinitionProvider": {},
            "documentHighlightProvider": false,
        });
        assert!(supports(&caps, Locate::Implementation.provider()));
        assert!(supports(&caps, Locate::TypeDefinition.provider()));
        assert!(!supports(&caps, "documentHighlightProvider"));
        assert!(!supports(&caps, "documentRangeFormattingProvider"));
    }

    #[test]
    fn on_type_triggers_read_first_and_more_characters() {
        let caps = serde_json::json!({
            "documentOnTypeFormattingProvider": {
                "firstTriggerCharacter": "}",
                "moreTriggerCharacter": [";", "\n", "ab"],
            },
        });
        assert_eq!(on_type_triggers(&caps), ['}', ';', '\n']);
        assert!(on_type_triggers(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn locate_sends_the_kind_method() {
        let uri = Url::parse("file:///a.rs").unwrap();
        let at = Position {
            line: 1,
            character: 2,
        };
        let sent = body(&locate(
            Locate::TypeDefinition,
            &RequestId::Number(7),
            &uri,
            at,
        ));
        assert_eq!(sent["method"], "textDocument/typeDefinition");
        assert_eq!(sent["params"]["position"]["character"], 2);
    }

    #[test]
    fn document_highlights_parse_sorted_ranges() {
        let answer = serde_json::json!([
            { "range": { "start": { "line": 3, "character": 1 }, "end": { "line": 3, "character": 4 } }, "kind": 2 },
            { "range": { "start": { "line": 0, "character": 5 }, "end": { "line": 0, "character": 8 } } },
        ]);
        let ranges = parse_document_highlights(&answer);
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges.first().map(|r| r.start.line), Some(0));
        assert!(parse_document_highlights(&Value::Null).is_empty());
    }
}
