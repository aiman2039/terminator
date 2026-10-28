//! Why a hook installer refused or failed to rewrite an agent configuration.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Unknown built-in integration")]
    UnknownIntegration { kind: String },
    #[error("Refusing to rewrite a symlinked agent configuration")]
    SymlinkedConfig { path: std::path::PathBuf },
    #[error("Existing plugin is not managed by Terminator")]
    UnmanagedPlugin { path: std::path::PathBuf },
    #[error("Expected config object")]
    ConfigNotObject,
    #[error("hooks is not an object")]
    HooksNotObject,
    #[error("Hook groups must be arrays")]
    GroupsNotArrays,
    #[error("Refusing to rewrite a symlinked Muse settings file")]
    SymlinkedMuseSettings { path: std::path::PathBuf },
    #[error("Muse settings must be an object")]
    MuseNotObject,
    #[error("Muse settings schema is not version 1; left untouched")]
    MuseSchema { version: serde_json::Value },
    #[error("Muse hooks must be an object")]
    MuseHooksNotObject,
    #[error("Cursor hooks.json must be an object")]
    CursorNotObject,
    #[error("Cursor hooks.json version is not 1; left untouched")]
    CursorVersion { version: serde_json::Value },
    #[error("Cursor hooks must be an object")]
    CursorHooksNotObject,
    #[error("Cursor {event} hooks must be an array")]
    CursorEventNotArray { event: &'static str },
    /// The text keeps its literal `{event}`, as users have always seen it.
    #[error("Unsupported Cursor hook format for {{event}}; left untouched")]
    UnsupportedCursorFormat { event: &'static str },
    #[error("Codex hooks must be a table")]
    CodexHooksNotTable,
    /// The text keeps its literal `{event}`, as users have always seen it.
    #[error("Unsupported Codex hook format for {{event}}; left untouched")]
    UnsupportedCodexFormat { event: &'static str },
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Paths(#[from] terminator_core::PathsError),
    #[error(transparent)]
    Write(#[from] terminator_core::AtomicWriteError),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    #[error("{context}")]
    Json {
        context: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("{context}")]
    Toml {
        context: &'static str,
        #[source]
        source: toml_edit::TomlError,
    },
}
