//! Opt-in Claude Code status-line source for the Usage tool. Install keeps
//! the user's own `statusLine` (saved next to Terminator's data) and points
//! Claude Code at `terminator-hook statusline`, which records only the
//! `rate_limits` object and then runs the user's command unchanged. Remove
//! restores the user's `statusLine`.
use crate::{
    Error, Result, backup_path, config_path, json_hooks::json_doc, read_existing, write_config,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use terminator_core::{atomic_write, quote};

/// Marks Terminator's `statusLine` command. Not the hooks marker, so the
/// status line alone never makes the Claude Code hooks look installed.
const STATUSLINE_MARKER: &str = "terminator-statusline:v1";

/// The user's original `statusLine` object (`null` when there was none).
#[must_use]
pub fn original_path(data: &Path) -> PathBuf {
    data.join("claude-statusline.json")
}

fn managed(doc: &Value) -> bool {
    doc.pointer("/statusLine/command")
        .and_then(Value::as_str)
        .is_some_and(|command| command.contains(STATUSLINE_MARKER))
}

fn settings_path(home: &Path) -> Result<PathBuf> {
    config_path(home, "claude")
}

#[must_use]
pub fn installed(home: &Path) -> bool {
    settings_path(home)
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .is_some_and(|doc| managed(&doc))
}

/// Installs (or with `remove`, removes) the status-line source. `data` is
/// the shared data folder where the hook writes and the GUI reads.
pub fn install(home: &Path, helper: &Path, data: &Path, remove: bool) -> Result<PathBuf> {
    let path = settings_path(home)?;
    if path.is_symlink() {
        return Err(Error::SymlinkedConfig { path });
    }
    let before = read_existing(&path)?;
    let mut doc = json_doc(
        &before,
        json!({}),
        "Agent configuration is invalid JSON; left untouched",
    )?;
    let was_managed = managed(&doc);
    let object = doc.as_object_mut().ok_or(Error::ConfigNotObject)?;
    if remove {
        if !was_managed {
            return Ok(path);
        }
        let original = std::fs::read(original_path(data))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .filter(|value| !value.is_null());
        match original {
            Some(original) => object.insert("statusLine".into(), original),
            None => object.remove("statusLine"),
        };
        let _ = std::fs::remove_file(original_path(data));
    } else {
        let current = object.get("statusLine").cloned().unwrap_or(Value::Null);
        if !was_managed {
            atomic_write(
                &original_path(data),
                serde_json::to_string(&current)?.as_bytes(),
            )?;
        }
        let command = format!(
            "{} statusline --data-dir {} # {STATUSLINE_MARKER}",
            quote(&helper.to_string_lossy()),
            quote(&data.to_string_lossy())
        );
        let mut line = serde_json::Map::new();
        line.insert("type".into(), "command".into());
        line.insert("command".into(), command.into());
        if let Some(padding) = current.get("padding") {
            line.insert("padding".into(), padding.clone());
        }
        let line = Value::Object(line);
        object.insert("statusLine".into(), line);
    }
    let after = serde_json::to_string_pretty(&doc)? + "\n";
    if !before.is_empty() && before != after {
        atomic_write(&backup_path(&path), before.as_bytes())?;
    }
    if path.exists() || !remove {
        write_config(&path, after.as_bytes())?;
    }
    Ok(path)
}

/// The user's own status-line command, run after the record is written.
#[must_use]
pub fn original_command(data: &Path) -> Option<String> {
    let bytes = std::fs::read(original_path(data)).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    value
        .get("command")
        .and_then(Value::as_str)
        .filter(|command| !command.contains(STATUSLINE_MARKER))
        .map(str::to_owned)
}

/// Keeps only `rate_limits` from Claude Code's status-line input, with the
/// time it was seen. `None` when the input has none (API-key accounts).
#[must_use]
pub fn quota_record(input: &[u8], now: u64) -> Option<Vec<u8>> {
    let value: Value = serde_json::from_slice(input).ok()?;
    let limits = value.get("rate_limits")?.as_object()?;
    let mut record = serde_json::Map::new();
    for window in ["five_hour", "seven_day"] {
        if let Some(window_value) = limits.get(window) {
            record.insert(window.into(), window_value.clone());
        }
    }
    record.insert("observed_at".into(), now.into());
    serde_json::to_vec(&record).ok()
}

#[cfg(test)]
#[path = "statusline_tests.rs"]
mod tests;
