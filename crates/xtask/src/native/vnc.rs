//! VNC against a real TigerVNC server in Docker (`fixtures/vnc-server`):
//! the daemon connects and decodes frames and the cursor shape, the GUI
//! shows the session as Detached, reattaches it in a tab, and finally
//! disconnects it from the sidebar. Skips when Docker is unavailable.
use super::CaptureInput;
use super::{Context, Harness, Result, Value, capture, ensure, id, json, root, save_prefs};
use std::{
    io::Read,
    net::{TcpListener, TcpStream},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const IMAGE: &str = "terminator-vnc-fixture";
const PASSWORD: &str = "terminator-vnc-fixture";
const LABEL: &str = "vnc-fixture";

/// Why the case cannot run here, or `None` when it can.
fn unavailable() -> Option<String> {
    if !root().join("fixtures/vnc-server/Dockerfile").is_file() {
        return Some("fixtures/vnc-server/Dockerfile is missing".into());
    }
    match Command::new("docker")
        .args(["info", "--format", "{{.ServerVersion}}"])
        .stdin(Stdio::null())
        .output()
    {
        Ok(out) if out.status.success() => None,
        Ok(_) => Some("Docker daemon is not reachable".into()),
        Err(error) => Some(format!("docker is not on PATH: {error}")),
    }
}

/// One Xvnc container on a loopback port; removed on drop.
struct Server {
    name: String,
    port: u16,
}

impl Server {
    fn start() -> Result<Self> {
        docker(&["build", "-q", "-t", IMAGE])
            .arg(root().join("fixtures/vnc-server"))
            .status()
            .context("docker build")?
            .success()
            .then_some(())
            .context("Building the VNC fixture image failed")?;
        let port = TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port();
        let server = Self {
            name: format!("terminator-vnc-xtask-{}", std::process::id()),
            port,
        };
        let publish = format!("127.0.0.1:{port}:5901");
        let started = docker(&[
            "run",
            "-d",
            "--rm",
            "--name",
            &server.name,
            "-p",
            &publish,
            IMAGE,
        ])
        .stdout(Stdio::null())
        .status()?;
        ensure!(
            started.success(),
            "Starting the VNC fixture container failed"
        );
        server.wait_banner()?;
        Ok(server)
    }

    /// Xvnc answers with `RFB 003.008` once it accepts clients.
    fn wait_banner(&self) -> Result<()> {
        let end = Instant::now()
            .checked_add(Duration::from_secs(30))
            .context("deadline overflow")?;
        loop {
            if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", self.port)) {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut banner = [0u8; 4];
                if stream.read_exact(&mut banner).is_ok() && &banner == b"RFB " {
                    // Let xsetroot paint the background and set the cursor.
                    thread::sleep(Duration::from_secs(1));
                    return Ok(());
                }
            }
            ensure!(Instant::now() < end, "Xvnc never answered on {}", self.port);
            thread::sleep(Duration::from_millis(200));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = docker(&["rm", "-f", &self.name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn docker(args: &[&str]) -> Command {
    let mut command = Command::new("docker");
    command.args(args).stdin(Stdio::null());
    command
}

fn vnc_sessions(state: &Value) -> Vec<Value> {
    state
        .get("vnc_sessions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn connected(state: &Value) -> bool {
    vnc_sessions(state)
        .iter()
        .any(|s| field(s, "label") == LABEL && field(s, "status") == "connected")
}

pub fn run(o: &super::Options) -> Result<()> {
    if let Some(reason) = unavailable() {
        println!("Skipping vnc: {reason}");
        return Ok(());
    }
    let server = Server::start()?;
    let h = Harness::new()?;
    h.setup()?;
    let project = h.project("vnc")?;
    h.rpc(json!({"SelectProject":{"project":id(&project)}}))?;
    h.rpc(json!({"VncConnect":{
        "project":id(&project),"host":"127.0.0.1","port":server.port,
        "password":PASSWORD,"shared":true,"view_only":false,"label":LABEL
    }}))?;
    let state = h.wait(connected, 20)?;
    let sid = vnc_sessions(&state)
        .iter()
        .find(|s| field(s, "label") == LABEL)
        .and_then(|s| field(s, "id").as_str().map(str::to_owned))
        .context("connected VNC session has no id")?;
    daemon_frame(&h, &sid)?;
    reattach(&h, o, &sid)?;
    disconnect(&h, o)
}

/// The daemon decodes a full frame of the server's screen and receives
/// the cursor shape, so the viewer can draw it locally.
fn daemon_frame(h: &Harness, sid: &str) -> Result<()> {
    let end = Instant::now()
        .checked_add(Duration::from_secs(15))
        .context("deadline overflow")?;
    loop {
        let reply = h.rpc(json!({"VncFrame":{"session":sid,"since":0,"regions":false}}))?;
        let frame = reply.get("VncFrame").cloned().unwrap_or_default();
        let pixels = field(&frame, "width").as_u64().unwrap_or(0) > 0
            && field(&frame, "zlib_rgba")
                .as_str()
                .is_some_and(|p| !p.is_empty());
        if pixels && !field(&frame, "cursor").is_null() {
            return Ok(());
        }
        ensure!(
            Instant::now() < end,
            "No frame with pixels and a cursor shape: {}",
            json!({"width":field(&frame, "width"),"cursor":field(&frame, "cursor").is_object(),"status":field(&frame, "status")})
        );
        thread::sleep(Duration::from_millis(200));
    }
}

/// With no tab the sidebar shows the session as Detached; clicking it
/// opens a tab on the live session. Quitting the GUI leaves it connected.
fn reattach(h: &Harness, o: &super::Options, sid: &str) -> Result<()> {
    save_prefs(h, &json!({"version":1,"tool":"vnc","visible":true}))?;
    let logs = capture(
        CaptureInput {
            h,
            opts: o,
            name: "vnc-reattach",
            actions: json!([
                {"at_ms":1500,"target":format!("vnc-detached:{LABEL}"),"hover":true},
                {"at_ms":2000,"target":format!("vnc-session:{LABEL}")}
            ]),
            after: 5000,
        },
        |_| Ok(()),
    )?;
    let state = h.wait(connected, 10).context(logs)?;
    let layout = state
        .get("projects")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|p| field(p, "layout").to_string())
        .collect::<String>();
    ensure!(
        layout.contains(sid),
        "Reattach did not open a tab on the live session"
    );
    Ok(())
}

/// The sidebar Disconnect ends the session in the daemon.
fn disconnect(h: &Harness, o: &super::Options) -> Result<()> {
    let logs = capture(
        CaptureInput {
            h,
            opts: o,
            name: "vnc-disconnect",
            actions: json!([{"at_ms":2000,"target":format!("vnc-disconnect:{LABEL}")}]),
            after: 3500,
        },
        |_| Ok(()),
    )?;
    h.wait(
        |state| {
            !vnc_sessions(state)
                .iter()
                .any(|s| field(s, "label") == LABEL)
        },
        10,
    )
    .context(logs)?;
    Ok(())
}

/// `value[key]` without indexing: missing keys read as null.
fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or(&Value::Null)
}
