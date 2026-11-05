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
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    /// `labelDetails.detail`: drawn dimmed right after the label (for
    /// example the parameters).
    pub label_detail: Option<String>,
    /// `labelDetails.description`: drawn dimmed on the right (for example
    /// the type, or the path an import comes from).
    pub label_description: Option<String>,
    pub kind: Option<String>,
    pub detail: Option<String>,
    /// The doc comment, as Markdown or plain text.
    pub documentation: Option<Hover>,
    /// Text to insert (snippet syntax when `snippet`).
    pub insert: String,
    pub snippet: bool,
    /// Text the typed prefix is matched against.
    pub filter: String,
    /// Server ordering key.
    pub sort: String,
    /// Range the insert replaces, when the server names one (the
    /// insert range of an insert-and-replace edit).
    pub edit: Option<Range>,
    /// The replace range of an insert-and-replace edit: it also covers
    /// the rest of the word after the caret.
    pub replace: Option<Range>,
    /// Edits elsewhere in the file applied with the item (auto-imports).
    pub additional: Vec<RenameTextEdit>,
    /// The server's own item, sent back by `completionItem/resolve`.
    pub raw: Value,
    /// A resolve answer was merged in ([`CompletionItem::merge_resolved`]).
    pub resolved: bool,
}

impl CompletionItem {
    /// Take what a `completionItem/resolve` answer filled in: the docs,
    /// the detail, and the auto-import edits. The listed label, insert
    /// text, and ranges stay (the protocol does not let resolve change
    /// them), so accepting inserts what the list showed.
    pub fn merge_resolved(&mut self, resolved: Self) {
        let Self {
            documentation,
            detail,
            additional,
            ..
        } = resolved;
        if documentation.is_some() {
            self.documentation = documentation;
        }
        if detail.is_some() {
            self.detail = detail;
        }
        if !additional.is_empty() {
            self.additional = additional;
        }
        self.resolved = true;
    }
}

/// A `textDocument/completion` answer. `incomplete` (`isIncomplete`) means
/// the server wants to be asked again as the user types.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompletionList {
    pub items: Vec<CompletionItem>,
    pub incomplete: bool,
}

/// Parse a `textDocument/completion` result (item array or list).
#[must_use]
pub fn parse_completion(result: &Value) -> Vec<CompletionItem> {
    parse_completion_list(result).items
}

/// Parse a `textDocument/completion` result with its `isIncomplete` flag.
/// An item that does not parse is skipped; the rest keep server order.
#[must_use]
pub fn parse_completion_list(result: &Value) -> CompletionList {
    let (items, incomplete) = match result {
        Value::Array(items) => (items.as_slice(), false),
        Value::Object(list) => (
            list.get("items")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default(),
            list.get("isIncomplete").and_then(Value::as_bool) == Some(true),
        ),
        _ => (<&[Value]>::default(), false),
    };
    CompletionList {
        items: items.iter().filter_map(parse_completion_item).collect(),
        incomplete,
    }
}

/// Parse one completion item: a list entry, or a `completionItem/resolve`
/// answer. `None` for anything that is not an item.
#[must_use]
pub fn parse_completion_item(raw: &Value) -> Option<CompletionItem> {
    let item: lsp_types::CompletionItem = serde_json::from_value(raw.clone()).ok()?;
    let (insert, edit, replace) = match &item.text_edit {
        Some(lsp_types::CompletionTextEdit::Edit(edit)) => {
            (edit.new_text.clone(), Some(native_range(&edit.range)), None)
        }
        Some(lsp_types::CompletionTextEdit::InsertAndReplace(edit)) => (
            edit.new_text.clone(),
            Some(native_range(&edit.insert)),
            Some(native_range(&edit.replace)),
        ),
        None => (
            item.insert_text
                .clone()
                .unwrap_or_else(|| item.label.clone()),
            None,
            None,
        ),
    };
    let additional = item
        .additional_text_edits
        .iter()
        .flatten()
        .map(|edit| RenameTextEdit {
            range: native_range(&edit.range),
            new_text: edit.new_text.clone(),
        })
        .collect();
    let details = item.label_details.as_ref();
    Some(CompletionItem {
        label_detail: shown(details.and_then(|details| details.detail.as_ref())),
        label_description: shown(details.and_then(|details| details.description.as_ref())),
        kind: item.kind.map(completion_kind_label),
        detail: item.detail.clone().filter(|detail| !detail.is_empty()),
        documentation: item.documentation.as_ref().and_then(documentation),
        snippet: item.insert_text_format == Some(lsp_types::InsertTextFormat::SNIPPET),
        filter: item
            .filter_text
            .clone()
            .unwrap_or_else(|| item.label.clone()),
        sort: item.sort_text.clone().unwrap_or_else(|| item.label.clone()),
        label: item.label,
        insert,
        edit,
        replace,
        additional,
        raw: raw.clone(),
        resolved: false,
    })
}

