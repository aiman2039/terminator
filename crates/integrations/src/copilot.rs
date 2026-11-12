//! GitHub Copilot CLI user hooks: `~/.copilot/hooks/terminator.json`,
//! `{"version": 1, "hooks": {event: [{"type": "command", "bash": ...}]}}`.
//! PascalCase event names select the VS Code-compatible payload, which
//! carries `hook_event_name` and `session_id` like Claude's.
use super::*;

/// Lifecycle events with a VS Code-compatible name. `permissionRequest`
/// and `notification` exist only in camelCase, whose payload has no event
/// name, so waiting states are not reported.
const EVENTS: [&str; 4] = ["SessionStart", "UserPromptSubmit", "PostToolUse", "Stop"];

pub(crate) fn copilot_config(before: &str, command: &str, remove: bool) -> Result<String> {
    let mut doc: Value = if before.trim().is_empty() {
        json!({"version": 1})
    } else {
        serde_json::from_str(before).map_err(|source| Error::Json {
            context: "Copilot hook file is invalid JSON; left untouched",
            source,
        })?
    };
    let object = doc.as_object_mut().ok_or(Error::ConfigNotObject)?;
    match object.get("version") {
        None => {
            object.insert("version".to_owned(), json!(1));
        }
        Some(version) if version.as_u64() == Some(1) => {}
        Some(version) => {
            return Err(Error::CopilotVersion {
                version: version.clone(),
            });
        }
    }
    let hooks = object
        .entry("hooks")
        .or_insert(json!({}))
        .as_object_mut()
        .ok_or(Error::HooksNotObject)?;
    for list in hooks.values_mut() {
        list.as_array_mut()
            .ok_or(Error::GroupsNotArrays)?
            .retain(|entry| !managed(entry));
    }
    if !remove {
        for event in EVENTS {
            hooks
                .entry(event)
                .or_insert(json!([]))
                .as_array_mut()
                .ok_or(Error::GroupsNotArrays)?
                .push(json!({"type": "command", "bash": command, "timeoutSec": 5}));
        }
    }
    if remove {
        hooks.retain(|_, list| list.as_array().is_none_or(|list| !list.is_empty()));
    }
    Ok(serde_json::to_string_pretty(&doc)? + "\n")
}

/// An entry this installer wrote: its command carries the marker.
fn managed(entry: &Value) -> bool {
    ["bash", "command", "powershell"].iter().any(|key| {
        entry
            .get(*key)
            .and_then(Value::as_str)
            .is_some_and(|command| command.contains(MARKER))
    })
}
