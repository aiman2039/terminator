//! Managed hook configuration. Installers only run on explicit user action.
#![forbid(unsafe_code)]
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};
use terminator_core::{
    AgentState, HookEvent, PROTOCOL_VERSION, Paths, Resume, atomic_write, id, now, quote,
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
        _ => bail!("Unknown built-in integration"),
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
    let invocation = format!(
        "{} event {} --data-dir {} --runtime-dir {}",
        quote(&helper.to_string_lossy()),
        kind,
        quote(&paths.data.to_string_lossy()),
        quote(&paths.runtime.to_string_lossy())
    );
    let command = format!("{invocation} # {MARKER}");
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
    } else if kind == "cursor" {
        if remove && before.trim().is_empty() {
            String::new()
        } else {
            cursor_config(&before, &invocation, remove)?
        }
    } else {
        let mut doc: Value = if before.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(&before)
                .context("Agent configuration is invalid JSON; left untouched")?
        };
        ensure!(doc.is_object(), "Expected config object");
        if doc.get("hooks").is_none() {
            doc.as_object_mut()
                .context("Expected config object")?
                .insert("hooks".to_owned(), json!({}));
        }
        let hooks = doc
            .get_mut("hooks")
            .and_then(Value::as_object_mut)
            .context("hooks is not an object")?;
        for groups in hooks.values_mut() {
            clean_json_groups(groups)?;
        }
        if !remove {
            let events = if kind == "muse" {
                muse_hook_events().to_vec()
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
    } else if !(after.is_empty() && !path.exists()) {
        atomic_write(&path, after.as_bytes())?;
    }
    if kind == "muse" {
        sync_muse_settings(home, &command, remove)?;
    }
    Ok(path)
}

fn sync_muse_settings(home: &Path, command: &str, remove: bool) -> Result<()> {
    let path = muse_settings_path(home);
    ensure!(
        !path.is_symlink(),
        "Refusing to rewrite a symlinked Muse settings file"
    );
    let before = fs::read_to_string(&path).or_else(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            Ok(String::new())
        } else {
            Err(error)
        }
    })?;
    if before.is_empty() && remove {
        return Ok(());
    }
    let mut doc: Value = if before.trim().is_empty() {
        json!({"schema_version": 1})
    } else {
        serde_json::from_str(&before).context("Muse settings are invalid JSON; left untouched")?
    };
    ensure!(doc.is_object(), "Muse settings must be an object");
    match doc.get("schema_version") {
        None => {
            doc.as_object_mut()
                .context("Muse settings must be an object")?
                .insert("schema_version".to_owned(), json!(1));
        }
        Some(version) if version.as_u64() == Some(1) => {}
        Some(_) => bail!("Muse settings schema is not version 1; left untouched"),
    }
    if doc.get("hooks").is_none() {
        doc.as_object_mut()
            .context("Muse settings must be an object")?
            .insert("hooks".to_owned(), json!({}));
    }
    let hooks = doc
        .get_mut("hooks")
        .and_then(Value::as_object_mut)
        .context("Muse hooks must be an object")?;
    for groups in hooks.values_mut() {
        clean_json_groups(groups)?;
    }
    if !remove {
        for event in muse_hook_events() {
            let list = hooks
                .entry(*event)
                .or_insert(json!([]))
                .as_array_mut()
                .context("Hook groups must be arrays")?;
            list.push(json!({"hooks":[{"type":"command","command":command,"timeout":2}]}));
        }
    }
    let after = serde_json::to_string_pretty(&doc)? + "\n";
    if !before.is_empty() && before != after {
        atomic_write(
            &path.with_extension(format!("terminator-backup-{}", now())),
            before.as_bytes(),
        )?;
    }
    if before != after {
        atomic_write(&path, after.as_bytes())?;
    }
    Ok(())
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
fn cursor_config(before: &str, invocation: &str, remove: bool) -> Result<String> {
    let mut doc: Value = if before.trim().is_empty() {
        json!({"version": 1})
    } else {
        serde_json::from_str(before).context("Cursor hooks.json is invalid JSON; left untouched")?
    };
    ensure!(doc.is_object(), "Cursor hooks.json must be an object");
    match doc.get("version") {
        None => {
            doc.as_object_mut()
                .context("Cursor hooks.json must be an object")?
                .insert("version".to_owned(), json!(1));
        }
        Some(version) if version.as_u64() == Some(1) => {}
        Some(_) => bail!("Cursor hooks.json version is not 1; left untouched"),
    }
    if doc.get("hooks").is_none() {
        doc.as_object_mut()
            .context("Cursor hooks.json must be an object")?
            .insert("hooks".to_owned(), json!({}));
    }
    let hooks = doc
        .get_mut("hooks")
        .and_then(Value::as_object_mut)
        .context("Cursor hooks must be an object")?;
    for &(event, response) in cursor_hook_events() {
        let list = hooks
            .entry(event)
            .or_insert(json!([]))
            .as_array_mut()
            .with_context(|| format!("Cursor {event} hooks must be an array"))?;
        if list.iter().any(|entry| {
            entry.as_object().is_some_and(|object| {
                object.contains_key("hooks") && !object.contains_key("command")
            })
        }) {
            bail!("Unsupported Cursor hook format for {event}; left untouched");
        }
        list.retain(|entry| {
            !entry
                .get("command")
                .and_then(Value::as_str)
                .is_some_and(|command| command.contains(MARKER))
        });
        if !remove {
            list.push(json!({
                "command": format!("{invocation} ; printf '%s\\n' '{response}' # {MARKER}"),
                "timeout": 2
            }));
        }
    }
    if remove {
        hooks.retain(|_, value| value.as_array().is_none_or(|list| !list.is_empty()));
    }
    Ok(serde_json::to_string_pretty(&doc)? + "\n")
}

