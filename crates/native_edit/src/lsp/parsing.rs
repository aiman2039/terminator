use super::*;

pub(super) fn native_position(value: lsp_types::Position) -> Position {
    Position {
        line: value.line,
        character: value.character,
    }
}

pub(super) fn native_range(value: &lsp_types::Range) -> Range {
    Range {
        start: native_position(value.start),
        end: native_position(value.end),
    }
}

pub(super) fn native_severity(value: lsp_types::DiagnosticSeverity) -> Severity {
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

pub(super) fn marked_text(value: &lsp_types::MarkedString) -> String {
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
                        #[expect(deprecated, reason = "Older servers only set the flag")]
                        deprecated: item.deprecated == Some(true)
                            || item.tags.as_ref().is_some_and(|tags| {
                                tags.contains(&lsp_types::SymbolTag::DEPRECATED)
                            }),
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
                    #[expect(deprecated, reason = "Older servers only set the flag")]
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

pub(super) fn symbol_kind_label(kind: lsp_types::SymbolKind) -> String {
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

pub(super) fn native_location(uri: &lsp_types::Uri, range: &lsp_types::Range) -> GotoTarget {
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

pub(super) fn javascript_id(path: &Path) -> &'static str {
    if extension(path) == "jsx" {
        "javascriptreact"
    } else {
        "javascript"
    }
}

pub(super) fn extension(path: &Path) -> String {
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
pub(super) fn find_server_with(
    candidates: &[&str],
    path_env: &str,
    extensions: &[String],
) -> Option<PathBuf> {
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
pub(super) fn candidate_file_names(candidate: &str, extensions: &[String]) -> Vec<String> {
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
pub(super) fn platform_extensions() -> Vec<String> {
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
