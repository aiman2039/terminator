//! JSON hook files: Claude, Grok, Muse (`hooks.json` plus Muse settings), and Cursor.
use super::*;

const DEFAULT_EVENTS: [&str; 9] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PermissionRequest",
    "Notification",
    "Stop",
    "StopFailure",
    "SessionEnd",
];

/// Claude-style `{"hooks": {event: [group]}}`: managed groups are removed,
/// then one group per event is added unless removing.
pub(crate) fn json_config(rewrite: &Rewrite<'_>) -> Result<String> {
    let Rewrite {
        before,
        kind,
        command,
        remove,
        ..
    } = *rewrite;
    let mut doc: Value = if before.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(before).map_err(|source| Error::Json {
            context: "Agent configuration is invalid JSON; left untouched",
            source,
        })?
    };
    if !doc.is_object() {
        return Err(Error::ConfigNotObject);
    }
    if doc.get("hooks").is_none() {
        doc.as_object_mut()
            .ok_or(Error::ConfigNotObject)?
            .insert("hooks".to_owned(), json!({}));
    }
    let hooks = doc
        .get_mut("hooks")
        .and_then(Value::as_object_mut)
        .ok_or(Error::HooksNotObject)?;
    for groups in hooks.values_mut() {
        clean_json_groups(groups)?;
    }
    if !remove {
        let events = if kind == "muse" {
            muse_hook_events()
        } else {
            &DEFAULT_EVENTS
        };
        add_groups(hooks, events, command)?;
    }
    Ok(serde_json::to_string_pretty(&doc)? + "\n")
}

fn add_groups(
    hooks: &mut serde_json::Map<String, Value>,
    events: &[&str],
    command: &str,
) -> Result<()> {
    for event in events {
        let list = hooks
            .entry(*event)
            .or_insert(json!([]))
            .as_array_mut()
            .ok_or(Error::GroupsNotArrays)?;
        list.push(json!({"hooks":[{"type":"command","command":command,"timeout":2}]}));
    }
    Ok(())
}

pub(crate) fn sync_muse_settings(home: &Path, command: &str, remove: bool) -> Result<()> {
    let path = muse_settings_path(home);
    if path.is_symlink() {
        return Err(Error::SymlinkedMuseSettings { path });
    }
    let before = read_existing(&path)?;
    if before.is_empty() && remove {
        return Ok(());
    }
    let mut doc: Value = if before.trim().is_empty() {
        json!({"schema_version": 1})
    } else {
        serde_json::from_str(&before).map_err(|source| Error::Json {
            context: "Muse settings are invalid JSON; left untouched",
            source,
        })?
    };
    let object = doc.as_object_mut().ok_or(Error::MuseNotObject)?;
    match object.get("schema_version") {
        None => {
            object.insert("schema_version".to_owned(), json!(1));
        }
        Some(version) if version.as_u64() == Some(1) => {}
        Some(version) => {
            return Err(Error::MuseSchema {
                version: version.clone(),
            });
        }
    }
    let hooks = object
        .entry("hooks")
        .or_insert(json!({}))
        .as_object_mut()
        .ok_or(Error::MuseHooksNotObject)?;
    for groups in hooks.values_mut() {
        clean_json_groups(groups)?;
    }
    if !remove {
        add_groups(hooks, muse_hook_events(), command)?;
    }
    let after = serde_json::to_string_pretty(&doc)? + "\n";
    if !before.is_empty() && before != after {
        atomic_write(&backup_path(&path), before.as_bytes())?;
    }
    if before != after {
        write_config(&path, after.as_bytes())?;
    }
    Ok(())
}

fn clean_json_groups(groups: &mut Value) -> Result<()> {
    let groups = groups.as_array_mut().ok_or(Error::GroupsNotArrays)?;
    for group in groups.iter_mut() {
        if let Some(hooks) = group.get_mut("hooks").and_then(Value::as_array_mut) {
            hooks.retain(|h| {
                !h.get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|c| c.contains(MARKER))
            });
        }
    }
    groups.retain(|g| {
        !g.get("hooks")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
    });
    Ok(())
}

pub(crate) fn cursor_config(before: &str, invocation: &str, remove: bool) -> Result<String> {
    let mut doc: Value = if before.trim().is_empty() {
        json!({"version": 1})
    } else {
        serde_json::from_str(before).map_err(|source| Error::Json {
            context: "Cursor hooks.json is invalid JSON; left untouched",
            source,
        })?
    };
    let object = doc.as_object_mut().ok_or(Error::CursorNotObject)?;
    match object.get("version") {
        None => {
            object.insert("version".to_owned(), json!(1));
        }
        Some(version) if version.as_u64() == Some(1) => {}
        Some(version) => {
            return Err(Error::CursorVersion {
                version: version.clone(),
            });
        }
    }
    let hooks = object
        .entry("hooks")
        .or_insert(json!({}))
        .as_object_mut()
        .ok_or(Error::CursorHooksNotObject)?;
    for &(event, response) in cursor_hook_events() {
        let list = hooks
            .entry(event)
            .or_insert(json!([]))
            .as_array_mut()
            .ok_or(Error::CursorEventNotArray { event })?;
        let command =
            (!remove).then(|| format!("{invocation} ; printf '%s\\n' '{response}' # {MARKER}"));
        replace_cursor_hook(list, event, command)?;
    }
    if remove {
        hooks.retain(|_, value| value.as_array().is_none_or(|list| !list.is_empty()));
    }
    Ok(serde_json::to_string_pretty(&doc)? + "\n")
}

/// Drops managed entries from one Cursor event and appends `command` if given.
fn replace_cursor_hook(
    list: &mut Vec<Value>,
    event: &'static str,
    command: Option<String>,
) -> Result<()> {
    if list.iter().any(|entry| {
        entry
            .as_object()
            .is_some_and(|object| object.contains_key("hooks") && !object.contains_key("command"))
    }) {
        return Err(Error::UnsupportedCursorFormat { event });
    }
    list.retain(|entry| {
        !entry
            .get("command")
            .and_then(Value::as_str)
            .is_some_and(|command| command.contains(MARKER))
    });
    if let Some(command) = command {
        list.push(json!({
            "command": command,
            "timeout": 2
        }));
    }
    Ok(())
}
