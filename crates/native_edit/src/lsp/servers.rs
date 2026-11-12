//! Language-server table: which languages have a server, its binary,
//! stdio arguments, and per-file protocol ids. User-defined servers
//! (Settings → Languages) live in [`super::custom`] and win over these.
use super::custom::{custom_family, custom_family_for};
use super::parsing::{extension, javascript_id};
use crate::highlight::Language;
use std::path::Path;

/// One language-server family: its id (Settings override key), Settings
/// label, binary candidates in preference order, and stdio arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServerFamily {
    pub id: &'static str,
    pub label: &'static str,
    pub candidates: &'static [&'static str],
    pub args: &'static [&'static str],
    /// Runs project code (build scripts, plugins): ask before starting.
    pub needs_trust: bool,
    /// How to install the server, shown when it is not found. Empty for
    /// user-defined servers. Nothing is installed or downloaded for you.
    pub install: &'static str,
}

/// Every supported server family. The CSS, HTML, and JSON servers come
/// from `vscode-langservers-extracted`. Nothing is bundled.
pub const SERVER_FAMILIES: &[ServerFamily] = &[
    ServerFamily {
        id: "rust",
        label: "Rust language server",
        candidates: &["rust-analyzer"],
        args: &[],
        needs_trust: true,
        install: "rustup component add rust-analyzer",
    },
    ServerFamily {
        id: "python",
        label: "Python language server",
        candidates: &["pylsp"],
        args: &[],
        needs_trust: true,
        install: "pipx install python-lsp-server",
    },
    ServerFamily {
        id: "typescript",
        label: "TypeScript language server",
        candidates: &["typescript-language-server"],
        args: &["--stdio"],
        needs_trust: true,
        install: "npm install -g typescript typescript-language-server",
    },
    ServerFamily {
        id: "markdown",
        label: "Markdown language server",
        candidates: &["marksman"],
        args: &["server"],
        needs_trust: true,
        install: "brew install marksman",
    },
    ServerFamily {
        id: "html",
        label: "HTML language server",
        candidates: &["vscode-html-language-server"],
        args: &["--stdio"],
        needs_trust: true,
        install: "npm install -g vscode-langservers-extracted",
    },
    ServerFamily {
        id: "css",
        label: "CSS / SCSS / LESS language server",
        candidates: &["vscode-css-language-server"],
        args: &["--stdio"],
        needs_trust: true,
        install: "npm install -g vscode-langservers-extracted",
    },
    ServerFamily {
        id: "json",
        label: "JSON language server",
        candidates: &["vscode-json-language-server"],
        args: &["--stdio"],
        needs_trust: true,
        install: "npm install -g vscode-langservers-extracted",
    },
    ServerFamily {
        id: "toml",
        label: "TOML language server",
        candidates: &["taplo"],
        args: &["lsp", "stdio"],
        needs_trust: true,
        install: "cargo install taplo-cli --locked --features lsp",
    },
    ServerFamily {
        id: "yaml",
        label: "YAML language server",
        candidates: &["yaml-language-server"],
        args: &["--stdio"],
        needs_trust: true,
        install: "npm install -g yaml-language-server",
    },
    ServerFamily {
        id: "go",
        label: "Go language server",
        candidates: &["gopls"],
        args: &[],
        needs_trust: true,
        install: "go install golang.org/x/tools/gopls@latest",
    },
    ServerFamily {
        id: "c",
        label: "C / C++ language server",
        candidates: &["clangd"],
        args: &[],
        needs_trust: true,
        install: "brew install llvm (macOS) or apt install clangd",
    },
    ServerFamily {
        id: "java",
        label: "Java language server",
        candidates: &["jdtls"],
        args: &[],
        needs_trust: true,
        install: "brew install jdtls",
    },
    ServerFamily {
        id: "lua",
        label: "Lua language server",
        candidates: &["lua-language-server"],
        args: &[],
        needs_trust: true,
        install: "brew install lua-language-server",
    },
    ServerFamily {
        id: "shell",
        label: "Bash language server",
        candidates: &["bash-language-server"],
        args: &["start"],
        needs_trust: true,
        install: "npm install -g bash-language-server",
    },
    ServerFamily {
        id: "ruby",
        label: "Ruby language server",
        candidates: &["ruby-lsp"],
        args: &[],
        needs_trust: true,
        install: "gem install ruby-lsp",
    },
    ServerFamily {
        id: "php",
        label: "PHP language server",
        candidates: &["intelephense"],
        args: &["--stdio"],
        needs_trust: true,
        install: "npm install -g intelephense",
    },
    ServerFamily {
        id: SPELL_SERVER,
        label: "Spell checker",
        candidates: &["harper-ls"],
        args: &["--stdio"],
        needs_trust: false,
        install: "brew install harper",
    },
];

/// Family id of the spell checker. It runs beside the file's language
/// server for every file, so no [`Language`] maps to it.
pub const SPELL_SERVER: &str = "spell";

