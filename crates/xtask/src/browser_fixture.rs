//! Live CDP proof against a fresh headless browser profile and a local HTTP page.
use crate::harness::{Process, artifacts, bin, output};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use tungstenite::Message;
struct Server {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn page() -> Result<(Server, u16)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    listener.set_nonblocking(true)?;
    let stop = Arc::new(AtomicBool::new(false));
    let running = Arc::clone(&stop);
    let thread = thread::spawn(move || {
        while !running.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                    let mut request = [0; 8192];
                    let _ = stream.read(&mut request);
                    let body = r#"<!doctype html><title>Terminator browser fixture</title><style>body{background:rgb(230,240,250);font:20px sans-serif;padding:30px}input,button{font:inherit}</style><input id="name"><button id="go" onclick="document.querySelector('#result').textContent=document.querySelector('#name').value">Apply</button><p id="result">Ready</p>"#;
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }
    });
    Ok((
        Server {
            stop,
            thread: Some(thread),
        },
        port,
    ))
}
/// `terminator-hook ctl browser` bound to one page endpoint.
struct BrowserCtl<'a> {
    endpoint: String,
    data_dir: &'a std::path::Path,
}
impl BrowserCtl<'_> {
    fn call(&self, action: &str, args: &[&str]) -> Result<Value> {
        let mut command = Command::new(bin().join("terminator-hook"));
        command
            .args(["ctl", "browser", action, "--endpoint", &self.endpoint])
            .args(args)
            .env("TERMINATOR_DATA_DIR", self.data_dir);
        serde_json::from_slice(&output(command)?).map_err(Into::into)
    }
}
pub fn run() -> Result<()> {
    let browser=["chromium","chromium-browser","google-chrome"].into_iter().find_map(terminator_core::find_executable).context("Install a browser for the optional live fixture; linux-check --browser installs Chromium only inside its test container")?;
    let temp = tempfile::Builder::new()
        .prefix("term-browser-")
        .tempdir_in("/tmp")?;
    let (_server, port) = page()?;
    let profile = temp.path().join("profile");
    let logpath = temp.path().join("browser.log");
    let log = fs::File::create(&logpath)?;
    let mut browser = launch(browser, &profile, log)?;
    let (debugging_port, browser_path) = wait_endpoint(&mut browser, &profile, &logpath)?;
    let target = create_target(debugging_port, &browser_path)?;
    let ctl = BrowserCtl {
        endpoint: format!("ws://127.0.0.1:{debugging_port}/devtools/page/{target}"),
        data_dir: temp.path(),
    };
    ctl.call("navigate", &[&format!("http://127.0.0.1:{port}/")])?;
    check_snapshot(&ctl.call("snapshot", &[])?)?;
    fill_and_click(&ctl)?;
    let directory = artifacts().join("browser");
    fs::create_dir_all(&directory)?;
    let image = directory.join("browser.png");
    ctl.call("screenshot", &[image.to_str().unwrap()])?;
    let image = image::open(image)?;
    ensure!(
        image.width() > 100 && image.height() > 100,
        "Invalid browser screenshot"
    );
    println!(
        "{}",
        json!({"live_external_browser":true,"navigation":true,"html_and_css_snapshot":true,"literal_fill_and_click":true,"screenshot":true})
    );
    Ok(())
}

fn launch(
    browser: std::path::PathBuf,
    profile: &std::path::Path,
    log: fs::File,
) -> Result<Process> {
    let mut command = Command::new(browser);
    command
        .args([
            "--headless=new",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-background-networking",
            "--remote-debugging-port=0",
        ])
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg("about:blank")
        .stdout(Stdio::null())
        .stderr(log);
    if std::path::Path::new("/.dockerenv").exists() {
        command.arg("--no-sandbox");
    }
    Ok(Process(command.spawn()?))
}

/// Wait for the debugging port file; returns the port and browser endpoint path.
fn wait_endpoint(
    browser: &mut Process,
    profile: &std::path::Path,
    logpath: &std::path::Path,
) -> Result<(u16, String)> {
    let child = &mut browser.0;
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(15))
        .ok_or_else(|| anyhow::anyhow!("browser deadline overflow"))?;
    let discovery = profile.join("DevToolsActivePort");
    while !discovery.exists() {
        ensure!(
            child.try_wait()?.is_none(),
            "Browser failed: {}",
            fs::read_to_string(logpath)?
        );
        ensure!(
            Instant::now() < deadline,
            "Browser debugging endpoint timed out"
        );
        thread::sleep(Duration::from_millis(50));
    }
    let data = fs::read_to_string(discovery)?;
    let mut lines = data.lines();
    let debugging_port = lines.next().context("Port missing")?.parse::<u16>()?;
    let browser_path = lines.next().context("Browser endpoint missing")?;
    Ok((debugging_port, browser_path.to_owned()))
}

fn create_target(debugging_port: u16, browser_path: &str) -> Result<String> {
    let stream = TcpStream::connect(("127.0.0.1", debugging_port))?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let (mut socket, _) = tungstenite::client(
        format!("ws://127.0.0.1:{debugging_port}{browser_path}"),
        stream,
    )?;
    socket.send(Message::Text(
        json!({"id":1,"method":"Target.createTarget","params":{"url":"about:blank"}})
            .to_string()
            .into(),
    ))?;
    loop {
        if let Message::Text(text) = socket.read()? {
            let v: Value = serde_json::from_str(&text)?;
            if v.get("id").is_some_and(|id| *id == 1) {
                return Ok(v
                    .get("result")
                    .and_then(|result| result.get("targetId"))
                    .and_then(Value::as_str)
                    .context("Target not created")?
                    .to_owned());
            }
        }
    }
}

fn check_snapshot(snapshot: &Value) -> Result<()> {
    ensure!(
        snapshot
            .get("title")
            .is_some_and(|title| title == "Terminator browser fixture")
            && snapshot
                .get("html")
                .and_then(Value::as_str)
                .is_some_and(|s| s.contains("id=\"name\"")),
        "DOM snapshot incorrect: {snapshot}"
    );
    ensure!(
        !snapshot
            .get("styles")
            .and_then(Value::as_array)
            .context("CSS snapshot missing")?
            .is_empty(),
        "Styles missing"
    );
    Ok(())
}

fn fill_and_click(ctl: &BrowserCtl<'_>) -> Result<()> {
    let value = "literal text '; window.bad=true;//";
    ctl.call("fill", &["#name", value])?;
    ctl.call("click", &["#go"])?;
    ensure!(
        ctl.call(
            "evaluate",
            &["document.querySelector('#result').textContent"]
        )? == value,
        "Fill/click did not preserve literal text"
    );
    ensure!(
        ctl.call("evaluate", &["typeof window.bad"])? == "undefined",
        "Input escaped into script"
    );
    Ok(())
}
