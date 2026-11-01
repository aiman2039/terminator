use super::super::CaptureInput;
use super::super::{
    Context, Duration, Harness, Options, Path, PathBuf, Result, Value, capture_when, ensure, fs,
    git, json, save_prefs, sessions, thread, waiting,
};
use super::init_repo;
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use terminator_core::{read_frame, transport, write_frame};

pub(in super::super) struct Proxy {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    pub rejected: Arc<AtomicUsize>,
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
pub(in super::super) fn proxy(root: &Path) -> Result<Proxy> {
    let dir = root.join("legacy");
    fs::create_dir(&dir)?;
    fs::copy(root.join("run/auth"), dir.join("auth"))?;
    let listener = transport::Listener::bind_ipc(&dir.join("daemon.sock"))?;
    listener.set_nonblocking(true)?;
    let target = root.join("run/daemon.sock");
    let stop = Arc::new(AtomicBool::new(false));
    let running = Arc::clone(&stop);
    let rejected = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&rejected);
    let thread = thread::spawn(move || {
        while !running.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut client, _)) => {
                    let target = target.clone();
                    let counter = Arc::clone(&counter);
                    thread::spawn(move || {
                        let _ = (|| -> Result<()> {
                            client.set_nonblocking(false)?;
                            client.set_read_timeout(Some(Duration::from_secs(5)))?;
                            let envelope: Value = read_frame(&mut client)?;
                            let request = envelope
                                .get("request")
                                .ok_or_else(|| anyhow::anyhow!("missing request"))?;
                            if request.get("CreateReview").is_some()
                                || request.get("TerminalNotify").is_some()
                            {
                                counter.fetch_add(1, Ordering::Relaxed);
                                return Ok(());
                            }
                            let mut upstream = transport::Stream::connect_ipc(&target)?;
                            write_frame(&mut upstream, &envelope)?;
                            if request == "Snapshot" {
                                let mut response: Value = read_frame(&mut upstream)?;
                                if let Some(state) =
                                    response.get_mut("State").and_then(Value::as_object_mut)
                                {
                                    state.remove("capabilities");
                                }
                                write_frame(&mut client, &response)?;
                                return Ok(());
                            }
                            let mut read_client = client.try_clone()?;
                            let mut write_upstream = upstream.try_clone()?;
                            thread::spawn(move || {
                                let _ = io::copy(&mut read_client, &mut write_upstream);
                                let _ = write_upstream.shutdown();
                            });
                            let _ = io::copy(&mut upstream, &mut client);
                            let _ = client.shutdown();
                            Ok(())
                        })();
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }
    });
    Ok(Proxy {
        stop,
        thread: Some(thread),
        rejected,
    })
}
pub fn legacy(o: &Options) -> Result<()> {
    let mut h = Harness::new()?;
    h.setup()?;
    let mut settings = h
        .state()?
        .get("settings")
        .cloned()
        .context("Missing settings")?;
    settings
        .as_object_mut()
        .context("Settings are not an object")?
        .insert("review_mode".into(), json!("neovim"));
    h.rpc(json!({"Settings":settings}))?;
    let p = h.project("legacy-diff")?;
    let root = PathBuf::from(
        p.get("path")
            .ok_or_else(|| anyhow::anyhow!("missing project path"))?
            .as_str()
            .unwrap(),
    );
    init_repo(&root)?;
    fs::write(root.join("review.rs"), "fn old() {}\n")?;
    git(&root, &["add", "."])?;
    git(&root, &["commit", "-qm", "base"])?;
    fs::write(root.join("review.rs"), "fn updated() {}\n")?;
    let shell = h.shell(&p)?;
    h.layout(&p, std::slice::from_ref(&shell))?;
    save_prefs(
        &h,
        &json!({"version":1,"tool":"Git","visible":true,"typography_migrated":true}),
    )?;
    let proxy = proxy(&h.root)?;
    h.env.insert(
        "TERMINATOR_RUNTIME_DIR".into(),
        h.root.join("legacy").to_string_lossy().into_owned(),
    );
    // The native fallback view draws its menu once the diff is shown.
    let logs = capture_when(
        CaptureInput {
            h: &h,
            opts: o,
            name: "legacy-diff",
            actions: waiting(json!([
                {"target":"git-file-review.rs","right_click":true},
                {"target":"Open changes","wait":{"retry_previous_ms":1500}},
                {"target":"legacy-diff-shown","check":true,"wait":{"present":["diff-menu"]}}
            ])),
            after: 15000,
        },
        |state| tab_count(state) == Some(2),
    )?;
    ensure!(
        !logs.contains("Fixture error") && proxy.rejected.load(Ordering::Relaxed) == 0,
        "Sent unsupported request or failed legacy diff: {logs}"
    );
    let state = h.state()?;
    ensure!(
        sessions(&state).len() == 1 && tab_count(&state) == Some(2),
        "Legacy diff allocated editor or missing tab"
    );
    h.assert_pids(&[shell])
}

/// Top-level tabs in the first project's saved layout.
fn tab_count(state: &Value) -> Option<usize> {
    state
        .get("projects")?
        .get(0)?
        .get("layout")?
        .get("tabs")?
        .as_array()
        .map(Vec::len)
}
