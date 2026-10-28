use anyhow::{Context, Result, anyhow, ensure};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};
use terminator_core::{read_frame, transport, write_frame};

mod editor_steps;
mod records;
use records::session_summary;
pub use records::{
    Process, artifacts, bin, first_layout, git, id, output, require_first_layout, root, session,
    session_closed, session_ids, session_present, sessions, wait_child,
};

pub struct Harness {
    pub _temp: tempfile::TempDir,
    pub root: PathBuf,
    pub env: BTreeMap<String, String>,
    pub daemon: Option<Process>,
}
impl Harness {
    pub fn new() -> Result<Self> {
        Self::configured(None, false)
    }
    /// Isolated daemon harness with an optional explicit daemon executable and
    /// OS notifications enabled. Notification benchmarks need the real
    /// notification path, so they opt out of the quiet default.
    pub fn configured(daemon: Option<PathBuf>, notifications: bool) -> Result<Self> {
        let temp = tempfile::Builder::new()
            .prefix("term-")
            .tempdir_in("/tmp")?;
        let root = temp.path().canonicalize()?;
        let mut env = BTreeMap::new();
        for (key, path) in [
            ("TERMINATOR_DATA_DIR", root.clone()),
            ("XDG_CONFIG_HOME", root.join("config")),
            ("XDG_DATA_HOME", root.join("data")),
            ("XDG_STATE_HOME", root.join("state")),
            ("NVIM_LOG_FILE", root.join("nvim.log")),
            ("TERMINATOR_CONFIG_DIR", root.join("appearance")),
        ] {
            env.insert(key.into(), path.to_string_lossy().into_owned());
        }
        if !notifications {
            env.insert("TERMINATOR_NO_NOTIFICATIONS".into(), "1".into());
        }
        if let Some(daemon) = daemon {
            ensure!(
                daemon.is_file(),
                "daemon binary missing: {}",
                daemon.display()
            );
            env.insert(
                "TERMINATOR_FIXTURE_DAEMON".into(),
                daemon.to_string_lossy().into_owned(),
            );
        }
        let mut harness = Self {
            _temp: temp,
            root,
            env,
            daemon: None,
        };
        harness.start()?;
        Ok(harness)
    }
    pub fn command(&self, name: &str) -> Command {
        let executable = if name == "terminator" {
            self.env
                .get("TERMINATOR_FIXTURE_GUI")
                .map_or_else(|| bin().join(name), PathBuf::from)
        } else if name == "terminator-daemon" {
            self.env
                .get("TERMINATOR_FIXTURE_DAEMON")
                .map_or_else(|| bin().join(name), PathBuf::from)
        } else {
            bin().join(name)
        };
        let mut c = Command::new(executable);
        clear_inherited_terminator(&mut c, std::env::vars_os().map(|(key, _)| key));
        c.env_remove("TERMINATOR_SESSION_ID")
            .env_remove("TERMINATOR_SESSION_TOKEN")
            .env_remove("TERMINATOR_RUNTIME_DIR")
            .env_remove("TERMINATOR_CONFIG_HOME")
            .env_remove("VIMINIT")
            .env_remove("EXINIT")
            .envs(&self.env);
        if name == "terminator" {
            configure_gui(
                &mut c,
                std::env::var_os("TERMINATOR_FIXTURE_RENDERER"),
                fixture_visible(),
            );
        }
        c
    }
    pub fn start(&mut self) -> Result<()> {
        let log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.root.join("daemon.log"))?;
        let mut command = self.command("terminator-daemon");
        command.stdout(log.try_clone()?).stderr(log);
        self.daemon =
            Some(Process(command.spawn().context(
                "Build workspace binaries before running fixtures",
            )?));
        let end = Instant::now()
            .checked_add(Duration::from_secs(10))
            .ok_or_else(|| anyhow!("daemon startup deadline overflow"))?;
        while self.state().is_err() {
            ensure!(
                self.daemon.as_mut().unwrap().0.try_wait()?.is_none(),
                "Daemon startup failed: {}",
                fs::read_to_string(self.root.join("daemon.log"))?
            );
            ensure!(Instant::now() < end, "Daemon startup timeout");
            thread::sleep(Duration::from_millis(30));
        }
        Ok(())
    }
    pub fn connect(
        &self,
        request: Value,
        token: Option<&str>,
        hint: Option<Value>,
    ) -> Result<transport::Stream> {
        let paths = terminator_core::generations::owner_for(
            &terminator_core::Paths::at(self.root.clone()),
            &serde_json::from_value(request.clone())?,
        )?;
        let mut stream = transport::Stream::connect_ipc(&paths.socket())?;
        stream.set_timeouts(Duration::from_secs(3), Duration::from_secs(3))?;
        let auth = paths.token()?;
        let mut envelope =
            json!({"version":1,"auth":token.unwrap_or(auth.trim()),"request":request});
        if let Some(hint) = hint {
            envelope
                .as_object_mut()
                .ok_or_else(|| anyhow!("request envelope is not an object"))?
                .insert("snapshot_hint".into(), hint);
        }
        write_frame(&mut stream, &envelope)?;
        Ok(stream)
    }
    pub fn rpc(&self, request: Value) -> Result<Value> {
        Ok(serde_json::to_value(terminator_core::rpc(
            &terminator_core::Paths::at(self.root.clone()),
            serde_json::from_value(request)?,
        )?)?)
    }
    pub fn state(&self) -> Result<Value> {
        self.rpc(json!("Snapshot"))?
            .get("State")
            .cloned()
            .ok_or_else(|| anyhow!("missing State"))
    }
    #[track_caller]
    pub fn wait(&self, mut check: impl FnMut(&Value) -> bool, seconds: u64) -> Result<Value> {
        let caller = std::panic::Location::caller();
        let started = Instant::now();
        let end = started
            .checked_add(Duration::from_secs(seconds))
            .ok_or_else(|| anyhow!("wait deadline overflow"))?;
        let mut warned = false;
        loop {
            let state = self.state()?;
            if check(&state) {
                return Ok(state);
            }
            let elapsed = started.elapsed();
            if !warned && elapsed >= Duration::from_secs(2) {
                eprintln!(
                    "slow wait at {}:{} ({:.1}s of {seconds}s)",
                    caller.file(),
                    caller.line(),
                    elapsed.as_secs_f32()
                );
                warned = true;
            }
            ensure!(
                Instant::now() < end,
                "State assertion timed out after {seconds}s at {}:{} ({})",
                caller.file(),
                caller.line(),
                session_summary(&state)
            );
            thread::sleep(Duration::from_millis(30));
        }
    }
    pub fn project(&self, name: &str) -> Result<Value> {
        let path = self.root.join(name);
        fs::create_dir_all(&path)?;
        self.rpc(json!({"AddProject":{"path":path}}))?;
        self.state()?
            .get("projects")
            .ok_or_else(|| anyhow!("missing projects"))?
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["path"] == path.to_string_lossy().as_ref())
            .cloned()
            .context("Created project missing")
    }
    pub fn shell(&self, project: &Value) -> Result<Value> {
        self.rpc(json!({"Create":{"project":id(project),"cwd":null,"file":null,"line":null,"editor":false}}))?
            .get("Created")
            .cloned()
            .ok_or_else(|| anyhow!("missing Created"))
    }
    pub fn editor(&self, project: &Value, path: &Path) -> Result<Value> {
        self.rpc(json!({"Create":{"project":id(project),"cwd":project["path"],"file":path,"line":null,"editor":true}}))?
            .get("Created")
            .cloned()
            .ok_or_else(|| anyhow!("missing Created"))
    }
    pub fn attach(&self, session: &Value) -> Result<transport::Stream> {
        let mut stream = self.connect(
            json!({"Attach":{"session":id(session),"rows":24,"cols":80}}),
            None,
            None,
        )?;
        let _: Value = read_frame(&mut stream)
            .with_context(|| format!("Fixture Attach failed for session {}", id(session)))?;
        Ok(stream)
    }
    pub fn write(&self, stream: &mut transport::Stream, text: &str) -> Result<()> {
        write_frame(stream, &json!({"Input":{"data":B64.encode(text)}}))?;
        stream.flush()?;
        Ok(())
    }
    pub fn history(&self, sid: &str) -> Result<String> {
        Ok(self
            .rpc(json!({"History":{"session":sid}}))?
            .get("Text")
            .and_then(Value::as_str)
            .context("History text missing")?
            .into())
    }
    /// Fixture baseline: a pin on classic top-level tabs plus `/bin/sh`.
    /// Fresh preferences default to IDE mode, which moves shells into the
    /// bottom strip before the scripted target is painted; fixtures assert
    /// the classic strip. The pin lands before the first GUI capture and a
    /// fixture that needs IDE mode writes its own preferences afterwards.
    pub fn setup(&self) -> Result<()> {
        let preferences = self.root.join("ui-preferences.json");
        if !preferences.exists() {
            fs::write(
                &preferences,
                serde_json::to_vec(&json!({"version": 2, "ide_mode": false}))?,
            )?;
        }
        let mut settings = self
            .state()?
            .get("settings")
            .cloned()
            .ok_or_else(|| anyhow!("missing settings"))?;
        settings
            .as_object_mut()
            .ok_or_else(|| anyhow!("settings is not an object"))?
            .insert("shell".into(), json!("/bin/sh"));
        self.rpc(json!({"Settings":settings}))?;
        Ok(())
    }
    /// Route terminal notices to the OS so the daemon exercises its real
    /// notification path (Cocoa pump + Notification Center enumeration).
    pub fn enable_os_notifications(&self) -> Result<()> {
        let mut settings = self
            .state()?
            .get("settings")
            .cloned()
            .ok_or_else(|| anyhow!("missing settings"))?;
        settings
            .as_object_mut()
            .ok_or_else(|| anyhow!("settings is not an object"))?
            .insert("terminal_notifications_os".into(), json!(true));
        self.rpc(json!({"Settings":settings}))?;
        Ok(())
    }
    pub fn terminal_notify(&self, session: &str, title: &str, body: &str) -> Result<()> {
        self.rpc(json!({
            "TerminalNotify": {"session": session, "title": title, "body": body}
        }))?;
        Ok(())
    }
    pub fn layout(&self, project: &Value, sessions: &[Value]) -> Result<Value> {
        let mut command = Command::new(bin().join("examples/layout"));
        command.args(sessions.iter().map(id));
        let layout: Value = serde_json::from_slice(&output(command)?)?;
        self.rpc(json!({"SaveLayout":{"project":id(project),"layout":layout}}))?;
        Ok(layout)
    }
    pub fn restart(&mut self) -> Result<()> {
        self.rpc(json!("Shutdown"))
            .context("Fixture daemon shutdown before restart failed")?;
        wait_child(&mut self.daemon.as_mut().unwrap().0, Duration::from_secs(5))?;
        self.daemon.take();
        self.start()
    }
    pub fn assert_pids(&self, original: &[Value]) -> Result<()> {
        let state = self.state()?;
        for old in original {
            let current = session(&state, id(old));
            ensure!(
                current["pid"] == old["pid"] && current["lifecycle"] == "running",
                "Original shell changed identity or exited"
            );
        }
        Ok(())
    }
}

