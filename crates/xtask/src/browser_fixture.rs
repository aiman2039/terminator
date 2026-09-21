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
    let running = stop.clone();
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
pub fn run() -> Result<()> {
    let browser=["chromium","chromium-browser","google-chrome"].into_iter().find_map(terminator_core::find_executable).context("Install a browser for the optional live fixture; linux-check --browser installs Chromium only inside its test container")?;
    let temp = tempfile::Builder::new()
        .prefix("term-browser-")
        .tempdir_in("/tmp")?;
    let (_server, port) = page()?;
    let profile = temp.path().join("profile");
    let logpath = temp.path().join("browser.log");
    let log = fs::File::create(&logpath)?;
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
    let mut browser = Process(command.spawn()?);
    let deadline = Instant::now() + Duration::from_secs(15);
    let discovery = profile.join("DevToolsActivePort");
    while !discovery.exists() {
        ensure!(
            browser.0.try_wait()?.is_none(),
            "Browser failed: {}",
            fs::read_to_string(&logpath)?
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
    let target = loop {
        if let Message::Text(text) = socket.read()? {
            let v: Value = serde_json::from_str(&text)?;
            if v["id"] == 1 {
                break v["result"]["targetId"]
                    .as_str()
                    .context("Target not created")?
                    .to_owned();
            }
        }
    };
    let endpoint = format!("ws://127.0.0.1:{debugging_port}/devtools/page/{target}");
    let ctl = |action: &str, args: &[&str]| -> Result<Value> {
        let mut command = Command::new(bin().join("terminator-hook"));
        command
            .args(["ctl", "browser", action, "--endpoint", &endpoint])
            .args(args)
            .env("TERMINATOR_DATA_DIR", temp.path());
        serde_json::from_slice(&output(command)?).map_err(Into::into)
    };
    ctl("navigate", &[&format!("http://127.0.0.1:{port}/")])?;
    let snapshot = ctl("snapshot", &[])?;
    ensure!(
        snapshot["title"] == "Terminator browser fixture"
            && snapshot["html"]
                .as_str()
                .is_some_and(|s| s.contains("id=\"name\"")),
        "DOM snapshot incorrect: {snapshot}"
    );
    ensure!(
        !snapshot["styles"]
            .as_array()
            .context("CSS snapshot missing")?
            .is_empty(),
        "Styles missing"
    );
    let value = "literal text '; window.bad=true;//";
    ctl("fill", &["#name", value])?;
    ctl("click", &["#go"])?;
    ensure!(
        ctl(
            "evaluate",
            &["document.querySelector('#result').textContent"]
        )? == value,
        "Fill/click did not preserve literal text"
    );
    ensure!(
        ctl("evaluate", &["typeof window.bad"])? == "undefined",
        "Input escaped into script"
    );
    let directory = artifacts().join("browser");
    fs::create_dir_all(&directory)?;
    let image = directory.join("browser.png");
    ctl("screenshot", &[image.to_str().unwrap()])?;
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