/// Text worth drawing: `None` when missing or blank.
fn shown(text: Option<&String>) -> Option<String> {
    text.filter(|text| !text.trim().is_empty()).cloned()
}

/// A completion item's doc comment. A plain string stays plain text, as
/// the protocol says; blank docs read as none.
fn documentation(doc: &lsp_types::Documentation) -> Option<Hover> {
    let (text, markdown) = match doc {
        lsp_types::Documentation::String(text) => (text.clone(), false),
        lsp_types::Documentation::MarkupContent(markup) => (
            markup.value.clone(),
            markup.kind == lsp_types::MarkupKind::Markdown,
        ),
    };
    (!text.trim().is_empty()).then_some(Hover { text, markdown })
}

/// `completionItem/resolve`: the server fills in the lazy parts of one
/// item (docs, detail, auto-import edits). `item` is the item as listed.
#[must_use]
pub fn completion_resolve(id: &RequestId, item: &Value) -> Vec<u8> {
    request(id, "completionItem/resolve", item.clone())
}

/// Why a completion request is sent (`CompletionContext.triggerKind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionTrigger {
    /// Typing an identifier, or Ctrl+Space.
    Invoked,
    /// One of the server's trigger characters was typed.
    Character(char),
    /// Typing went on while the last answer was incomplete.
    Incomplete,
}

impl CompletionTrigger {
    /// The protocol `CompletionContext` for this trigger.
    #[must_use]
    pub fn context(self) -> Value {
        match self {
            Self::Invoked => serde_json::json!({ "triggerKind": 1 }),
            Self::Character(typed) => {
                serde_json::json!({ "triggerKind": 2, "triggerCharacter": typed.to_string() })
            }
            Self::Incomplete => serde_json::json!({ "triggerKind": 3 }),
        }
    }
}

/// `completionProvider.triggerCharacters` from `initialize` capabilities:
/// the single characters that open completion as they are typed. Empty
/// when the server lists none.
#[must_use]
pub fn completion_triggers(capabilities: &Value) -> Vec<char> {
    capabilities
        .get("completionProvider")
        .and_then(|provider| provider.get("triggerCharacters"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|text| {
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some(typed), None) => Some(typed),
                _ => None,
            }
        })
        .collect()
}

/// Whether the server fills in items on `completionItem/resolve`
/// (`completionProvider.resolveProvider`).
#[must_use]
pub fn completion_resolves(capabilities: &Value) -> bool {
    capabilities
        .get("completionProvider")
        .and_then(|provider| provider.get("resolveProvider"))
        .and_then(Value::as_bool)
        == Some(true)
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

/// Expand LSP snippet syntax to plain text ([`parse_snippet`], without
/// variable values). Returns the text and the char offset where the caret
/// goes: the first tab stop, else `$0`, else the end.
#[must_use]
pub fn expand_snippet(snippet: &str) -> (String, usize) {
    let snippet = parse_snippet(snippet, &SnippetVars::default());
    let caret = snippet.caret();
    (snippet.text, caret)
}