pub(crate) fn fixture_visible() -> bool {
    std::env::var("TERMINATOR_FIXTURE_VISIBLE").is_ok_and(|value| value == "1")
}

fn configure_gui(command: &mut Command, renderer: Option<std::ffi::OsString>, visible: bool) {
    command
        .env(
            "TERMINATOR_RENDERER",
            renderer.unwrap_or_else(|| "glow".into()),
        )
        .env("TERMINATOR_TEST_RENDER_OCCLUDED", "1")
        .env_remove("TERMINATOR_TEST_NATIVE_INPUT")
        .env_remove("TERMINATOR_TEST_VISIBLE_CAPTURE")
        .env_remove("TERMINATOR_TEST_FOREGROUND");
    if visible {
        command.env_remove("TERMINATOR_TEST_BACKGROUND");
    } else {
        command.env("TERMINATOR_TEST_BACKGROUND", "1");
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        if let Ok(state) = self.state() {
            self.stop_live_sessions(&state);
            self.shutdown_owners();
            if let Some(daemon) = &mut self.daemon {
                let _ = wait_child(&mut daemon.0, Duration::from_secs(5));
            }
        }
        self.daemon.take();
    }
}
impl Harness {
    fn stop_live_sessions(&self, state: &Value) {
        for session in sessions(state) {
            if matches!(session["lifecycle"].as_str(), Some("running" | "stopping")) {
                let _ = self.rpc(json!({"Stop":{"session":id(session)}}));
            }
        }
        let _ = self.wait(
            |state| {
                sessions(state)
                    .iter()
                    .all(|s| !matches!(s["lifecycle"].as_str(), Some("running" | "stopping")))
            },
            5,
        );
    }
    fn shutdown_owners(&self) {
        let paths = terminator_core::Paths::at(self.root.clone());
        if let Ok(catalog) = terminator_core::generations::Catalog::open(&paths)
            && let Ok(owners) = catalog.generations()
        {
            for owner in owners {
                Self::shutdown_owner(&owner.paths());
            }
        } else {
            let _ = self.rpc(json!("Shutdown"));
        }
    }
    fn shutdown_owner(endpoint: &terminator_core::Paths) {
        let _ = terminator_core::rpc(endpoint, terminator_core::Request::ShutdownIfIdle);
        let Some(deadline) = Instant::now().checked_add(Duration::from_secs(5)) else {
            return;
        };
        while endpoint.socket().exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
        }
    }
}

fn clear_inherited_terminator(
    command: &mut Command,
    keys: impl IntoIterator<Item = std::ffi::OsString>,
) {
    for key in keys {
        if key.to_string_lossy().starts_with("TERMINATOR_") {
            command.env_remove(key);
        }
    }
}
#[cfg(test)]
mod isolation_tests;
