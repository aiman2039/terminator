//! Language-server table: which languages have a server, its binary,
//! stdio arguments, and per-file protocol ids.
use super::parsing::{extension, javascript_id};
use crate::highlight::Language;
use std::path::Path;

/// One language-server family: its id (Settings override key), Settings
/// label, binary candidates in preference order, and stdio arguments.
#[derive(Clone, Copy, Debug)]
pub struct ServerFamily {
    pub id: &'static str,
    pub label: &'static str,
    pub candidates: &'static [&'static str],
    pub args: &'static [&'static str],
}

/// Every supported server family. The CSS, HTML, and JSON servers come
/// from `vscode-langservers-extracted`.
pub const SERVER_FAMILIES: &[ServerFamily] = &[
    ServerFamily {
        id: "rust",
        label: "Rust language server",
        candidates: &["rust-analyzer"],
        args: &[],
    },
    ServerFamily {
        id: "python",
        label: "Python language server",
        candidates: &["pylsp"],
        args: &[],
    },
    ServerFamily {
        id: "typescript",
        label: "TypeScript language server",
        candidates: &["typescript-language-server"],
        args: &["--stdio"],
    },
    ServerFamily {
        id: "markdown",
        label: "Markdown language server",
        candidates: &["marksman"],
        args: &["server"],
    },
    ServerFamily {
        id: "html",
        label: "HTML language server",
        candidates: &["vscode-html-language-server"],
        args: &["--stdio"],
    },
    ServerFamily {
        id: "css",
        label: "CSS / SCSS / LESS language server",
        candidates: &["vscode-css-language-server"],
        args: &["--stdio"],
    },
    ServerFamily {
        id: "json",
        label: "JSON language server",
        candidates: &["vscode-json-language-server"],
        args: &["--stdio"],
    },
    ServerFamily {
        id: "toml",
        label: "TOML language server",
        candidates: &["taplo"],
        args: &["lsp", "stdio"],
    },
    ServerFamily {
        id: "yaml",
        label: "YAML language server",
        candidates: &["yaml-language-server"],
        args: &["--stdio"],
    },
];

/// Server family id for a language, if it has a server. TypeScript and
/// JavaScript share one family, because one `typescript-language-server`
/// serves both; CSS, SCSS, and LESS share the CSS server. The per-file
/// `didOpen` id comes from [`language_id_for_path`].
#[must_use]
pub fn server_language(language: Language) -> Option<&'static str> {
    match language {
        Language::Rust => Some("rust"),
        Language::Python => Some("python"),
        Language::TypeScript | Language::JavaScript => Some("typescript"),
        Language::Markdown => Some("markdown"),
        Language::Html => Some("html"),
        Language::Css | Language::Scss | Language::Less => Some("css"),
        Language::Json => Some("json"),
        Language::Toml => Some("toml"),
        Language::Yaml => Some("yaml"),
        _ => None,
    }
}

/// Server family for a language, if it has a server.
#[must_use]
pub fn server_family(language: Language) -> Option<&'static ServerFamily> {
    let id = server_language(language)?;
    SERVER_FAMILIES.iter().find(|family| family.id == id)
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
        Language::Markdown => Some("markdown"),
        Language::Html => Some("html"),
        Language::Css => Some("css"),
        Language::Scss => Some("scss"),
        Language::Less => Some("less"),
        Language::Json => Some(if extension(path) == "jsonc" {
            "jsonc"
        } else {
            "json"
        }),
        Language::Toml => Some("toml"),
        Language::Yaml => Some("yaml"),
        _ => None,
    }
}

/// Server binary candidates for a language, in preference order. Only
/// languages here get a server; everything else edits without LSP.
#[must_use]
pub fn server_candidates(language: Language) -> &'static [&'static str] {
    server_family(language).map_or(&[], |family| family.candidates)
}

/// Extra command-line arguments the server needs to speak LSP on stdio.
#[must_use]
pub fn server_args(language: Language) -> &'static [&'static str] {
    server_family(language).map_or(&[], |family| family.args)
}
