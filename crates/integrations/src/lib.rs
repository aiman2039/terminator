//! Managed hook configuration. Installers only run on explicit user action.
#![forbid(unsafe_code)]

pub type Result<T> = std::result::Result<T, Error>;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};
use terminator_core::{
    AgentState, HookEvent, PROTOCOL_VERSION, Paths, Resume, atomic_write, id, quote,
};
pub const AGENTS: [&str; 6] = ["claude", "codex", "opencode", "muse", "grok", "cursor"];
const MARKER: &str = "terminator-managed:v1";
pub fn config_path(home: &Path, kind: &str) -> Result<PathBuf> {
    Ok(home.join(match kind {
        "claude" => ".claude/settings.json",
        "codex" => ".codex/config.toml",
        "opencode" => ".config/opencode/plugins/terminator.js",
        "muse" => ".muse/hooks.json",
        "grok" => ".grok/hooks/terminator.json",
        "cursor" => ".cursor/hooks.json",
        _ => {
            return Err(Error::UnknownIntegration {
                kind: kind.to_owned(),
            });
        }
    }))
}
#[must_use]
pub fn installed(home: &Path, kind: &str) -> bool {
    let mut paths = Vec::new();
    if let Ok(path) = config_path(home, kind) {
        paths.push(path);
    }
    if kind == "muse" {
        paths.push(muse_settings_path(home));
    }
    paths
        .iter()
        .any(|path| fs::read_to_string(path).is_ok_and(|text| text.contains(MARKER)))
}

/// Settings hover path. Muse Code 1.4 reads user hooks from this file.
#[must_use]
pub fn settings_path(home: &Path, kind: &str) -> PathBuf {
    if kind == "muse" {
        muse_settings_path(home)
    } else {
        config_path(home, kind).unwrap_or_else(|_| home.to_path_buf())
    }
}

fn muse_settings_path(home: &Path) -> PathBuf {
    home.join(".config/muse/settings.json")
}

/// Cursor lifecycle hooks. Permission hooks (`preToolUse`, `beforeShellExecution`,
/// `beforeMCPExecution`, `beforeReadFile`, `subagentStart`) are omitted: exit 0
/// with a body that fails their schema blocks the action.
fn cursor_hook_events() -> &'static [(&'static str, &'static str)] {
    &[
        ("sessionStart", "{}"),
        ("beforeSubmitPrompt", "{\"continue\":true}"),
        ("postToolUse", "{}"),
        ("postToolUseFailure", "{}"),
        ("afterAgentResponse", "{}"),
        ("stop", "{}"),
        ("sessionEnd", "{}"),
    ]
}

fn muse_hook_events() -> &'static [&'static str] {
    &[
        "SessionStart",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "PermissionRequest",
        "Stop",
        "SubagentStart",
        "SubagentStop",
        "SessionEnd",
    ]
}
pub struct InstallInput<'a> {
    pub home: &'a Path,
    pub kind: &'a str,
    pub helper: &'a Path,
    pub remove: bool,
}

pub fn install(input: InstallInput<'_>) -> Result<PathBuf> {
    let InstallInput {
        home,
        kind,
        helper,
        remove,
    } = input;
    install_at(InstallAtInput {
        home,
        kind,
        helper,
        remove,
        paths: &Paths::discover()?,
    })
}
pub struct InstallAtInput<'a> {
    pub home: &'a Path,
    pub kind: &'a str,
    pub helper: &'a Path,
    pub remove: bool,
    pub paths: &'a Paths,
}

pub fn install_at(input: InstallAtInput<'_>) -> Result<PathBuf> {
    let InstallAtInput {
        home,
        kind,
        helper,
        remove,
        paths,
    } = input;
    let path = config_path(home, kind)?;
    let before = read_existing(&path)?;
    if path.is_symlink() {
        return Err(Error::SymlinkedConfig { path });
    }
    let invocation = format!(
        "{} event {} --data-dir {} --runtime-dir {}",
        quote(&helper.to_string_lossy()),
        kind,
        quote(&paths.data.to_string_lossy()),
        quote(&paths.runtime.to_string_lossy())
    );
    let command = format!("{invocation} # {MARKER}");
    let after = Rewrite {
        path: &path,
        kind,
        before: &before,
        helper,
        invocation: &invocation,
        command: &command,
        remove,
    }
    .after()?;
    if !before.is_empty() && before != after {
        atomic_write(&backup_path(&path), before.as_bytes())?;
    }
    if remove && kind == "opencode" {
        if path.exists() {
            fs::remove_file(&path)?;
        }
    } else if !(after.is_empty() && !path.exists()) {
        write_config(&path, after.as_bytes())?;
    }
    if kind == "muse" {
        sync_muse_settings(home, &command, remove)?;
    }
    Ok(path)
}

/// One agent configuration rewrite: the current text and the managed hook to
/// install, or to strip when `remove` is set.
pub(crate) struct Rewrite<'a> {
    path: &'a Path,
    kind: &'a str,
    before: &'a str,
    helper: &'a Path,
    invocation: &'a str,
    command: &'a str,
    remove: bool,
}

impl Rewrite<'_> {
    /// The configuration text to write.
    fn after(&self) -> Result<String> {
        match self.kind {
            "opencode" => self.opencode(),
            "codex" => codex_config(self.before, self.command, self.remove),
            "cursor" if self.remove && self.before.trim().is_empty() => Ok(String::new()),
            "cursor" => cursor_config(self.before, self.invocation, self.remove),
            _ => json_config(self),
        }
    }

    fn opencode(&self) -> Result<String> {
        if !(self.before.is_empty() || self.before.contains(MARKER)) {
            return Err(Error::UnmanagedPlugin {
                path: self.path.to_path_buf(),
            });
        }
        Ok(if self.remove {
            String::new()
        } else {
            opencode_plugin(self.helper)
        })
    }
}

/// The file's text, or empty when it does not exist.
fn read_existing(path: &Path) -> Result<String> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error.into()),
    }
}

/// A backup name no earlier backup uses: two installs within one second
/// must not overwrite the backup of the user's original configuration.
fn backup_path(path: &Path) -> PathBuf {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis());
    let base = format!("terminator-backup-{millis}");
    let mut candidate = path.with_extension(&base);
    let mut n = 1u32;
    while candidate.exists() {
        candidate = path.with_extension(format!("{base}-{n}"));
        n = n.saturating_add(1);
    }
    candidate
}

/// Replace an agent's configuration file, keeping the permissions the user
/// gave it instead of the private mode used for new files.
fn write_config(path: &Path, bytes: &[u8]) -> Result<()> {
    let permissions = fs::metadata(path).ok().map(|meta| meta.permissions());
    atomic_write(path, bytes)?;
    if let Some(permissions) = permissions {
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

mod codex;
mod error;
mod json_hooks;
mod normalize;
mod opencode;
use codex::codex_config;
pub use error::Error;
use json_hooks::{cursor_config, json_config, sync_muse_settings};
pub use normalize::{NormalizeInput, normalize};
use opencode::opencode_plugin;

#[cfg(test)]
mod tests;
