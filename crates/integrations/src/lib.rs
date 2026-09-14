//! Managed hook configuration. Installers only run on explicit user action.
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};
use terminator_core::*;
pub const AGENTS: [&str; 5] = ["claude", "codex", "opencode", "muse", "grok"];
const MARKER: &str = "terminator-managed:v1";
pub fn config_path(home: &Path, kind: &str) -> Result<PathBuf> {
    Ok(home.join(match kind {
        "claude" => ".claude/settings.json",
        "codex" => ".codex/config.toml",
        "opencode" => ".config/opencode/plugins/terminator.js",
        "muse" => ".muse/hooks.json",
        "grok" => ".grok/hooks/terminator.json",
        _ => bail!("Unknown built-in integration"),
    }))
}
pub fn installed(home: &Path, kind: &str) -> bool {
    config_path(home, kind)
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .is_some_and(|s| s.contains(MARKER))
}
pub fn install(home: &Path, kind: &str, helper: &Path, remove: bool) -> Result<PathBuf> {
    install_at(home, kind, helper, remove, &Paths::discover()?)
}
pub fn install_at(
    home: &Path,
    kind: &str,
    helper: &Path,
    remove: bool,
    paths: &Paths,
) -> Result<PathBuf> {
    let path = config_path(home, kind)?;
    let before = fs::read_to_string(&path).or_else(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Ok(String::new())
        } else {
            Err(e)
        }
    })?;
    ensure!(
        !path.is_symlink(),
        "Refusing to rewrite a symlinked agent configuration"
    );
    let command = format!(
        "{} event {} --data-dir {} --runtime-dir {} # {MARKER}",
        quote(&helper.to_string_lossy()),
        kind,
        quote(&paths.data.to_string_lossy()),
        quote(&paths.runtime.to_string_lossy())
    );
    let after = if kind == "opencode" {
        ensure!(
            before.is_empty() || before.contains(MARKER),
            "Existing plugin is not managed by Terminator"
        );
        if remove {
            String::new()
        } else {
            opencode_plugin(helper)
        }
    } else if kind == "codex" {
        codex_config(&before, &command, remove)?
    } else {
        let mut doc: Value = if before.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&before)
                .context("Agent configuration is invalid JSON; left untouched")?
        };
        ensure!(doc.is_object(), "Expected config object");
        if doc.get("hooks").is_none() {
            doc["hooks"] = json!({});
        }
        let hooks = doc["hooks"]
            .as_object_mut()
            .context("hooks is not an object")?;
        for groups in hooks.values_mut() {
            clean_json_groups(groups)?;
        }
        if !remove {
            let events = if kind == "muse" {
                vec![
                    "SessionStart",
                    "UserPromptSubmit",
                    "PreToolUse",
                    "PostToolUse",
                    "PermissionRequest",
                    "Stop",
                    "SessionEnd",
                ]
            } else {
                vec![
                    "SessionStart",
                    "UserPromptSubmit",
                    "PreToolUse",
                    "PostToolUse",
                    "PermissionRequest",
                    "Notification",
                    "Stop",
                    "StopFailure",
                    "SessionEnd",
                ]
            };
            for event in events {
                let list = hooks
                    .entry(event)
                    .or_insert(json!([]))
                    .as_array_mut()
                    .context("Hook groups must be arrays")?;
                list.push(json!({"hooks":[{"type":"command","command":command,"timeout":2}]}));
            }
        }
        serde_json::to_string_pretty(&doc)? + "\n"
    };
    if !before.is_empty() && before != after {
        atomic_write(
            &path.with_extension(format!("terminator-backup-{}", now())),
            before.as_bytes(),
        )?;
    }
    if remove && kind == "opencode" {
        if path.exists() {
            fs::remove_file(&path)?;
        }
    } else {
        atomic_write(&path, after.as_bytes())?;
    }
    Ok(path)
}
fn clean_json_groups(groups: &mut Value) -> Result<()> {
    let groups = groups
        .as_array_mut()
        .context("Hook groups must be arrays")?;
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
fn codex_config(before: &str, command: &str, remove: bool) -> Result<String> {
    use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value as T};
    let mut doc = before
        .parse::<DocumentMut>()
        .context("Invalid Codex TOML; left untouched")?;
    if doc.get("hooks").is_none() {
        doc["hooks"] = Item::Table(Table::new());
    }
    let hooks = doc["hooks"]
        .as_table_mut()
        .context("Codex hooks must be a table")?;
    let events = [
        "SessionStart",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "PermissionRequest",
        "Notification",
        "Stop",
        "SessionEnd",
        "Interrupt",
    ];
    for event in events {
        if let Some(item) = hooks.get_mut(event) {
            if let Some(groups) = item.as_array_of_tables_mut() {
                for group in groups.iter_mut() {
                    if let Some(h) = group.get_mut("hooks").and_then(Item::as_array_mut) {
                        h.retain(|v| {
                            !v.as_inline_table()
                                .and_then(|t| t.get("command"))
                                .and_then(T::as_str)
                                .is_some_and(|c| c.contains(MARKER))
                        });
                    }
                }
                groups.retain(|g| {
                    g.get("hooks")
                        .and_then(Item::as_array)
                        .is_none_or(|a| !a.is_empty())
                });
            } else if let Some(groups) = item.as_array_mut() {
                for g in groups.iter_mut() {
                    if let Some(h) = g
                        .as_inline_table_mut()
                        .and_then(|t| t.get_mut("hooks"))
                        .and_then(T::as_array_mut)
                    {
                        h.retain(|v| {
                            !v.as_inline_table()
                                .and_then(|t| t.get("command"))
                                .and_then(T::as_str)
                                .is_some_and(|c| c.contains(MARKER))
                        });
                    }
                }
                groups.retain(|g| {
                    g.as_inline_table()
                        .and_then(|t| t.get("hooks"))
                        .and_then(T::as_array)
                        .is_none_or(|h| !h.is_empty())
                });
            } else {
                bail!("Unsupported Codex hook format for {event}; left untouched")
            }
        }
        if !remove {
            let mut handler = InlineTable::new();
            handler.insert("type", T::from("command"));
            handler.insert("command", T::from(command));
            handler.insert("timeout", T::from(2));
            let mut hs = Array::new();
            hs.push(handler);
            if let Some(arr) = hooks.get_mut(event).and_then(Item::as_array_mut) {
                let mut group = InlineTable::new();
                group.insert("hooks", T::Array(hs));
                arr.push(group);
            } else {
                if hooks.get(event).is_none() {
                    hooks[event] = Item::ArrayOfTables(ArrayOfTables::new());
                }
                let mut group = Table::new();
                group["hooks"] = Item::Value(T::Array(hs));
                hooks[event].as_array_of_tables_mut().unwrap().push(group);
            }
        }
    }
    Ok(doc.to_string())
}
fn opencode_plugin(helper: &Path) -> String {
    let helper = serde_json::to_string(&helper.to_string_lossy()).unwrap();
    format!(
        r#"// {MARKER}
import {{ spawn }} from 'node:child_process';
import {{ randomUUID }} from 'node:crypto';
export const Terminator = async () => {{
  if (!process.env.TERMINATOR_SESSION_ID) return {{}};
  const invocation = randomUUID();
  let sequence = 0;
  return {{ event: async ({{ event }}) => {{
    if (!['session.created','session.status','session.idle','session.error','session.deleted','permission.asked','permission.replied','question.asked','question.replied'].includes(event.type)) return;
    const payload = {{...event, terminator_invocation: invocation, terminator_sequence: ++sequence, terminator_event: randomUUID()}};
    const child = spawn({helper}, ['event','opencode'], {{stdio:['pipe','ignore','ignore'], env:process.env}});
    child.on('error', () => {{}}); child.stdin.on('error', () => {{}});
    child.stdin.end(JSON.stringify(payload));
    const timer = setTimeout(() => child.kill(), 1800); timer.unref(); child.on('exit', () => clearTimeout(timer));
  }} }};
}};
"#
    )
}
pub fn normalize(
    kind: &str,
    session: &str,
    parent: &str,
    payload: &Value,
) -> Result<Option<HookEvent>> {
    let get = |key: &str| payload.get(key).and_then(Value::as_str);
    let provider = get("session_id")
        .or(get("sessionId"))
        .or(get("thread-id"))
        .or_else(|| {
            payload
                .pointer("/properties/sessionID")
                .and_then(Value::as_str)
        })
        .or_else(|| {
            payload
                .pointer("/properties/info/id")
                .and_then(Value::as_str)
        });
    let name = get("hook_event_name")
        .or(get("type"))
        .or(get("hookEventName"))
        .unwrap_or("");
    let tool = get("tool_name")
        .or(get("toolName"))
        .unwrap_or("")
        .to_lowercase();
    let state = match name {
        "SessionStart" | "session.created" => AgentState::Unknown,
        "UserPromptSubmit" | "PostToolUse" | "permission.replied" | "question.replied" => {
            AgentState::Running
        }
        "PreToolUse" if tool.contains("askuser") || tool.contains("request_user_input") => {
            AgentState::WaitingInput
        }
        "PreToolUse" => AgentState::Running,
        "PermissionRequest" | "permission.asked" | "permission_request" => {
            AgentState::WaitingPermission
        }
        "question.asked" => AgentState::WaitingInput,
        "Stop" | "session.idle" | "agent-turn-complete" => AgentState::Completed,
        "StopFailure" | "session.error" => AgentState::Failed,
        "SessionEnd" | "session.deleted" => AgentState::Stopped,
        "Interrupt" => AgentState::Unknown,
        "Notification" | "notification" => {
            match get("notification_type").or(get("notificationType")) {
                Some("permission_prompt") => AgentState::WaitingPermission,
                Some("idle_prompt" | "elicitation_dialog") => AgentState::WaitingInput,
                _ => return Ok(None),
            }
        }
        "session.status" => match payload
            .pointer("/properties/status/type")
            .and_then(Value::as_str)
        {
            Some("busy" | "retry") => AgentState::Running,
            Some("idle") => AgentState::Completed,
            _ => return Ok(None),
        },
        _ => return Ok(None),
    };
    let Some(provider) = provider else {
        return Ok(None);
    };
    let process = get("terminator_invocation").unwrap_or(parent);
    let invocation = format!("{kind}:{process}:{provider}");
    let summary = get("message")
        .or(get("last_assistant_message"))
        .or(get("last-assistant-message"))
        .or(get("lastAssistantMessage"))
        .unwrap_or(state.label())
        .chars()
        .take(1000)
        .collect();
    // Deliberately omit tool input, prompts, transcripts and credentials from details.
    let details = format!("Agent: {kind}\nEvent: {name}\nSession: {provider}");
    let request = get("tool_use_id")
        .or(get("toolUseId"))
        .or(get("request_id"))
        .or(get("requestId"))
        .or_else(|| payload.pointer("/properties/id").and_then(Value::as_str))
        .map(str::to_owned);
    let resume = match kind {
        "claude" => Some(Resume {
            program: "claude".into(),
            args: vec!["--resume".into(), provider.into()],
        }),
        "codex" => Some(Resume {
            program: "codex".into(),
            args: vec!["resume".into(), provider.into()],
        }),
        "opencode" => Some(Resume {
            program: "opencode".into(),
            args: vec!["--session".into(), provider.into()],
        }),
        "muse" => Some(Resume {
            program: "muse".into(),
            args: vec!["resume".into(), provider.into()],
        }),
        "grok" => Some(Resume {
            program: "grok".into(),
            args: vec!["--resume".into(), provider.into()],
        }),
        _ => None,
    };
    Ok(Some(HookEvent {
        protocol_version: PROTOCOL_VERSION,
        event_id: get("terminator_event")
            .map(str::to_owned)
            .unwrap_or_else(id),
        terminal_session_id: session.into(),
        agent_invocation_id: invocation,
        agent_kind: kind.into(),
        provider_session_id: Some(provider.into()),
        state,
        request_id: request,
        sequence: payload.get("terminator_sequence").and_then(Value::as_u64),
        summary,
        details,
        resume,
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installers_preserve_unrelated_hooks_and_are_idempotent() {
        for kind in ["claude", "grok", "muse"] {
            let home = tempfile::tempdir().unwrap();
            let path = config_path(home.path(), kind).unwrap();
            atomic_write(&path,br#"{"theme":"dark","hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo mine"}]}]}}"#).unwrap();
            let helper = Path::new("/tmp/space dir/hook");
            install(home.path(), kind, helper, false).unwrap();
            let once = fs::read_to_string(&path).unwrap();
            install(home.path(), kind, helper, false).unwrap();
            assert_eq!(once, fs::read_to_string(&path).unwrap());
            install(home.path(), kind, helper, true).unwrap();
            let after = fs::read_to_string(path).unwrap();
            assert!(after.contains("echo mine"));
            assert!(!after.contains(MARKER));
        }
    }
    #[test]
    fn codex_preserves_comments_and_hooks() {
        let source = "# user comment\nmodel = \"example\"\n[[hooks.Stop]]\nhooks = [{type=\"command\", command=\"echo user\"}]\n";
        let once = codex_config(source, &format!("/tmp/hook # {MARKER}"), false).unwrap();
        let twice = codex_config(&once, &format!("/tmp/hook # {MARKER}"), false).unwrap();
        assert_eq!(once, twice);
        let clean = codex_config(&twice, "", true).unwrap();
        assert!(clean.contains("# user comment"));
        assert!(clean.contains("echo user"));
        assert!(!clean.contains(MARKER));
    }
    #[test]
    fn anonymous_and_unrelated_events_not_guessed() {
        assert!(
            normalize("claude", "s", "p", &json!({"hook_event_name":"Stop"}))
                .unwrap()
                .is_none()
        );
        assert!(normalize("claude","s","p",&json!({"hook_event_name":"Notification","notification_type":"auth_success","session_id":"a"})).unwrap().is_none());
    }
    #[test]
    fn permission_and_resume_mapping() {
        let e = normalize(
            "claude",
            "s",
            "p",
            &json!({"hook_event_name":"PermissionRequest","session_id":"a"}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(e.state, AgentState::WaitingPermission);
        assert_eq!(e.resume.unwrap().args, vec!["--resume", "a"]);
    }
    #[test]
    fn grok_camel_case_permission_notification() {
        let e = normalize(
            "grok",
            "s",
            "p",
            &json!({
                "hookEventName": "notification",
                "sessionId": "abc",
                "notificationType": "permission_prompt"
            }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(e.state, AgentState::WaitingPermission);
        assert_eq!(e.provider_session_id.as_deref(), Some("abc"));
    }
    #[test]
    fn codex_installs_notification_hooks() {
        let config = codex_config("", "/tmp/hook # terminator-managed:v1", false).unwrap();
        assert!(config.contains("[[hooks.Notification]]"));
        assert!(config.contains("[[hooks.PermissionRequest]]"));
    }
}