fn codex_config(before: &str, command: &str, remove: bool) -> Result<String> {
    use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value as T};
    let mut doc = before
        .parse::<DocumentMut>()
        .context("Invalid Codex TOML; left untouched")?;
    if doc.get("hooks").is_none() {
        doc.insert("hooks", Item::Table(Table::new()));
    }
    let hooks = doc
        .get_mut("hooks")
        .and_then(Item::as_table_mut)
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
                    hooks.insert(event, Item::ArrayOfTables(ArrayOfTables::new()));
                }
                let mut group = Table::new();
                group.insert("hooks", Item::Value(T::Array(hs)));
                hooks
                    .get_mut(event)
                    .and_then(Item::as_array_of_tables_mut)
                    .context("Unsupported Codex hook format for {event}; left untouched")?
                    .push(group);
            }
        }
    }
    Ok(doc.to_string())
}
fn opencode_plugin(helper: &Path) -> String {
    let helper = json!(helper.to_string_lossy()).to_string();
    format!(
        r"// {MARKER}
import {{ spawn }} from 'node:child_process';
import {{ randomUUID }} from 'node:crypto';
const TYPES = ['session.created','session.status','session.idle','session.error','session.deleted','permission.asked','permission.replied','permission.v2.asked','permission.v2.replied','question.asked','question.replied','question.rejected','question.v2.asked','question.v2.replied','question.v2.rejected'];
let started = false;
function bind() {{
  if (!process.env.TERMINATOR_SESSION_ID || started) return null;
  started = true;
  return {{ invocation: randomUUID(), seq: {{ n: 0 }} }};
}}
function emit(invocation, seq, event) {{
  const type = event && event.type;
  if (!TYPES.includes(type)) return;
  const payload = {{...event, terminator_invocation: invocation, terminator_sequence: ++seq.n, terminator_event: randomUUID()}};
  const child = spawn({helper}, ['event','opencode'], {{stdio:['pipe','ignore','ignore'], env:process.env}});
  child.on('error', () => {{}}); child.stdin.on('error', () => {{}});
  child.stdin.end(JSON.stringify(payload));
  const timer = setTimeout(() => child.kill(), 1800); timer.unref(); child.on('exit', () => clearTimeout(timer));
}}
async function server() {{
  const s = bind();
  if (!s) return {{}};
  return {{ event: async ({{ event }}) => emit(s.invocation, s.seq, event) }};
}}
export const Terminator = server;
export default {{
  id: 'terminator',
  server,
  async setup(ctx) {{
    const s = bind();
    if (!s) return;
    const c = new AbortController();
    void (async () => {{
      for await (const event of ctx.event.subscribe({{ signal: c.signal }})) emit(s.invocation, s.seq, event);
    }})();
    return () => c.abort();
  }},
}};
"
    )
}

