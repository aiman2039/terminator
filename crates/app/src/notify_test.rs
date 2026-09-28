//! Notification self-tests: direct ntfy posts and per-agent hello events.
//!
//! The ntfy test posts straight to ntfy.sh with the draft channel, so it needs
//! neither Apply nor a live terminal. The hello test goes through the daemon's
//! normal hook path, so it verifies the Agents inbox plus ntfy end to end and
//! does need saved settings and a live terminal.
use std::{process::Command, time::Duration};
use terminator_core::{
    AgentState, HookEvent, PROTOCOL_VERSION, id,
    process::{CommandOptions, run_command},
};

pub const ENDPOINT: &str = "https://ntfy.sh";

/// Same channel/machine rules as [`terminator_core::Settings::validate`]; the
/// test must not depend on the rest of the settings draft being valid.
pub fn validate(channel: &str, machine: &str) -> Result<(), String> {
    if channel.is_empty()
        || channel.len() > 128
        || !channel
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(
            "ntfy channel must contain 1–128 letters, digits, underscores or hyphens".into(),
        );
    }
    if machine.len() > 128 || machine.chars().any(char::is_control) {
        return Err(
            "ntfy machine name must be at most 128 bytes without control characters".into(),
        );
    }
    Ok(())
}

fn title(machine: &str) -> String {
    if machine.is_empty() {
        "test: Needs input".into()
    } else {
        format!("[{machine}] test: Needs input")
    }
}

pub fn payload(channel: &str, machine: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "topic": channel,
        "title": title(machine),
        "message": "test: Needs input",
        "tags": ["robot"],
    }))
    .unwrap_or_default()
}

/// Copy/paste equivalent of [`send`] for any terminal.
#[must_use]
pub fn curl_command(channel: &str, machine: &str) -> String {
    let body = String::from_utf8_lossy(&payload(channel, machine)).into_owned();
    format!(
        "curl --disable --silent --show-error --fail --max-time 8 --output /dev/null --header 'Content-Type: application/json' --data-binary {} --url {ENDPOINT}",
        terminator_core::quote(&body)
    )
}

/// Blocking POST; call on a worker, never during rendering.
pub fn send(endpoint: &str, channel: &str, machine: &str) -> anyhow::Result<()> {
    validate(channel, machine).map_err(anyhow::Error::msg)?;
    let mut command = Command::new("curl");
    command.args([
        "--disable",
        "--silent",
        "--show-error",
        "--fail",
        "--max-time",
        "8",
        "--output",
        "/dev/null",
        "--header",
        "Content-Type: application/json",
        "--data-binary",
        "@-",
        "--url",
        endpoint,
    ]);
    run_command(
        command,
        CommandOptions {
            timeout: Duration::from_secs(10),
            input: Some(payload(channel, machine)),
            stdout_limit: 4096,
            stderr_limit: 4096,
            ..Default::default()
        },
    )?;
    Ok(())
}

/// One `WaitingInput` hello per built-in agent for `session`. Fresh ids every
/// call so repeats are never deduplicated.
#[must_use]
pub fn hello_events(session: &str) -> Vec<HookEvent> {
    terminator_integrations::AGENTS
        .iter()
        .map(|kind| HookEvent {
            protocol_version: PROTOCOL_VERSION,
            event_id: format!("hello-test-{}-{kind}", id()),
            terminal_session_id: session.into(),
            agent_invocation_id: format!("hello-test-invocation-{}-{kind}", id()),
            agent_kind: (*kind).into(),
            provider_session_id: None,
            state: AgentState::WaitingInput,
            request_id: Some(format!("hello-test-request-{}-{kind}", id())),
            sequence: None,
            summary: format!("hello from {kind} test"),
            details: "hello".into(),
            resume: None,
            process: None,
        })
        .collect()
}

