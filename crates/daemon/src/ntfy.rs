//! Best-effort hook notifications; a bounded worker never holds daemon locks.
//! The worker also owns the access token: it reads and writes the OS
//! credential store, never an IPC request.
use crate::ntfy_token::{Keychain, TokenStore};
use std::{
    collections::HashMap, path::PathBuf, process::Command, sync::mpsc, thread, time::Duration,
};
use terminator_core::{
    HookEvent, Settings,
    process::{CommandOptions, run_command},
};
use zeroize::Zeroizing;

/// Notice text kept in a push.
const TEXT_LIMIT: usize = 500;

pub struct Ping {
    payload: Vec<u8>,
    endpoint: String,
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
        let options = &settings.ntfy_options;
        let mut message = format!("{agent}: {}", event.state.label());
        // Opt-in: the notice text can hold the agent's question.
        let text = notice_text(&event.summary);
        if options.include_text && !text.is_empty() && text != event.state.label() {
            message = format!("{message}\n{text}");
        }
        let mut payload = serde_json::json!({
            "topic": settings.ntfy_channel,
            "title": title,
            "message": message,
            "tags": ["robot"],
        });
        if !options.click.is_empty()
            && let Some(object) = payload.as_object_mut()
        {
            object.insert("click".into(), options.click.clone().into());
        }
        Some(Self {
            payload: serde_json::to_vec(&payload).ok()?,
            endpoint: settings.ntfy_endpoint(),
        })
    }

    /// curl reads the request from stdin as a config file, so the token
    /// never shows in a process list or a temporary file.
    fn curl_config(&self, token: Option<&str>) -> Vec<u8> {
        let mut config = String::from("header = \"Content-Type: application/json\"\n");
        if let Some(token) = token {
            config.push_str(&format!("header = \"Authorization: Bearer {token}\"\n"));
        }
        let body = String::from_utf8_lossy(&self.payload);
        config.push_str(&format!("data-binary = \"{}\"\n", quoted(&body)));
        config.push_str(&format!("url = \"{}\"\n", quoted(&self.endpoint)));
        config.into_bytes()
    }

    fn send(&self, token: Option<&str>) -> anyhow::Result<()> {
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
            if cfg!(windows) { "NUL" } else { "/dev/null" },
            "--config",
            "-",
        ]);
        run_command(
            command,
            CommandOptions {
                timeout: Duration::from_secs(10),
                input: Some(self.curl_config(token)),
                stdout_limit: 4096,
                stderr_limit: 4096,
                ..Default::default()
            },
        )?;
        Ok(())
    }
}

