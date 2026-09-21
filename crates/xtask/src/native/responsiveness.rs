//! Isolated stalled-radio responsiveness and rapid-switch resource fixture.
use super::{
    Duration, Options, Path, Result, Value, capture, ensure, fs, id, json, output, setup, thread,
};
use std::{
    io::Read,
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use terminator_core::{Paths, ui_control};
struct RadioServer {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl RadioServer {
    fn new() -> Result<(Self, String)> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let url = format!("http://{}/radio", listener.local_addr()?);
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let thread = thread::spawn(move || {
            let mut connections = Vec::new();
            while !done.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
                        let mut bytes = [0; 4096];
                        let _ = stream.read(&mut bytes);
                        // Deliberately never send headers. Limit fixture-side descriptors too.
                        connections.push(stream);
                        if connections.len() > 8 {
                            connections.remove(0);
                        }
                    }
                    Err(_) => thread::sleep(Duration::from_millis(2)),
                }
            }
        });
        Ok((
            Self {
                stop,
                thread: Some(thread),
            },
            url,
        ))
    }
}
impl Drop for RadioServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.thread.take() {
            let _ = worker.join();
        }
    }
}
struct NvimServer {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl NvimServer {
    fn new(path: &Path) -> Result<Self> {
        let listener = std::os::unix::net::UnixListener::bind(path)?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let thread = thread::spawn(move || {
            let mut connections = Vec::new();
            while !done.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
                        let mut bytes = [0; 4096];
                        let _ = stream.read(&mut bytes);
                        connections.push(stream);
                        if connections.len() > 8 {
                            connections.remove(0);
                        }
                    }
                    Err(_) => thread::sleep(Duration::from_millis(2)),
                }
            }
        });
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}
impl Drop for NvimServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.thread.take() {
            let _ = worker.join();
        }
    }
}
fn snapshot(paths: &Paths) -> Result<Value> {
    ui_control::rpc(paths, ui_control::Request::Snapshot)
}
fn player(paths: &Paths, action: &str, url: Option<String>) -> Result<Value> {
    ui_control::rpc(
        paths,
        ui_control::Request::FixturePlayer {
            action: action.into(),
            url,
        },
    )
}
fn resources(pid: u32) -> Result<Value> {
    let mut command = std::process::Command::new("ps");
    command.args(["-o", "%cpu=", "-o", "rss=", "-p", &pid.to_string()]);
    let bytes = output(command)?;
    let fields: Vec<_> = std::str::from_utf8(&bytes)?.split_whitespace().collect();
    let cpu = fields.first().and_then(|s| s.parse::<f64>().ok());
    let rss_kib = fields.get(1).and_then(|s| s.parse::<u64>().ok());
    let threads = if cfg!(target_os = "macos") {
        let mut command = std::process::Command::new("ps");
        command.args(["-M", "-p", &pid.to_string()]);
        String::from_utf8_lossy(&output(command)?)
            .lines()
            .count()
            .saturating_sub(1)
    } else {
        fs::read_to_string(format!("/proc/{pid}/status"))?
            .lines()
            .find_map(|s| {
                s.strip_prefix("Threads:")
                    .and_then(|v| v.trim().parse().ok())
            })
            .unwrap_or(0)
    };
    let mut command = std::process::Command::new("lsof");
    command.args(["-a", "-p", &pid.to_string(), "-Ff"]);
    let descriptors = String::from_utf8_lossy(&output(command)?)
        .lines()
        .filter(|line| {
            line.strip_prefix('f')
                .is_some_and(|value| value.chars().next().is_some_and(|c| c.is_ascii_digit()))
        })
        .count();
    Ok(json!({"cpu_percent":cpu,"rss_kib":rss_kib,"threads":threads,"descriptors":descriptors}))
}
pub fn run(options: &Options) -> Result<()> {
    let baseline = std::env::var_os("TERMINATOR_TEST_BASELINE").is_some();
    let (mut h, _, sessions, _) = setup("responsiveness")?;
    h.env
        .insert("TERMINATOR_TEST_RESPONSIVENESS".into(), "1".into());
    let socket = Paths::at(h.root.clone()).runtime.join("stalled.nvim");
    let _nvim = NvimServer::new(&socket)?;
    h.env.insert(
        "TERMINATOR_TEST_STALL_NVIM".into(),
        socket.to_string_lossy().into_owned(),
    );
    let (_server, url) = RadioServer::new()?;
    let paths = Paths::at(h.root.clone());
    let mut report = Value::Null;
    let after = options.seconds.max(12) * 1000;
    capture(&h, options, "stalled-radio", json!([]), after, |child| {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            if snapshot(&paths)
                .is_ok_and(|s| s["selected_project"].is_string() || s["selected"].is_string())
            {
                break;
            }
            ensure!(Instant::now() < deadline, "GUI did not become ready");
            thread::sleep(Duration::from_millis(20));
        }
        let initial = snapshot(&paths)?;
        let resources_before = resources(child.id())?;
        let mut acknowledgments = Vec::new();
        let mut switches = Vec::new();
        let start = Instant::now();
        for index in 0..100 {
            let at = Instant::now();
            player(&paths, "play", Some(format!("{url}/{index}")))?;
            switches.push(at.elapsed().as_secs_f64() * 1000.0);
            let at = Instant::now();
            ui_control::rpc(
                &paths,
                ui_control::Request::Focus {
                    session: id(&sessions[index % sessions.len()]).into(),
                },
            )?;
            acknowledgments.push(at.elapsed().as_secs_f64() * 1000.0);
            let state = snapshot(&paths)?;
            ensure!(
                baseline || state["services"]["active"].as_u64().unwrap_or(99) <= 32,
                "Active operation limit exceeded"
            );
            ensure!(
                baseline || state["services"]["queued"].as_u64().unwrap_or(999) <= 128,
                "Admission limit exceeded"
            );
            ensure!(
                baseline || state["player"]["engine"]["decoders"].as_u64().unwrap_or(99) <= 1,
                "Overlapping decoder pipelines"
            );
            ensure!(
                baseline
                    || state["player"]["engine"]["connections"]
                        .as_u64()
                        .unwrap_or(99)
                        <= 1,
                "Overlapping radio connections"
            );
        }
        // --seconds 1800 is the long resource soak; short runs cover rapid switching.
        let soak = Duration::from_secs(options.seconds.saturating_sub(5));
        let mut samples = Vec::new();
        let mut resource_samples = Vec::new();
        let mut resource_at = Instant::now();
        let mut switch_at = Instant::now();
        let mut soak_switches = 0;
        while start.elapsed() < soak {
            if switch_at.elapsed() >= Duration::from_secs(1) {
                soak_switches += 1;
                player(&paths, "play", Some(format!("{url}/soak/{soak_switches}")))?;
                switch_at = Instant::now();
            }
            let state = snapshot(&paths)?;
            ensure!(
                baseline
                    || (state["services"]["active"].as_u64().unwrap_or(99) <= 32
                        && state["services"]["queued"].as_u64().unwrap_or(999) <= 128),
                "Task budget exceeded during soak"
            );
            samples.push(state["services"].clone());
            if resource_at.elapsed() >= Duration::from_secs(5) {
                let sample = resources(child.id())?;
                println!(
                    "{}",
                    json!({"soak_seconds":start.elapsed().as_secs(),"resources":sample})
                );
                resource_samples.push(sample);
                resource_at = Instant::now();
            }
            thread::sleep(Duration::from_millis(250));
        }
        let at = Instant::now();
        player(&paths, "stop", None)?;
        let stop_ms = at.elapsed().as_secs_f64() * 1000.0;
        let deadline = Instant::now() + Duration::from_secs(2);
        let final_state = loop {
            let state = snapshot(&paths)?;
            if baseline
                || (state["player"]["engine"]["decoders"] == 0
                    && state["player"]["engine"]["connections"] == 0)
            {
                break state;
            }
            ensure!(
                Instant::now() < deadline,
                "Stopped radio pipeline did not clean up"
            );
            thread::sleep(Duration::from_millis(10));
        };
        acknowledgments.sort_by(f64::total_cmp);
        switches.sort_by(f64::total_cmp);
        let p95 = acknowledgments[94];
        let switch_p95 = switches[94];
        let peak = final_state["services"]["ui_processing_peak_ms"]
            .as_f64()
            .unwrap_or(f64::INFINITY);
        report = json!({"pid":child.id(),"baseline":baseline,"pipeline_cleanup_verified":!baseline,"switches":100,"soak_switches":soak_switches,"ack_p95_ms":p95,"switch_p95_ms":switch_p95,"stop_ms":stop_ms,
            "ui_processing_peak_ms":peak,"elapsed_seconds":start.elapsed().as_secs_f64(),"initial":initial["services"],"final":final_state["services"],"samples":samples,"resources_before":resources_before,"resources_after":resources(child.id())?,"resource_samples":resource_samples});
        fs::create_dir_all(&options.output)?;
        fs::write(
            options.output.join("responsiveness.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
        ensure!(
            baseline || (p95 < 100.0 && switch_p95 < 100.0 && stop_ms < 100.0 && peak < 250.0),
            "Responsiveness limit exceeded: ack p95={p95:.1}ms, switch p95={switch_p95:.1}ms, Stop={stop_ms:.1}ms, processing peak={peak:.1}ms"
        );
        Ok(())
    })?;
    h.assert_pids(&sessions)?;
    println!(
        "{}",
        json!({"responsiveness":if baseline {"baseline-recorded"} else {"passed"},"report":options.output.join("responsiveness.json"),"ack_p95_ms":report["ack_p95_ms"]})
    );
    Ok(())
}