fn first_str<'a>(payload: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| {
        if key.starts_with('/') {
            payload.pointer(key).and_then(Value::as_str)
        } else {
            payload.get(*key).and_then(Value::as_str)
        }
    })
}

fn pre_tool_state(tool: &str) -> AgentState {
    if tool.contains("askuser") || tool.contains("request_user_input") {
        AgentState::WaitingInput
    } else {
        AgentState::Running
    }
}

fn notification_state(payload: &Value) -> Option<AgentState> {
    match first_str(payload, &["notification_type", "notificationType"]) {
        Some("permission_prompt") => Some(AgentState::WaitingPermission),
        Some("idle_prompt" | "elicitation_dialog") => Some(AgentState::WaitingInput),
        _ => None,
    }
}

fn session_status_state(payload: &Value) -> Option<AgentState> {
    match first_str(payload, &["/properties/status/type", "/data/status/type"]) {
        Some("busy" | "retry") => Some(AgentState::Running),
        Some("idle") => Some(AgentState::Completed),
        _ => None,
    }
}

fn cursor_stop_state(payload: &Value) -> AgentState {
    match first_str(payload, &["status"]) {
        Some("error") => AgentState::Failed,
        Some("aborted") => AgentState::Unknown,
        _ => AgentState::Completed,
    }
}

fn cursor_session_end_state(payload: &Value) -> AgentState {
    // Cursor CLI can report only sessionStart/sessionEnd, with no stop event.
    // The reason describes the outcome; closing a window is not completion.
    match first_str(payload, &["reason"]) {
        Some("completed") => AgentState::Completed,
        Some("error") => AgentState::Failed,
        Some("aborted") => AgentState::Unknown,
        _ => AgentState::Stopped,
    }
}

fn hook_state(kind: &str, name: &str, payload: &Value, tool: &str) -> Option<AgentState> {
    match name {
        "SessionStart" | "session.created" | "sessionStart" => Some(AgentState::Unknown),
        "UserPromptSubmit"
        | "PostToolUse"
        | "beforeSubmitPrompt"
        | "postToolUse"
        | "postToolUseFailure"
        | "afterAgentResponse"
        | "permission.replied"
        | "permission.v2.replied"
        | "question.replied"
        | "question.rejected"
        | "question.v2.replied"
        | "question.v2.rejected" => Some(AgentState::Running),
        "PreToolUse" => Some(pre_tool_state(tool)),
        "PermissionRequest" | "permission.asked" | "permission.v2.asked" | "permission_request" => {
            Some(AgentState::WaitingPermission)
        }
        "question.asked" | "question.v2.asked" => Some(AgentState::WaitingInput),
        "Stop" | "session.idle" | "agent-turn-complete" | "SubagentStop" => {
            Some(AgentState::Completed)
        }
        "stop" => Some(cursor_stop_state(payload)),
        "SubagentStart" => Some(AgentState::Running),
        "StopFailure" | "session.error" => Some(AgentState::Failed),
        "sessionEnd" if kind == "cursor" => Some(cursor_session_end_state(payload)),
        "SessionEnd" | "session.deleted" | "sessionEnd" => Some(AgentState::Stopped),
        "Interrupt" => Some(AgentState::Unknown),
        "Notification" | "notification" => notification_state(payload),
        "session.status" => session_status_state(payload),
        _ => None,
    }
}

fn resume_for(kind: &str, provider: &str) -> Option<Resume> {
    let (program, arg) = match kind {
        "claude" => ("claude", "--resume"),
        "codex" => ("codex", "resume"),
        "opencode" => ("opencode", "--session"),
        "muse" => ("muse", "resume"),
        "grok" => ("grok", "--resume"),
        "cursor" => ("cursor-agent", "--resume"),
        _ => return None,
    };
    Some(Resume {
        program: program.into(),
        args: vec![arg.into(), provider.into()],
    })
}

