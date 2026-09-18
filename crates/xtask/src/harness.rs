use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD as B64};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use terminator_core::{read_frame, write_frame};

pub fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .into()
}
pub fn bin() -> PathBuf {
    if let Some(path) = std::env::var_os("TERMINATOR_TEST_BIN_DIR") {
        return PathBuf::from(path);
    }

    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root().join("target"))
        .join("debug")
}
pub fn artifacts() -> PathBuf {
    bin().parent().unwrap().join("validation")
}
pub fn id(value: &Value) -> &str {
    value["id"].as_str().expect("fixture record ID")
}
pub fn sessions(state: &Value) -> &[Value] {
    state["sessions"].as_array().expect("session inventory")
}
pub fn session<'a>(state: &'a Value, sid: &str) -> &'a Value {
    sessions(state)
        .iter()
        .find(|s| id(s) == sid)
        .expect("session exists")
}
pub fn session_ids(value: &Value) -> Vec<String> {
    match value {
        Value::Object(map) if map.get("version").is_some() && map.contains_key("tabs") => {
            session_ids(&map["tabs"])
        }
        Value::Object(map) if map.get("Terminal").is_some_and(Value::is_string) => {
            vec![map["Terminal"].as_str().unwrap().into()]
        }
        Value::Object(map) => map
            .iter()
            .filter(|(k, _)| k.as_str() != "primary")
            .flat_map(|(_, v)| session_ids(v))
            .collect(),
        Value::Array(list) => list.iter().flat_map(session_ids).collect(),
        _ => vec![],
    }
}
pub fn output(mut command: Command) -> Result<Vec<u8>> {
    command.stdin(Stdio::null());
    Ok(terminator_core::run_command(
        command,
        terminator_core::CommandOptions {
            timeout: Duration::from_secs(20),
            stdout_limit: 8 * 1024 * 1024,
            ..Default::default()
        },
    )?
    .stdout)
}
pub fn git(cwd: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let mut c = Command::new("git");
    c.arg("-C").arg(cwd).args(args);
    output(c)
}
pub fn wait_child(child: &mut Child, timeout: Duration) -> Result<std::process::ExitStatus> {
    let end = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        ensure!(Instant::now() < end, "Child process timed out");
        thread::sleep(Duration::from_millis(25));
    }
}
pub struct Process(pub Child);
impl Drop for Process {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}
pub struct Harness {
    pub _temp: tempfile::TempDir,
    pub root: PathBuf,
    pub env: BTreeMap<String, String>,
    pub daemon: Option<Process>,
}
impl Harness {
    pub fn new() -> Result<Self> {
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
        env.insert("TERMINATOR_NO_NOTIFICATIONS".into(), "1".into());
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
                .map(PathBuf::from)
                .unwrap_or_else(|| bin().join(name))
        } else if name == "terminator-daemon" {
            self.env
                .get("TERMINATOR_FIXTURE_DAEMON")
                .map(PathBuf::from)
                .unwrap_or_else(|| bin().join(name))
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
        let end = Instant::now() + Duration::from_secs(10);
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
    ) -> Result<UnixStream> {
        let paths = terminator_core::generations::owner_for(
            &terminator_core::Paths::at(self.root.clone()),
            &serde_json::from_value(request.clone())?,
        )?;
        let mut stream = UnixStream::connect(paths.socket())?;
        stream.set_read_timeout(Some(Duration::from_secs(3)))?;
        stream.set_write_timeout(Some(Duration::from_secs(3)))?;
        let auth = paths.token()?;
        let mut envelope =
            json!({"version":1,"auth":token.unwrap_or(auth.trim()),"request":request});
        if let Some(hint) = hint {
            envelope["snapshot_hint"] = hint;
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
        Ok(self.rpc(json!("Snapshot"))?["State"].clone())
    }
    pub fn wait(&self, mut check: impl FnMut(&Value) -> bool, seconds: u64) -> Result<Value> {
        let end = Instant::now() + Duration::from_secs(seconds);
        loop {
            let state = self.state()?;
            if check(&state) {
                return Ok(state);
            }
            ensure!(Instant::now() < end, "State assertion timed out");
            thread::sleep(Duration::from_millis(30));
        }
    }
    pub fn project(&self, name: &str) -> Result<Value> {
        let path = self.root.join(name);
        fs::create_dir_all(&path)?;
        self.rpc(json!({"AddProject":{"path":path}}))?;
        self.state()?["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["path"] == path.to_string_lossy().as_ref())
            .cloned()
            .context("Created project missing")
    }
    pub fn shell(&self, project: &Value) -> Result<Value> {
        Ok(self.rpc(json!({"Create":{"project":id(project),"cwd":null,"file":null,"line":null,"editor":false}}))?["Created"].clone())
    }
    pub fn editor(&self, project: &Value, path: &Path) -> Result<Value> {
        Ok(self.rpc(json!({"Create":{"project":id(project),"cwd":project["path"],"file":path,"line":null,"editor":true}}))?["Created"].clone())
    }
    pub fn attach(&self, session: &Value) -> Result<UnixStream> {
        let mut stream = self.connect(
            json!({"Attach":{"session":id(session),"rows":24,"cols":80}}),
            None,
            None,
        )?;
        let _: Value = read_frame(&mut stream)?;
        Ok(stream)
    }
    pub fn write(&self, stream: &mut UnixStream, text: &str) -> Result<()> {
        write_frame(stream, &json!({"Input":{"data":B64.encode(text)}}))?;
        stream.flush()?;
        Ok(())
    }
    pub fn history(&self, sid: &str) -> Result<String> {
        Ok(self.rpc(json!({"History":{"session":sid}}))?["Text"]
            .as_str()
            .context("History text missing")?
            .into())
    }
    pub fn setup(&self) -> Result<()> {
        let mut settings = self.state()?["settings"].clone();
        settings["shell"] = json!("/bin/sh");
        self.rpc(json!({"Settings":settings}))?;
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
        self.rpc(json!("Shutdown"))?;
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
impl Drop for Harness {
    fn drop(&mut self) {
        if let Ok(state) = self.state() {
            for session in sessions(&state) {
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
            let paths = terminator_core::Paths::at(self.root.clone());
            if let Ok(catalog) = terminator_core::generations::Catalog::open(&paths)
                && let Ok(owners) = catalog.generations()
            {
                for owner in owners {
                    let endpoint = owner.paths();
                    let _ =
                        terminator_core::rpc(&endpoint, terminator_core::Request::ShutdownIfIdle);
                    let deadline = Instant::now() + Duration::from_secs(5);
                    while endpoint.socket().exists() && Instant::now() < deadline {
                        thread::sleep(Duration::from_millis(50));
                    }
                }
            } else {
                let _ = self.rpc(json!("Shutdown"));
            }
            if let Some(daemon) = &mut self.daemon {
                let _ = wait_child(&mut daemon.0, Duration::from_secs(5));
            }
        }
        self.daemon.take();
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
mod isolation_tests {
    use super::*;
    #[test]
    fn inherited_catalog_generation_and_session_routing_cannot_escape_fixture() {
        let mut command = Command::new("fixture");
        let names = [
            "TERMINATOR_CATALOG_DATA",
            "TERMINATOR_CATALOG_RUNTIME",
            "TERMINATOR_GENERATION",
            "TERMINATOR_DATA_DIR",
            "TERMINATOR_RUNTIME_DIR",
            "TERMINATOR_SESSION_ID",
            "TERMINATOR_SESSION_TOKEN",
        ];
        clear_inherited_terminator(&mut command, names.iter().map(std::ffi::OsString::from));
        command.env("TERMINATOR_DATA_DIR", "/isolated/fixture");
        for name in names {
            let value = command
                .get_envs()
                .find(|(key, _)| *key == std::ffi::OsStr::new(name))
                .unwrap()
                .1;
            if name == "TERMINATOR_DATA_DIR" {
                assert_eq!(value, Some(std::ffi::OsStr::new("/isolated/fixture")));
            } else {
                assert!(value.is_none(), "inherited route {name} was retained");
            }
        }
    }
}
