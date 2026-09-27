//! Best-effort hook notifications; a bounded worker never holds daemon locks.
use std::{process::Command, sync::mpsc, thread, time::Duration};
use terminator_core::{
    HookEvent, Settings,
    process::{CommandOptions, run_command},
};

pub struct Ping {
    payload: Vec<u8>,
}
impl Ping {
    pub fn from_event(settings: &Settings, event: &HookEvent) -> Option<Self> {
        if !settings.ntfy_enabled
            || settings.ntfy_channel.is_empty()
            || settings.validate().is_err()
        {
            return None;
        }
        let agent: String = event
            .agent_kind
            .chars()
            .filter(|c| !c.is_control())
            .take(80)
            .collect();
        let title = if settings.ntfy_machine.is_empty() {
            format!("{agent}: {}", event.state.label())
        } else {
            format!(
                "[{}] {agent}: {}",
                settings.ntfy_machine,
                event.state.label()
            )
        };
        Some(Self {
            payload: serde_json::to_vec(&serde_json::json!({
                "topic": settings.ntfy_channel,
                "title": title,
                "message": format!("{agent}: {}", event.state.label()),
                "tags": ["robot"],
            }))
            .ok()?,
        })
    }

    fn send(self, endpoint: &str) -> anyhow::Result<()> {
        // Disable curlrc: user defaults must not add retries, redirects or extra recipients.
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
                input: Some(self.payload),
                stdout_limit: 4096,
                stderr_limit: 4096,
                ..Default::default()
            },
        )?;
        Ok(())
    }
}

pub fn start() -> mpsc::SyncSender<Ping> {
    let (tx, rx) = mpsc::sync_channel::<Ping>(64);
    thread::spawn(move || {
        while let Ok(ping) = rx.recv() {
            if std::env::var_os("TERMINATOR_NO_NOTIFICATIONS").is_none()
                && ping.send("https://ntfy.sh").is_err()
            {
                // Do not expose the channel or provider text in logs.
                eprintln!("ntfy notification delivery failed");
            }
        }
    });
    tx
}

#[cfg(test)]
mod tests {
    use super::*;
    use terminator_core::{AgentState, PROTOCOL_VERSION};
    fn event() -> HookEvent {
        HookEvent {
            protocol_version: PROTOCOL_VERSION,
            event_id: "event".into(),
            terminal_session_id: "session".into(),
            agent_invocation_id: "invocation".into(),
            agent_kind: "codex".into(),
            provider_session_id: None,
            state: AgentState::WaitingPermission,
            request_id: None,
            sequence: None,
            summary: "private prompt".into(),
            details: "private details".into(),
            resume: None,
        }
    }
    #[test]
    fn ntfy_is_opt_in_and_excludes_provider_content() {
        let mut settings = Settings::default();
        assert!(Ping::from_event(&settings, &event()).is_none());
        settings.ntfy_channel = "test-channel".into();
        assert!(Ping::from_event(&settings, &event()).is_none());
        settings.ntfy_enabled = true;
        settings.ntfy_machine = "laptop".into();
        let ping = Ping::from_event(&settings, &event()).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&ping.payload).unwrap();
        assert_eq!(json["topic"], "test-channel");
        assert!(
            json["title"]
                .as_str()
                .unwrap()
                .starts_with("[laptop] codex:")
        );
        assert!(!String::from_utf8(ping.payload).unwrap().contains("private"));
        settings.ntfy_channel.clear();
        assert!(Ping::from_event(&settings, &event()).is_none());
    }

    #[test]
    fn ntfy_posts_json_to_http_server() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
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
            let headers = String::from_utf8(request).unwrap();
            assert!(headers.starts_with("POST / HTTP/1.1"));
            let length: usize = headers
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
        let settings = Settings {
            ntfy_enabled: true,
            ntfy_channel: "fixture".into(),
            ..Default::default()
        };
        Ping::from_event(&settings, &event())
            .unwrap()
            .send(&endpoint)
            .unwrap();
        server.join().unwrap();
    }
}