pub fn normalize(
    kind: &str,
    session: &str,
    parent: &str,
    payload: &Value,
) -> Result<Option<HookEvent>> {
    let provider = first_str(
        payload,
        &[
            "session_id",
            "sessionId",
            "conversation_id",
            "conversationId",
            "thread-id",
            "sessionID",
            "/properties/sessionID",
            "/properties/info/id",
            "/data/sessionID",
            "/data/info/id",
        ],
    );
    let name = first_str(payload, &["hook_event_name", "type", "hookEventName"]).unwrap_or("");
    let tool = first_str(payload, &["tool_name", "toolName"])
        .unwrap_or("")
        .to_lowercase();
    let Some(state) = hook_state(kind, name, payload, &tool) else {
        return Ok(None);
    };
    let Some(provider) = provider else {
        return Ok(None);
    };
    let process = first_str(payload, &["terminator_invocation"]).unwrap_or(parent);
    let invocation = format!("{kind}:{process}:{provider}");
    let summary = first_str(
        payload,
        &[
            "message",
            "last_assistant_message",
            "last-assistant-message",
            "lastAssistantMessage",
        ],
    )
    .unwrap_or(state.label())
    .chars()
    .take(1000)
    .collect();
    // Deliberately omit tool input, prompts, transcripts and credentials from details.
    let details = format!("Agent: {kind}\nEvent: {name}\nSession: {provider}");
    let request = first_str(
        payload,
        &[
            "tool_use_id",
            "toolUseId",
            "request_id",
            "requestId",
            "/properties/id",
            "/properties/requestID",
            "/data/id",
            "/data/requestID",
        ],
    )
    .map(str::to_owned);
    Ok(Some(HookEvent {
        protocol_version: PROTOCOL_VERSION,
        event_id: first_str(payload, &["terminator_event"]).map_or_else(id, str::to_owned),
        terminal_session_id: session.into(),
        agent_invocation_id: invocation,
        agent_kind: kind.into(),
        provider_session_id: Some(provider.into()),
        state,
        request_id: request,
        sequence: payload.get("terminator_sequence").and_then(Value::as_u64),
        summary,
        details,
        resume: resume_for(kind, provider),
        // The hook helper attaches ancestor process identity after
        // normalization; the normalizer itself never inspects processes.
        process: None,
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
    #[test]
    fn opencode_plugin_forwards_permission_and_question_events() {
        let home = tempfile::tempdir().unwrap();
        let helper = Path::new("/tmp/space dir/hook");
        let path = install(home.path(), "opencode", helper, false).unwrap();
        let once = fs::read_to_string(&path).unwrap();
        assert!(once.contains(MARKER));
        assert!(once.contains("permission.v2.asked"));
        assert!(once.contains("question.v2.asked"));
        assert!(once.contains("question.rejected"));
        assert!(once.contains("export default"));
        assert!(once.contains("async setup(ctx)"));
        assert!(once.contains("async function server"));
        assert!(once.contains("/tmp/space dir/hook"));
        install(home.path(), "opencode", helper, false).unwrap();
        assert_eq!(once, fs::read_to_string(&path).unwrap());
        install(home.path(), "opencode", helper, true).unwrap();
        assert!(!path.exists());
    }
    #[test]
    fn opencode_permission_and_question_map_like_notifications() {
        let perm = normalize(
            "opencode",
            "s",
            "p",
            &json!({
                "type": "permission.asked",
                "properties": { "id": "req-1", "sessionID": "oc-1" }
            }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(perm.state, AgentState::WaitingPermission);
        assert_eq!(perm.provider_session_id.as_deref(), Some("oc-1"));
        assert_eq!(perm.request_id.as_deref(), Some("req-1"));
        assert_eq!(perm.resume.unwrap().args, vec!["--session", "oc-1"]);

        let perm_v2 = normalize(
            "opencode",
            "s",
            "p",
            &json!({
                "type": "permission.v2.asked",
                "data": { "id": "req-2", "sessionID": "oc-2" }
            }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(perm_v2.state, AgentState::WaitingPermission);
        assert_eq!(perm_v2.provider_session_id.as_deref(), Some("oc-2"));
        assert_eq!(perm_v2.request_id.as_deref(), Some("req-2"));

        let question = normalize(
            "opencode",
            "s",
            "p",
            &json!({
                "type": "question.asked",
                "properties": { "id": "q-1", "sessionID": "oc-1" }
            }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(question.state, AgentState::WaitingInput);

        let question_v2 = normalize(
            "opencode",
            "s",
            "p",
            &json!({
                "type": "question.v2.asked",
                "data": { "id": "q-2", "sessionID": "oc-2" }
            }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(question_v2.state, AgentState::WaitingInput);

        let replied = normalize(
            "opencode",
            "s",
            "p",
            &json!({
                "type": "question.v2.replied",
                "data": { "requestID": "q-2", "sessionID": "oc-2" }
            }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(replied.state, AgentState::Running);

        let rejected = normalize(
            "opencode",
            "s",
            "p",
            &json!({
                "type": "question.rejected",
                "properties": { "requestID": "q-1", "sessionID": "oc-1" }
            }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(rejected.state, AgentState::Running);
    }
    #[test]
    fn muse_settings_keep_user_keys_and_subagent_hooks() {
        let home = tempfile::tempdir().unwrap();
        let settings = home.path().join(".config/muse/settings.json");
        atomic_write(
            &settings,
            br#"{"schema_version":1,"model":"muse-spark","hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo mine"}]}]}}"#,
        )
        .unwrap();
        let helper = Path::new("/tmp/space dir/hook");
        install(home.path(), "muse", helper, false).unwrap();
        let once = fs::read_to_string(&settings).unwrap();
        assert!(once.contains(MARKER));
        assert!(once.contains("SubagentStop"));
        assert!(once.contains("echo mine"));
        assert!(once.contains("muse-spark"));
        assert!(
            fs::read_to_string(config_path(home.path(), "muse").unwrap())
                .unwrap()
                .contains("SubagentStop")
        );
        install(home.path(), "muse", helper, false).unwrap();
        assert_eq!(once, fs::read_to_string(&settings).unwrap());
        assert!(installed(home.path(), "muse"));
        install(home.path(), "muse", helper, true).unwrap();
        let after = fs::read_to_string(&settings).unwrap();
        assert!(after.contains("echo mine"));
        assert!(after.contains("muse-spark"));
        assert!(!after.contains(MARKER));
        assert!(!installed(home.path(), "muse"));
    }
    #[test]
    fn muse_subagent_stop_completes() {
        let event = normalize(
            "muse",
            "s",
            "p",
            &json!({"hook_event_name":"SubagentStop","session_id":"child"}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(event.state, AgentState::Completed);
        assert_eq!(event.provider_session_id.as_deref(), Some("child"));
    }
    #[test]
    fn opencode_unknown_events_ignored() {
        assert!(
            normalize(
                "opencode",
                "s",
                "p",
                &json!({"type":"file.edited","properties":{"sessionID":"x"}})
            )
            .unwrap()
            .is_none()
        );
    }
    #[test]
    fn cursor_hooks_keep_user_commands_and_skip_permission_events() {
        let home = tempfile::tempdir().unwrap();
        let path = config_path(home.path(), "cursor").unwrap();
        atomic_write(
            &path,
            br#"{"version":1,"extra":true,"hooks":{"stop":[{"command":"echo mine"}],"beforeShellExecution":[{"command":"echo gate"}]}}"#,
        )
        .unwrap();
        let helper = Path::new("/tmp/space dir/hook");
        install(home.path(), "cursor", helper, false).unwrap();
        let once = fs::read_to_string(&path).unwrap();
        install(home.path(), "cursor", helper, false).unwrap();
        assert_eq!(once, fs::read_to_string(&path).unwrap());
        let doc: Value = serde_json::from_str(&once).unwrap();
        assert_eq!(doc["version"].as_u64(), Some(1));
        assert_eq!(doc["extra"].as_bool(), Some(true));
        let hooks = doc["hooks"].as_object().unwrap();
        for event in [
            "sessionStart",
            "beforeSubmitPrompt",
            "postToolUse",
            "postToolUseFailure",
            "afterAgentResponse",
            "stop",
            "sessionEnd",
        ] {
            assert!(
                hooks[event]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|entry| entry["command"].as_str().unwrap_or("").contains(MARKER)),
                "{event}"
            );
        }
        assert!(
            hooks["stop"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["command"] == "echo mine")
        );
        assert_eq!(hooks["beforeShellExecution"][0]["command"], "echo gate");
        assert!(!hooks.contains_key("preToolUse"));
        assert!(!hooks.contains_key("beforeReadFile"));
        assert!(!hooks.contains_key("subagentStart"));
        let prompt = hooks["beforeSubmitPrompt"][0]["command"].as_str().unwrap();
        assert!(prompt.contains("{\"continue\":true}"));
        assert!(prompt.contains("/tmp/space dir/hook"));
        assert_eq!(hooks["sessionStart"][0]["timeout"].as_u64(), Some(2));
        let observe = hooks["stop"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["command"].as_str().unwrap_or("").contains(MARKER))
            .unwrap()["command"]
            .as_str()
            .unwrap();
        assert!(observe.contains("printf '%s\\n' '{}'"));
        install(home.path(), "cursor", helper, true).unwrap();
        let after = fs::read_to_string(&path).unwrap();
        assert!(after.contains("echo mine"));
        assert!(after.contains("echo gate"));
        assert!(after.contains("\"extra\": true"));
        assert!(!after.contains(MARKER));
        assert!(!installed(home.path(), "cursor"));
    }
    #[test]
    fn cursor_rejects_unknown_schema_and_nested_groups() {
        let home = tempfile::tempdir().unwrap();
        let path = config_path(home.path(), "cursor").unwrap();
        let versioned = "{\"version\":2,\"hooks\":{}}\n";
        atomic_write(&path, versioned.as_bytes()).unwrap();
        let helper = Path::new("/tmp/hook");
        assert!(install(home.path(), "cursor", helper, false).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), versioned);
        atomic_write(
            &path,
            br#"{"version":1,"hooks":{"stop":[{"hooks":[{"type":"command","command":"echo mine"}]}]}}"#,
        )
        .unwrap();
        let before = fs::read_to_string(&path).unwrap();
        assert!(install(home.path(), "cursor", helper, false).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
    }
    #[test]
    fn cursor_cli_session_end_reports_the_real_outcome_without_a_stop_hook() {
        // Sanitized payload from the real Cursor CLI Reply OK check.
        let payload: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/cursor-session-end-completed.json"
        ))
        .unwrap();
        let completed = normalize("cursor", "s", "p", &payload).unwrap().unwrap();
        assert_eq!(completed.state, AgentState::Completed);
        assert_eq!(completed.summary, "Completed");
        assert!(completed.state.actionable());
        for (reason, expected) in [
            ("error", AgentState::Failed),
            ("aborted", AgentState::Unknown),
            ("window_close", AgentState::Stopped),
            ("user_close", AgentState::Stopped),
            ("unrecognized", AgentState::Stopped),
        ] {
            let mut ended = payload.clone();
            ended["reason"] = json!(reason);
            // A stale final_status must not turn a user-close into completion.
            assert_eq!(
                normalize("cursor", "s", "p", &ended)
                    .unwrap()
                    .unwrap()
                    .state,
                expected
            );
        }
        assert_eq!(
            normalize("claude", "s", "p", &payload)
                .unwrap()
                .unwrap()
                .state,
            AgentState::Stopped
        );
    }

    #[test]
    fn cursor_events_map_conversation_without_prompt_text() {
        let completed = normalize(
            "cursor",
            "s",
            "p",
            &json!({
                "hook_event_name": "stop",
                "conversation_id": "chat-1",
                "status": "completed"
            }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(completed.state, AgentState::Completed);
        assert_eq!(completed.provider_session_id.as_deref(), Some("chat-1"));
        let resume = completed.resume.unwrap();
        assert_eq!(resume.program, "cursor-agent");
        assert_eq!(resume.args, vec!["--resume", "chat-1"]);
        let failed = normalize(
            "cursor",
            "s",
            "p",
            &json!({"hook_event_name":"stop","conversation_id":"chat-1","status":"error"}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(failed.state, AgentState::Failed);
        let aborted = normalize(
            "cursor",
            "s",
            "p",
            &json!({"hook_event_name":"stop","conversation_id":"chat-1","status":"aborted"}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(aborted.state, AgentState::Unknown);
        let prompt = normalize(
            "cursor",
            "s",
            "p",
            &json!({
                "hook_event_name": "beforeSubmitPrompt",
                "conversation_id": "chat-1",
                "prompt": "secret token"
            }),
        )
        .unwrap()
        .unwrap();
        assert_eq!(prompt.state, AgentState::Running);
        assert_eq!(prompt.summary, "Working");
        assert!(!prompt.details.contains("secret"));
        let ended = normalize(
            "cursor",
            "s",
            "p",
            &json!({"hook_event_name":"sessionEnd","session_id":"chat-1","reason":"user_close"}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(ended.state, AgentState::Stopped);
        assert!(
            normalize(
                "cursor",
                "s",
                "p",
                &json!({"hook_event_name":"workspaceOpen","cursor_version":"1.0"})
            )
            .unwrap()
            .is_none()
        );
    }
}