/// The notice text on one line set, without control characters.
fn notice_text(summary: &str) -> String {
    summary
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(TEXT_LIMIT)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// A curl config string body: backslash and quote escaped.
fn quoted(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Work for the ntfy worker thread.
pub enum Job {
    Ping(Ping),
    /// A new access token; empty removes it.
    Token(Zeroizing<String>),
}

/// Delivers pings with the saved token. The token is read from `store`
/// once, on the first ping, and replaced when a new one is saved.
fn run_worker(
    rx: &mpsc::Receiver<Job>,
    mut store: Option<Box<dyn TokenStore>>,
    mut deliver: impl FnMut(&Ping, Option<&str>) -> anyhow::Result<()>,
) {
    let mut token: Option<Option<Zeroizing<String>>> = None;
    while let Ok(job) = rx.recv() {
        match job {
            Job::Token(new) => {
                let new = (!new.is_empty()).then_some(new);
                let saved = store
                    .as_mut()
                    .map(|store| store.save(new.as_ref().map(|t| t.as_str())));
                if !matches!(saved, Some(Ok(()))) {
                    crate::console::Console::err(format_args!("ntfy token could not be saved"));
                }
                token = Some(new);
            }
            Job::Ping(ping) => {
                let token = token.get_or_insert_with(|| {
                    store.as_mut().and_then(|store| store.load().ok().flatten())
                });
                if deliver(&ping, token.as_ref().map(|t| t.as_str())).is_err() {
                    // Do not expose the channel, token or provider text in logs.
                    crate::console::Console::err(format_args!("ntfy notification delivery failed"));
                }
            }
        }
    }
}

/// Repeat pushes from one agent run and state are dropped for a minute;
/// any push also holds back every other push for a minute so a
/// multi-agent burst buzzes the phone once instead of once per agent.
pub const PER_AGENT_WINDOW_SECS: u64 = 60;
pub const GLOBAL_WINDOW_SECS: u64 = 60;

/// In-memory dispatch gate. The Agents inbox keeps every notification; this
/// only decides which ones also buzz the phone.
#[derive(Default)]
pub struct Cooldown {
    last_global: Option<u64>,
    last_per_key: HashMap<String, u64>,
}

impl Cooldown {
    /// Key by agent invocation plus state, e.g. from the hook event.
    pub fn allow(&mut self, key: &str, now: u64) -> bool {
        if self
            .last_global
            .is_some_and(|last| now.saturating_sub(last) < GLOBAL_WINDOW_SECS)
        {
            return false;
        }
        if self
            .last_per_key
            .get(key)
            .is_some_and(|last| now.saturating_sub(*last) < PER_AGENT_WINDOW_SECS)
        {
            return false;
        }
        self.last_global = Some(now);
        self.last_per_key.insert(key.to_string(), now);
        if self.last_per_key.len() > 1024 {
            self.last_per_key
                .retain(|_, last| now.saturating_sub(*last) < PER_AGENT_WINDOW_SECS);
        }
        true
    }
}

/// Starts the worker. `data` names the credential store entry.
pub fn start(data: PathBuf) -> mpsc::SyncSender<Job> {
    let (tx, rx) = mpsc::sync_channel::<Job>(64);
    thread::spawn(move || {
        // Fixtures neither deliver nor touch the OS credential store.
        if std::env::var_os("TERMINATOR_NO_NOTIFICATIONS").is_some() {
            run_worker(&rx, None, |_, _| Ok(()));
            return;
        }
        let store = Keychain::new(&data)
            .ok()
            .map(|store| Box::new(store) as Box<dyn TokenStore>);
        run_worker(&rx, store, Ping::send);
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
            process: None,
            usage: None,
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

    fn payload(settings: &Settings) -> serde_json::Value {
        let ping = Ping::from_event(settings, &event()).unwrap();
        serde_json::from_slice(&ping.payload).unwrap()
    }

    #[test]
    fn ntfy_notice_text_and_click_are_opt_in() {
        let mut settings = Settings {
            ntfy_enabled: true,
            ntfy_channel: "fixture".into(),
            ..Default::default()
        };
        let plain = payload(&settings);
        assert!(plain.get("click").is_none());
        assert!(!plain.to_string().contains("private"));
        settings.ntfy_options.include_text = true;
        settings.ntfy_options.click = "https://example.com/agents".into();
        let rich = payload(&settings);
        assert_eq!(rich["click"], "https://example.com/agents");
        let message = rich["message"].as_str().unwrap();
        assert!(message.ends_with("\nprivate prompt"), "{message}");
        // An invalid click URL disables the push instead of sending it.
        settings.ntfy_options.click = "javascript:alert(1)".into();
        assert!(Ping::from_event(&settings, &event()).is_none());
    }

    #[test]
    fn ntfy_uses_the_configured_server() {
        let mut settings = Settings {
            ntfy_enabled: true,
            ntfy_channel: "fixture".into(),
            ..Default::default()
        };
        let ping = Ping::from_event(&settings, &event()).unwrap();
        assert_eq!(ping.endpoint, "https://ntfy.sh");
        settings.ntfy_options.server = "https://push.example.com/".into();
        let ping = Ping::from_event(&settings, &event()).unwrap();
        assert_eq!(ping.endpoint, "https://push.example.com");
        let config = String::from_utf8(ping.curl_config(Some("tk_secret"))).unwrap();
        assert!(config.contains("header = \"Authorization: Bearer tk_secret\"\n"));
        assert!(config.contains("url = \"https://push.example.com\"\n"));
        assert!(config.contains("\\\"topic\\\":\\\"fixture\\\""), "{config}");
        let config = String::from_utf8(ping.curl_config(None)).unwrap();
        assert!(!config.contains("Authorization"));
    }

    /// An in-memory credential store; tests never touch the OS keychain.
    struct Memory(std::sync::Arc<std::sync::Mutex<(Option<String>, usize)>>);
    impl TokenStore for Memory {
        fn load(&mut self) -> anyhow::Result<Option<Zeroizing<String>>> {
            let mut state = self.0.lock().unwrap();
            state.1 = state.1.saturating_add(1);
            Ok(state.0.clone().map(Zeroizing::new))
        }
        fn save(&mut self, token: Option<&str>) -> anyhow::Result<()> {
            self.0.lock().unwrap().0 = token.map(str::to_owned);
            Ok(())
        }
    }

    #[test]
    fn the_worker_reads_the_token_once_and_uses_a_new_one_at_once() {
        let state = std::sync::Arc::new(std::sync::Mutex::new((Some("old".to_owned()), 0)));
        let (tx, rx) = mpsc::sync_channel(8);
        let settings = Settings {
            ntfy_enabled: true,
            ntfy_channel: "fixture".into(),
            ..Default::default()
        };
        for job in [
            Job::Ping(Ping::from_event(&settings, &event()).unwrap()),
            Job::Ping(Ping::from_event(&settings, &event()).unwrap()),
            Job::Token(Zeroizing::new("new".into())),
            Job::Ping(Ping::from_event(&settings, &event()).unwrap()),
            Job::Token(Zeroizing::new(String::new())),
            Job::Ping(Ping::from_event(&settings, &event()).unwrap()),
        ] {
            tx.send(job).unwrap();
        }
        drop(tx);
        let mut used = Vec::new();
        run_worker(
            &rx,
            Some(Box::new(Memory(std::sync::Arc::clone(&state)))),
            |_, token| {
                used.push(token.map(str::to_owned));
                Ok(())
            },
        );
        assert_eq!(
            used,
            [
                Some("old".into()),
                Some("old".into()),
                Some("new".into()),
                None
            ]
        );
        let state = state.lock().unwrap();
        assert_eq!(state.1, 1, "one credential store read");
        assert_eq!(state.0, None, "an empty token removes the saved one");
    }

    #[test]
    fn cooldown_holds_back_same_agent_repeats_and_global_bursts() {
        let mut gate = Cooldown::default();
        assert!(gate.allow("a:waiting", 1000));
        // Same agent run and state inside the per-agent window: silent.
        assert!(!gate.allow("a:waiting", 1001));
        assert!(!gate.allow("a:waiting", 1059));
        // A different agent inside the global window: silent too.
        assert!(!gate.allow("b:waiting", 1001));
        assert!(!gate.allow("b:waiting", 1059));
        // Global window elapsed, fresh key: push.
        assert!(gate.allow("b:waiting", 1060));
        // A state change from the same run is its own key once global clears.
        assert!(!gate.allow("a:completed", 1061));
        assert!(gate.allow("a:completed", 1120));
        // Per-agent window elapsed: push again once global clears.
        assert!(!gate.allow("a:waiting", 1121));
        assert!(gate.allow("a:waiting", 1180));
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
            assert!(headers.contains("Authorization: Bearer tk_fixture\r\n"));
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
            ntfy_options: terminator_core::NtfyOptions {
                server: endpoint,
                ..Default::default()
            },
            ..Default::default()
        };
        Ping::from_event(&settings, &event())
            .unwrap()
            .send(Some("tk_fixture"))
            .unwrap();
        server.join().unwrap();
    }
}
