use super::*;

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

struct HookStateInput<'a> {
    kind: &'a str,
    name: &'a str,
    payload: &'a Value,
    tool: &'a str,
}

fn hook_state(input: HookStateInput<'_>) -> Option<AgentState> {
    let HookStateInput {
        kind,
        name,
        payload,
        tool,
    } = input;
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

/// Payload keys naming the provider's own session, in lookup order.
const PROVIDER_SESSION_KEYS: &[&str] = &[
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
];

/// Payload keys carrying a human-readable summary, in lookup order.
const SUMMARY_KEYS: &[&str] = &[
    "message",
    "last_assistant_message",
    "last-assistant-message",
    "lastAssistantMessage",
];

/// Payload keys identifying the pending request, in lookup order.
const REQUEST_KEYS: &[&str] = &[
    "tool_use_id",
    "toolUseId",
    "request_id",
    "requestId",
    "/properties/id",
    "/properties/requestID",
    "/data/id",
    "/data/requestID",
];

pub struct NormalizeInput<'a> {
    pub kind: &'a str,
    pub session: &'a str,
    pub parent: &'a str,
    pub payload: &'a Value,
}

pub fn normalize(input: NormalizeInput<'_>) -> Result<Option<HookEvent>> {
    let NormalizeInput {
        kind,
        session,
        parent,
        payload,
    } = input;
    let provider = first_str(payload, PROVIDER_SESSION_KEYS);
    let name = first_str(payload, &["hook_event_name", "type", "hookEventName"]).unwrap_or("");
    let tool = first_str(payload, &["tool_name", "toolName"])
        .unwrap_or("")
        .to_lowercase();
    let Some(state) = hook_state(HookStateInput {
        kind,
        name,
        payload,
        tool: &tool,
    }) else {
        return Ok(None);
    };
    let Some(provider) = provider else {
        return Ok(None);
    };
    let process = first_str(payload, &["terminator_invocation"]).unwrap_or(parent);
    let invocation = format!("{kind}:{process}:{provider}");
    let summary = first_str(payload, SUMMARY_KEYS)
        .unwrap_or(state.label())
        .chars()
        .take(1000)
        .collect();
    // Deliberately omit tool input, prompts, transcripts and credentials from details.
    let details = format!("Agent: {kind}\nEvent: {name}\nSession: {provider}");
    let request = first_str(payload, REQUEST_KEYS).map(str::to_owned);
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