/// Server family by id: a built-in family, else a user-defined one.
#[must_use]
pub fn family(id: &str) -> Option<&'static ServerFamily> {
    SERVER_FAMILIES
        .iter()
        .find(|family| family.id == id)
        .or_else(|| custom_family(id))
}

/// [`server_candidates`] by family id.
#[must_use]
pub fn family_candidates(id: &str) -> &'static [&'static str] {
    family(id).map_or(&[], |family| family.candidates)
}

/// [`server_args`] by family id.
#[must_use]
pub fn family_args(id: &str) -> &'static [&'static str] {
    family(id).map_or(&[], |family| family.args)
}

/// Install hint by family id (empty when none is known).
#[must_use]
pub fn family_install(id: &str) -> &'static str {
    family(id).map_or("", |family| family.install)
}

/// Whether a family may start only in a trusted project root.
#[must_use]
pub fn family_needs_trust(id: &str) -> bool {
    family(id).is_none_or(|family| family.needs_trust)
}

/// `languageId` for the spell checker: the language server's id, else a
/// common id for languages without a server here, else plain text.
/// `harper-ls` checks comments in code and all of plain text and Markdown.
#[must_use]
pub fn spell_language_id(path: &Path, language: Language) -> &'static str {
    language_id_for_path(path, language).unwrap_or_else(|| common_language_id(path))
}

/// Usual LSP `languageId` for languages with no built-in id.
fn common_language_id(path: &Path) -> &'static str {
    match extension(path).as_str() {
        "swift" => "swift",
        "kt" | "kts" => "kotlin",
        "cs" => "csharp",
        "nix" => "nix",
        "dart" => "dart",
        "scala" => "scala",
        "hs" => "haskell",
        "sol" => "solidity",
        "typ" => "typst",
        "tex" => "latex",
        "org" => "org",
        "zig" => "zig",
        "ex" | "exs" => "elixir",
        "erl" => "erlang",
        "ml" | "mli" => "ocaml",
        "clj" | "cljs" => "clojure",
        "vue" => "vue",
        "svelte" => "svelte",
        "tf" => "terraform",
        "lua" => "lua",
        "rb" => "ruby",
        "php" => "php",
        _ => "plaintext",
    }
}

/// Built-in family id for a highlight language. Lua, Ruby and PHP have
/// no dedicated [`Language`]: they come from the bundled grammar names.
fn builtin_language(language: Language) -> Option<&'static str> {
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
        Language::Go => Some("go"),
        Language::C | Language::Cpp => Some("c"),
        Language::Java => Some("java"),
        Language::Shell => Some("shell"),
        Language::Syntax(name) => match name {
            "Lua" => Some("lua"),
            "Ruby" | "Ruby on Rails" => Some("ruby"),
            "PHP" | "PHP Source" => Some("php"),
            _ => None,
        },
        Language::Plain => None,
    }
}

/// Server family id for a language, if it has a server. A user-defined
/// server for the language wins. TypeScript and JavaScript share one
/// family, because one `typescript-language-server` serves both; CSS,
/// SCSS, and LESS share the CSS server; C and C++ share clangd. The
/// per-file `didOpen` id comes from [`language_id_for_path`].
#[must_use]
pub fn server_language(language: Language) -> Option<&'static str> {
    if let Some(custom) = custom_family_for(language) {
        return Some(custom.id);
    }
    builtin_language(language)
}

/// Server family for a language, if it has a server.
#[must_use]
pub fn server_family(language: Language) -> Option<&'static ServerFamily> {
    family(server_language(language)?)
}

/// Protocol `languageId` for `didOpen`: TypeScript and JavaScript have
/// several (`.tsx` is `typescriptreact`, `.jsx` is `javascriptreact`).
#[must_use]
pub fn language_id_for_path(path: &Path, language: Language) -> Option<&'static str> {
    let id = match language {
        Language::Rust => "rust",
        Language::Python => "python",
        Language::TypeScript => {
            if extension(path) == "tsx" {
                "typescriptreact"
            } else {
                "typescript"
            }
        }
        Language::JavaScript => javascript_id(path),
        Language::Markdown => "markdown",
        Language::Html => "html",
        Language::Css => "css",
        Language::Scss => "scss",
        Language::Less => "less",
        Language::Json => {
            if extension(path) == "jsonc" {
                "jsonc"
            } else {
                "json"
            }
        }
        Language::Toml => "toml",
        Language::Yaml => "yaml",
        Language::Go => "go",
        Language::C => "c",
        Language::Cpp => "cpp",
        Language::Java => "java",
        Language::Shell => "shellscript",
        Language::Syntax(_) | Language::Plain => match builtin_language(language) {
            Some(family) => family,
            // A user-defined server still needs an id for its files.
            None if custom_family_for(language).is_some() => common_language_id(path),
            None => return None,
        },
    };
    Some(id)
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
