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
#[must_use]
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

fn hook_state(name: &str, payload: &Value, tool: &str) -> Option<AgentState> {
    match name {
        "SessionStart" | "session.created" => Some(AgentState::Unknown),
        "UserPromptSubmit"
        | "PostToolUse"
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
        "Stop" | "session.idle" | "agent-turn-complete" => Some(AgentState::Completed),
        "StopFailure" | "session.error" => Some(AgentState::Failed),
        "SessionEnd" | "session.deleted" => Some(AgentState::Stopped),
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
    let Some(state) = hook_state(name, payload, &tool) else {
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
}
