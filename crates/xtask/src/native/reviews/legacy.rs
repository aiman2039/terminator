use super::super::CaptureInput;
use super::super::{
    Context, Duration, Harness, Options, Path, PathBuf, Result, Value, ensure, fs, git, json,
    plain, save_prefs, sessions, thread,
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
    let logs = plain(CaptureInput {
        h: &h,
        opts: o,
        name: "legacy-diff",
        actions: json!([{"at_ms":1200,"target":"git-file-review.rs","right_click":true},{"at_ms":1700,"target":"Open changes"}]),
        after: 3500,
    })?;
    ensure!(
        !logs.contains("Fixture error") && proxy.rejected.load(Ordering::Relaxed) == 0,
        "Sent unsupported request or failed legacy diff: {logs}"
    );
    let state = h.state()?;
    ensure!(
        sessions(&state).len() == 1
            && state
                .get("projects")
                .and_then(|projects| projects.get(0))
                .and_then(|project| project.get("layout"))
                .and_then(|layout| layout.get("tabs"))
                .ok_or_else(|| anyhow::anyhow!("missing projects[0].layout.tabs"))?
                .as_array()
                .unwrap()
                .len()
                == 2,
        "Legacy diff allocated editor or missing tab"
    );
    h.assert_pids(&[shell])
}