/// Copy/paste loop with the same hello events. Must run inside a Terminator
/// terminal so `emit` inherits the session; the payload session is overridden.
#[must_use]
pub fn hello_command(helper: &str) -> String {
    let agents = terminator_integrations::AGENTS.join(" ");
    let event = "{\"protocol_version\":1,\"event_id\":\"hello-test-%s\",\"terminal_session_id\":\"ignored\",\"agent_invocation_id\":\"hello-test-invocation-%s\",\"agent_kind\":\"%s\",\"provider_session_id\":null,\"state\":\"waiting_input\",\"request_id\":\"hello-test-request-%s\",\"sequence\":null,\"summary\":\"hello from %s test\",\"details\":\"hello\",\"resume\":null}";
    format!(
        "for agent in {agents}; do nonce=\"$(date +%s)-$RANDOM-$agent\"; printf '{event}' \"$nonce\" \"$nonce\" \"$agent\" \"$nonce\" \"$agent\" | {} emit; done",
        terminator_core::quote(helper)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use terminator_core::Settings;

    #[test]
    fn test_validation_matches_settings_rules() {
        assert!(validate("phone_123-test", "My laptop").is_ok());
        for channel in [
            "",
            "https://ntfy.sh/topic",
            "a/b",
            "a?b",
            "a\nB",
            "has space",
        ] {
            assert!(validate(channel, "").is_err(), "{channel}");
        }
        assert!(validate("valid", "bad\nheader").is_err());
        // The widget copy stays valid for Settings too.
        let settings = Settings {
            ntfy_enabled: true,
            ntfy_channel: "phone_123-test".into(),
            ntfy_machine: "My laptop".into(),
            ..Default::default()
        };
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn test_payload_matches_daemon_shape() {
        let json: serde_json::Value =
            serde_json::from_slice(&payload("fixture", "laptop")).unwrap();
        assert_eq!(json["topic"], "fixture");
        assert_eq!(json["title"], "[laptop] test: Needs input");
        assert_eq!(json["message"], "test: Needs input");
        assert_eq!(json["tags"], serde_json::json!(["robot"]));
        let json: serde_json::Value = serde_json::from_slice(&payload("fixture", "")).unwrap();
        assert_eq!(json["title"], "test: Needs input");
    }

    #[test]
    fn test_posts_json_to_http_server() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            let length: usize = String::from_utf8(request)
                .unwrap()
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .map(|n| n.parse().unwrap())
                })
                .unwrap();
            let mut body = vec![0; length];
            stream.read_exact(&mut body).unwrap();
            let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(payload["topic"], "fixture");
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
                .unwrap();
        });
        send(&endpoint, "fixture", "laptop").unwrap();
        server.join().unwrap();
    }

    #[test]
    fn copy_commands_survive_tricky_machine_names() {
        // Single quotes in the machine name must not break the shell quoting.
        let command = curl_command("fixture", "o'brien");
        assert!(command.contains("https://ntfy.sh"));
        assert!(command.contains("'\\''"));
        let rendered = hello_command("/tmp/dir with space/terminator-hook");
        for kind in terminator_integrations::AGENTS {
            assert!(rendered.contains(kind), "{rendered}");
        }
        assert!(rendered.contains("emit"));
        assert!(rendered.contains("ignored"));
    }

    #[test]
    fn hello_events_cover_every_agent_without_dedup() {
        let events = hello_events("session");
        assert_eq!(events.len(), terminator_integrations::AGENTS.len());
        let mut ids = std::collections::HashSet::new();
        for event in &events {
            assert_eq!(event.protocol_version, PROTOCOL_VERSION);
            assert_eq!(event.terminal_session_id, "session");
            assert_eq!(event.state, AgentState::WaitingInput);
            assert!(event.state.actionable());
            assert!(event.summary.contains("hello"));
            assert!(event.request_id.is_some());
            assert!(ids.insert(event.event_id.clone()));
            assert!(ids.insert(event.agent_invocation_id.clone()));
        }
        let kinds: Vec<_> = events.iter().map(|e| e.agent_kind.as_str()).collect();
        assert_eq!(kinds, terminator_integrations::AGENTS);
        // A second batch never reuses ids, so repeats still notify.
        let again = hello_events("session");
        for event in &again {
            assert!(ids.insert(event.event_id.clone()));
        }
    }

    #[test]
    fn hello_events_notify_through_saved_settings() {
        let mut state = terminator_core::State::default();
        state.sessions.push(terminator_core::Session {
            review: false,
            id: "session".into(),
            project_id: "project".into(),
            label: "shell".into(),
            cwd: std::path::PathBuf::from("/tmp"),
            kind: terminator_core::SessionKind::Shell,
            file: None,
            lifecycle: terminator_core::Lifecycle::Running,
            created: 0,
            exit_code: None,
            rows: 24,
            cols: 80,
            generation: String::new(),
            pid: None,
            truncated: false,
            cwd_confirmed: false,
        });
        for event in hello_events("session") {
            assert!(state.apply_hook(event).unwrap().is_some());
        }
        assert_eq!(
            state.notifications.len(),
            terminator_integrations::AGENTS.len()
        );
    }
}
