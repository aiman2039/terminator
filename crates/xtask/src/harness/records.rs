use anyhow::{Result, anyhow, ensure};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

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
        .map_or_else(|| root().join("target"), PathBuf::from)
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
/// True when the daemon still tracks this session. Ended shells and editors
/// without an agent resume command are pruned from state entirely.
pub fn session_present(state: &Value, sid: &str) -> bool {
    sessions(state).iter().any(|s| id(s) == sid)
}
/// Closed sessions may remain as ended history or be pruned from inventory.
pub fn session_closed(state: &Value, sid: &str) -> bool {
    sessions(state)
        .iter()
        .find(|session| id(session) == sid)
        .is_none_or(|session| {
            session
                .get("lifecycle")
                .is_some_and(|value| value == "ended")
        })
}
pub(super) fn session_summary(state: &Value) -> String {
    let Some(list) = state["sessions"].as_array() else {
        return "no session inventory".into();
    };
    if list.is_empty() {
        return "no sessions".into();
    }
    list.iter()
        .map(|session| {
            format!(
                "{}:{}",
                session["id"].as_str().unwrap_or("?"),
                session["lifecycle"].as_str().unwrap_or("unknown")
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
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
    let end = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| anyhow!("child deadline overflow"))?;
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
