//! Completion and signature help: requests, parsers and snippet expansion.
use super::*;

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

fn push_first_choice(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, out: &mut String) {
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
