use super::{
    Context, Duration, Harness, Options, Path, PathBuf, Result, Value, ensure, fs, git, id, json,
    output, plain, save_prefs, session, session_ids, sessions, thread,
};
use std::{
    io,
    net::Shutdown,
    os::unix::net::{UnixListener, UnixStream},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use terminator_core::{read_frame, write_frame};
fn expression(h: &Harness, s: &Value, expression: &str) -> Result<String> {
    let mut cmd = std::process::Command::new("nvim");
    cmd.arg("--server")
        .arg(h.root.join("run").join(format!("{}.nvim", &id(s)[..8])))
        .args(["--remote-expr", expression]);
    Ok(String::from_utf8(output(cmd)?)?.trim().into())
}
fn inspect(h: &Harness, s: &Value) -> Result<Value> {
    serde_json::from_str(&expression(h,s,"json_encode({'user_init':get(g:,'terminator_user_init',0),'messages':execute('messages'),'buffers':map(filter(getbufinfo(), 'v:val.loaded && !empty(v:val.name)'),{_,b -> {'name':b.name,'modifiable':getbufvar(b.bufnr,'&modifiable'),'readonly':getbufvar(b.bufnr,'&readonly'),'lines':getbufline(b.bufnr,1,'$')}})})")?).map_err(Into::into)
}
fn verify(h: &Harness, s: &Value, left: Value, right: Value) -> Result<()> {
    h.wait(
        |_| {
            expression(h, s, "get(g:, 'terminator_review_ready', 0)")
                .is_ok_and(|s| matches!(s.as_str(), "v:true" | "true" | "1"))
        },
        8,
    )?;
    thread::sleep(Duration::from_millis(150));
    let data = inspect(h, s)?;
    ensure!(
        data["user_init"] == 0 && data["messages"] == "",
        "Isolated review loaded user config or reported error: {data}"
    );
    let buffers = data["buffers"]
        .as_array()
        .context("Missing review buffers")?;
    ensure!(
        buffers.len() == 2
            && buffers
                .iter()
                .all(|b| b["modifiable"] == 0 && b["readonly"] == 1),
        "Review buffers not read-only: {data}"
    );
    ensure!(
        buffers[0]["lines"] == left && buffers[1]["lines"] == right,
        "Review compared wrong snapshots: {data}"
    );
    ensure!(
        h.rpc(json!({"EditorStatus":{"session":id(s)}}))?["Text"] == "0",
        "Review marked modified"
    );
    Ok(())
}
pub fn run(o: &Options) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let mut settings = h.state()?["settings"].clone();
    settings["editor_program"] = json!("/missing/user-editor");
    settings["review_mode"] = json!("native");
    h.rpc(json!({"Settings":settings}))?;
    let p = h.project("review")?;
    let root = PathBuf::from(p["path"].as_str().unwrap());
    let init = h.root.join("config/nvim/init.lua");
    fs::create_dir_all(init.parent().unwrap())?;
    fs::write(init, "vim.g.terminator_user_init = true\n")?;
    git(&root, &["init", "-q"])?;
    git(&root, &["config", "user.name", "Fixture"])?;
    git(&root, &["config", "user.email", "fixture@example.invalid"])?;
    let source = root.join("space | ' 日本.rs");
    fs::write(&source, "fn main() {\n    base();\n}\n")?;
    git(&root, &["add", "."])?;
    git(&root, &["commit", "-qm", "base"])?;
    fs::write(&source, "fn main() {\n    staged();\n}\n")?;
    git(&root, &["add", "."])?;
    fs::write(&source, "fn main() {\n    working();\n}\n")?;
    let shell = h.shell(&p)?;
    let original_index = git(&root, &["ls-files", "--stage", "-z"])?;
    let original_status = git(&root, &["status", "--porcelain=v1", "-z"])?;
    let original_source = fs::read(&source)?;
    for (staged, left, right) in [(true, "base", "staged"), (false, "staged", "working")] {
        let s = h.rpc(
            json!({"CreateReview":{"project":id(&p),"cwd":root,"path":source,"staged":staged}}),
        )?["Created"]
            .clone();
        ensure!(
            s["review"] == true && s["kind"] == "editor",
            "Wrong review identity"
        );
        verify(
            &h,
            &s,
            json!(["fn main() {", format!("    {left}();"), "}"]),
            json!(["fn main() {", format!("    {right}();"), "}"]),
        )?;
        let mut stream = h.attach(&s)?;
        h.write(&mut stream, "t")?;
        thread::sleep(Duration::from_millis(300));
        ensure!(
            inspect(&h, &s)?["buffers"]
                .as_array()
                .unwrap()
                .iter()
                .all(|b| b["modifiable"] == 0 && b["readonly"] == 1),
            "Layout toggle made review writable"
        );
        h.write(&mut stream, "q")?;
        h.wait(|st| session(st, id(&s))["lifecycle"] == "ended", 5)?;
        h.wait(
            |_| !h.root.join(format!("run/review-{}", id(&s))).exists(),
            5,
        )?;
    }
    ensure!(
        git(&root, &["ls-files", "--stage", "-z"])? == original_index
            && git(&root, &["status", "--porcelain=v1", "-z"])? == original_status
            && fs::read(&source)? == original_source,
        "Review mutated Git or source"
    );
    let gui_source = root.join("review.rs");
    fs::write(
        &gui_source,
        "fn main() {\n    working();\n}\n// שלום עולם\n",
    )?;
    h.layout(&p, std::slice::from_ref(&shell))?;
    save_prefs(
        &h,
        &json!({"version":1,"tool":"Git","visible":true,"typography_migrated":true}),
    )?;
    plain(
        &h,
        o,
        "review",
        json!([{"at_ms":1200,"target":"git-file-review.rs","right_click":true},{"at_ms":1600,"target":"Neovim diff"}]),
        3500,
    )?;
    let state = h.state()?;
    let review = sessions(&state)
        .iter()
        .rev()
        .find(|s| s["review"] == true && s["lifecycle"] == "running")
        .context("Native review not created")?;
    verify(
        &h,
        review,
        json!([""]),
        json!(["fn main() {", "    working();", "}", "// שלום עולם"]),
    )?;
    ensure!(
        state["projects"][0]["layout"]["tabs"]
            .as_array()
            .unwrap()
            .len()
            == 2,
        "Review not in top-level tab"
    );
    plain(
        &h,
        o,
        "review-closed",
        json!([{"at_ms":1200,"target":"workspace-close:Diff: review.rs"}]),
        3000,
    )?;
    h.wait(|s| session(s, id(review))["lifecycle"] == "ended", 5)?;
    ensure!(
        session_ids(&h.state()?["projects"][0]["layout"]) == [id(&shell)],
        "Closing review affected shell"
    );
    plain(
        &h,
        o,
        "native-side-by-side",
        json!([
            {"at_ms":1200,"target":"git-file-review.rs","right_click":true},
            {"at_ms":1600,"target":"Native diff"},
            {"at_ms":2200,"target":"diff-side-by-side"}
        ]),
        3500,
    )?;
    ensure!(
        sessions(&h.state()?).len() == sessions(&state).len(),
        "Native diff allocated a PTY"
    );
    h.assert_pids(&[shell])
}
pub(super) struct Proxy {
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
pub(super) fn proxy(root: &Path) -> Result<Proxy> {
    let dir = root.join("legacy");
    fs::create_dir(&dir)?;
    fs::copy(root.join("run/auth"), dir.join("auth"))?;
    let listener = UnixListener::bind(dir.join("daemon.sock"))?;
    listener.set_nonblocking(true)?;
    let target = root.join("run/daemon.sock");
    let stop = Arc::new(AtomicBool::new(false));
    let running = stop.clone();
    let rejected = Arc::new(AtomicUsize::new(0));
    let counter = rejected.clone();
    let thread = thread::spawn(move || {
        while !running.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut client, _)) => {
                    let target = target.clone();
                    let counter = counter.clone();
                    thread::spawn(move || {
                        let _ = (|| -> Result<()> {
                            client.set_nonblocking(false)?;
                            client.set_read_timeout(Some(Duration::from_secs(5)))?;
                            let envelope: Value = read_frame(&mut client)?;
                            let request = &envelope["request"];
                            if request.get("CreateReview").is_some()
                                || request.get("TerminalNotify").is_some()
                            {
                                counter.fetch_add(1, Ordering::Relaxed);
                                return Ok(());
                            }
                            let mut upstream = UnixStream::connect(target)?;
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
                                let _ = write_upstream.shutdown(Shutdown::Both);
                            });
                            let _ = io::copy(&mut upstream, &mut client);
                            let _ = client.shutdown(Shutdown::Both);
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
    let p = h.project("legacy-diff")?;
    let root = PathBuf::from(p["path"].as_str().unwrap());
    git(&root, &["init", "-q"])?;
    git(&root, &["config", "user.name", "Fixture"])?;
    git(&root, &["config", "user.email", "fixture@example.invalid"])?;
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
    let logs = plain(
        &h,
        o,
        "legacy-diff",
        json!([{"at_ms":1200,"target":"git-file-review.rs","right_click":true},{"at_ms":1700,"target":"Neovim diff"}]),
        3500,
    )?;
    ensure!(
        !logs.contains("Fixture error") && proxy.rejected.load(Ordering::Relaxed) == 0,
        "Sent unsupported request or failed legacy diff: {logs}"
    );
    let state = h.state()?;
    ensure!(
        sessions(&state).len() == 1
            && state["projects"][0]["layout"]["tabs"]
                .as_array()
                .unwrap()
                .len()
                == 2,
        "Legacy diff allocated editor or missing tab"
    );
    h.assert_pids(&[shell])
}
