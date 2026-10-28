//! Opt-in live SSH checks against the Docker fixture in `fixtures/ssh-project`.
//! Drives a real isolated daemon over IPC. Skips (exit 0) without Docker or
//! the fixture unless `--require` is given.
mod steps;

use crate::harness::{Harness, root};
use anyhow::{Context, Result, anyhow, bail, ensure};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};
use steps::steps;
use terminator_core::{
    Request, Response,
    remote::{Operation, RemoteProject, Reply, TransferStatus},
    ssh::{SshError, SshFailure, SshOptions},
};

const MAIN: &str = "ssh-project";
const PW: &str = "ssh-project-pw";
const ROOT: &str = "/workspace/project";
const BIG_BYTES: usize = 50 * 1024 * 1024;

/// Why the target cannot run here, or `None` when it can.
pub(crate) fn unavailable() -> Option<String> {
    if !cfg!(unix) {
        return Some("OpenSSH multiplexing needs Unix".into());
    }
    let script = root().join("scripts/ssh-project-fixture.sh");
    let compose = root().join("fixtures/ssh-project/compose.yaml");
    if !script.is_file() || !compose.is_file() {
        return Some("fixtures/ssh-project or scripts/ssh-project-fixture.sh is missing".into());
    }
    for tool in ["ssh", "ssh-keygen", "ssh-keyscan"] {
        if !Command::new(tool)
            .arg("-?")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
        {
            return Some(format!("{tool} is not on PATH"));
        }
    }
    match Command::new("docker")
        .args(["info", "--format", "{{.ServerVersion}}"])
        .stdin(Stdio::null())
        .output()
    {
        Ok(out) if out.status.success() => None,
        Ok(out) => Some(format!(
            "Docker daemon is not reachable: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
        Err(error) => Some(format!("docker is not on PATH: {error}")),
    }
}

pub fn run(require: bool) -> Result<()> {
    if let Some(reason) = unavailable() {
        ensure!(!require, "ssh integration unavailable: {reason}");
        println!("Skipping ssh integration: {reason}. Pass --require to fail instead.");
        return Ok(());
    }
    let mut fixture = Fixture::start()?;
    let result = steps(&mut fixture);
    let cleanup = fixture.cleanup();
    match (result, cleanup) {
        (Ok(report), Ok(())) => {
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        (Err(error), cleanup) => {
            if let Err(cleanup) = cleanup {
                eprintln!("ssh fixture cleanup also failed: {cleanup:#}");
            }
            Err(error)
        }
        (Ok(_), Err(error)) => Err(error.context("ssh fixture cleanup failed")),
    }
}

/// The Docker fixture, its key directory and an isolated daemon. Cleanup
/// runs on every exit path, including panics, through `Drop`.
pub(crate) struct Fixture {
    pub temp: tempfile::TempDir,
    pub project: String,
    pub port: u16,
    pub pw_port: u16,
    pub harness: Option<Harness>,
    cleaned: bool,
}
impl Fixture {
    pub(crate) fn start() -> Result<Self> {
        let temp = tempfile::Builder::new()
            .prefix("term-ssh-")
            .tempdir_in("/tmp")?;
        let mut fixture = Self {
            project: format!("terminator-ssh-xtask-{}", std::process::id()),
            port: free_port()?,
            pw_port: free_port()?,
            temp,
            harness: None,
            cleaned: false,
        };
        fs::create_dir_all(fixture.keys())?;
        println!(
            "Starting SSH fixture {} on ports {} and {}",
            fixture.project, fixture.port, fixture.pw_port
        );
        fixture.script(&["start"]).context("Fixture start failed")?;
        fixture.wait_listening(fixture.port)?;
        fixture.wait_listening(fixture.pw_port)?;
        fixture.harness = Some(Harness::new()?);
        Ok(fixture)
    }
    pub(crate) fn keys(&self) -> PathBuf {
        self.temp.path().join("keys")
    }
    pub(crate) fn h(&self) -> &Harness {
        self.harness.as_ref().expect("daemon harness")
    }
    pub(crate) fn ssh(&self, port: u16, known_hosts: &Path) -> SshOptions {
        SshOptions {
            host: "127.0.0.1".into(),
            port: Some(port),
            username: "developer".into(),
            identity_file: Some(self.keys().join("id_ed25519")),
            known_hosts_file: Some(known_hosts.to_owned()),
        }
    }
    fn envs(&self, command: &mut Command) {
        command
            .env("SSH_PROJECT_FIXTURE_KEY_DIR", self.keys())
            .env("SSH_PROJECT_FIXTURE_PROJECT", &self.project)
            .env("SSH_PROJECT_FIXTURE_PORT", self.port.to_string())
            .env("SSH_PROJECT_FIXTURE_PW_PORT", self.pw_port.to_string());
    }
    pub(crate) fn script(&self, args: &[&str]) -> Result<String> {
        let mut command = Command::new(root().join("scripts/ssh-project-fixture.sh"));
        command.args(args).stdin(Stdio::null());
        self.envs(&mut command);
        checked(command.output()?, "ssh-project-fixture.sh")
    }
    fn compose(&self, args: &[&str]) -> Result<String> {
        let mut command = Command::new("docker");
        command
            .args(["compose", "-p", &self.project, "-f"])
            .arg(root().join("fixtures/ssh-project/compose.yaml"))
            .args(args)
            .stdin(Stdio::null());
        self.envs(&mut command);
        checked(command.output()?, "docker compose")
    }
    pub(crate) fn exec(&self, service: &str, script: &str) -> Result<String> {
        self.compose(&["exec", "-T", service, "sh", "-c", script])
    }
    /// sshd logs (`-e`), one line per event, for this service.
    fn logs(&self, service: &str) -> Result<String> {
        let mut command = Command::new("docker");
        command
            .args(["compose", "-p", &self.project, "-f"])
            .arg(root().join("fixtures/ssh-project/compose.yaml"))
            .args(["logs", "--no-color", "--no-log-prefix", service])
            .stdin(Stdio::null());
        self.envs(&mut command);
        let out = command.output()?;
        ensure!(out.status.success(), "docker compose logs failed");
        // sshd writes to stderr; compose may split streams.
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        Ok(text)
    }
    pub(crate) fn connections(&self, service: &str) -> Result<usize> {
        Ok(self
            .logs(service)?
            .lines()
            .filter(|l| l.contains("Connection from "))
            .count())
    }
    /// sshd may answer health checks before it accepts TCP.
    fn wait_listening(&self, port: u16) -> Result<()> {
        let end = Instant::now()
            .checked_add(Duration::from_secs(30))
            .context("deadline overflow")?;
        loop {
            if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut banner = [0u8; 4];
                if stream.read_exact(&mut banner).is_ok() && &banner == b"SSH-" {
                    return Ok(());
                }
            }
            ensure!(Instant::now() < end, "sshd on port {port} never answered");
            thread::sleep(Duration::from_millis(200));
        }
    }
    /// Exits every multiplexing master under the daemon's data directory,
    /// stops the daemon, then removes containers and volumes.
    pub(crate) fn cleanup(&mut self) -> Result<()> {
        if self.cleaned {
            return Ok(());
        }
        self.cleaned = true;
        let mut errors = Vec::new();
        if let Some(harness) = &self.harness {
            let _ = harness.rpc(serde_json::json!("Shutdown"));
            for socket in control_sockets(&harness.root) {
                let _ = terminator_core::ssh::exit_control(&socket)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            // Anything still holding a socket in this directory, e.g. a
            // master that ignored `-O exit`.
            let _ = Command::new("pkill")
                .args(["-f", &format!("ControlPath={}", harness.root.display())])
                .status();
        }
        self.harness = None;
        if let Err(error) = self.script(&["down"]) {
            errors.push(format!("{error:#}"));
        }
        ensure!(errors.is_empty(), "{}", errors.join("; "));
        Ok(())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            eprintln!("ssh fixture cleanup failed: {error:#}");
        }
    }
}

fn checked(out: Output, what: &str) -> Result<String> {
    ensure!(
        out.status.success(),
        "{what} failed ({}): {}{}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
fn free_port() -> Result<u16> {
    Ok(TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port())
}
/// `ssh-<8 hex>` sockets anywhere under `dir`.
fn control_sockets(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            found.extend(control_sockets(&entry.path()));
            continue;
        }
        #[cfg(unix)]
        if std::os::unix::fs::FileTypeExt::is_socket(&kind)
            && entry
                .file_name()
                .to_str()
                .and_then(|n| n.strip_prefix("ssh-"))
                .is_some_and(|hex| hex.len() == 8 && hex.chars().all(|c| c.is_ascii_hexdigit()))
        {
            found.push(entry.path());
        }
    }
    found
}

pub(crate) fn rpc(h: &Harness, request: Request) -> Result<Response> {
    terminator_core::rpc(&terminator_core::Paths::at(h.root.clone()), request)
}
fn ssh_failure(result: Result<Response>) -> Result<SshFailure> {
    match result {
        Ok(response) => bail!("Expected an SSH failure, got {response:?}"),
        Err(error) => error
            .downcast_ref::<SshError>()
            .map(|e| e.failure.clone())
            .ok_or_else(|| anyhow!("Unclassified failure: {error:#}")),
    }
}
pub(crate) fn remote_project(name: &str, ssh: SshOptions) -> RemoteProject {
    RemoteProject {
        id: String::new(),
        name: name.into(),
        ssh,
        root: ROOT.into(),
        layout: serde_json::Value::Null,
        group: None,
        forwards: Vec::new(),
    }
}
fn probe(h: &Harness, project: &RemoteProject) -> Result<Response> {
    rpc(
        h,
        Request::ProbeRemoteProject {
            project: project.clone(),
        },
    )
}
pub(crate) fn add(h: &Harness, project: &RemoteProject) -> Result<String> {
    rpc(
        h,
        Request::AddRemoteProject {
            project: project.clone(),
        },
    )?;
    let Response::State(state) = rpc(h, Request::Snapshot)? else {
        bail!("Snapshot did not return state");
    };
    state
        .remote_projects
        .iter()
        .find(|p| p.ssh == project.ssh && p.root == project.root)
        .map(|p| p.id.clone())
        .context("Added SSH project missing from state")
}
fn operation(h: &Harness, project: &str, operation: Operation) -> Result<Reply> {
    match rpc(
        h,
        Request::RemoteProject {
            project: project.into(),
            operation,
        },
    )? {
        Response::RemoteProject(reply) => Ok(reply),
        other => bail!("Unexpected reply {other:?}"),
    }
}
/// Scans, checks the fingerprint against the container's own key, trusts.
pub(crate) fn trust(fixture: &Fixture, service: &str, ssh: &SshOptions) -> Result<String> {
    let h = fixture.h();
    let Response::HostKeys(scan) = rpc(h, Request::ScanHostKey { ssh: ssh.clone() })? else {
        bail!("ScanHostKey did not return keys");
    };
    let port = ssh.port.context("fixture port")?;
    ensure!(
        scan.host == format!("[127.0.0.1]:{port}"),
        "Unexpected known_hosts host {}",
        scan.host
    );
    let expected = fixture.exec(
        service,
        "ssh-keygen -l -E sha256 -f /var/lib/ssh-host-keys/ssh_host_ed25519_key.pub",
    )?;
    let expected = expected
        .split_whitespace()
        .nth(1)
        .context("container fingerprint")?
        .to_owned();
    ensure!(
        scan.keys.iter().any(|k| k.fingerprint == expected),
        "Scanned fingerprints {:?} do not include the container's {expected}",
        scan.keys
    );
    rpc(
        h,
        Request::TrustHostKey {
            ssh: ssh.clone(),
            keys: scan.keys,
        },
    )?;
    Ok(expected)
}
fn wait_transfer(h: &Harness, project: &str, id: &str) -> Result<TransferStatus> {
    let end = Instant::now()
        .checked_add(Duration::from_mins(5))
        .context("deadline overflow")?;
    loop {
        let Reply::Transfer(status) =
            operation(h, project, Operation::TransferStatus { id: id.into() })?
        else {
            bail!("TransferStatus did not return a transfer");
        };
        if status.done {
            ensure!(status.error.is_none(), "Transfer failed: {status:?}");
            return Ok(status);
        }
        ensure!(Instant::now() < end, "Transfer timed out: {status:?}");
        thread::sleep(Duration::from_millis(200));
    }
}
fn sha256_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(buffer.get(..read).unwrap_or_default());
    }
    Ok(hex(&hasher.finalize()))
}
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}
/// Deterministic incompressible bytes, so SSH compression cannot hide size.
fn write_big(path: &Path) -> Result<()> {
    let mut file = fs::File::create(path)?;
    let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut chunk = vec![0u8; 1 << 20];
    for _ in 0..BIG_BYTES.checked_div(chunk.len()).unwrap_or(0) {
        for word in chunk.chunks_exact_mut(8) {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            word.copy_from_slice(&state.to_le_bytes());
        }
        file.write_all(&chunk)?;
    }
    file.sync_all()?;
    Ok(())
}
fn http_get(port: u16, path: &str) -> Result<String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    write!(
        stream,
        "GET {path} HTTP/1.0\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    Ok(response)
}
fn forwards(response: Response) -> Result<terminator_core::remote::ForwardStatus> {
    match response {
        Response::Forwards(status) => Ok(status),
        other => bail!("Expected forward status, got {other:?}"),
    }
}
