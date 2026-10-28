use super::super::CaptureInput;
use super::super::{
    Context, Duration, Harness, Options, PathBuf, Result, Value, ensure, fs, git, id, json, plain,
    require_first_layout, save_prefs, session_closed, session_ids, sessions, thread,
};
use super::{Expected, init_repo, inspect, verify};

/// A repository with one file that has both staged and working changes.
struct Repo {
    root: PathBuf,
    source: PathBuf,
}

pub fn run(o: &Options) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    h.update_settings(|settings| {
        settings.insert("editor_program".into(), json!("/missing/user-editor"));
        settings.insert("review_mode".into(), json!("neovim"));
    })?;
    let p = h.project("review")?;
    let repo = seed_repo(&h, &p)?;
    let Repo { root, source } = &repo;
    let shell = h.shell(&p)?;
    let original_index = git(root, &["ls-files", "--stage", "-z"])?;
    let original_status = git(root, &["status", "--porcelain=v1", "-z"])?;
    let original_source = fs::read(source)?;
    for review in [(true, "base", "staged"), (false, "staged", "working")] {
        review_snapshot(&h, (&p, &repo), review)?;
    }
    ensure!(
        git(root, &["ls-files", "--stage", "-z"])? == original_index
            && git(root, &["status", "--porcelain=v1", "-z"])? == original_status
            && fs::read(source)? == original_source,
        "Review mutated Git or source"
    );
    gui_review(&h, o, (&p, root, &shell))?;
    native_side_by_side(&h, o)?;
    h.assert_pids(&[shell])
}

/// Write a user Neovim config the review must ignore, then commit, stage and
/// edit one file.
fn seed_repo(h: &Harness, p: &Value) -> Result<Repo> {
    let root = PathBuf::from(
        p.get("path")
            .ok_or_else(|| anyhow::anyhow!("missing project path"))?
            .as_str()
            .unwrap(),
    );
    let init = h.root.join("config/nvim/init.lua");
    fs::create_dir_all(init.parent().unwrap())?;
    fs::write(init, "vim.g.terminator_user_init = true\n")?;
    init_repo(&root)?;
    let source = root.join("space | ' 日本.rs");
    fs::write(&source, "fn main() {\n    base();\n}\n")?;
    git(&root, &["add", "."])?;
    git(&root, &["commit", "-qm", "base"])?;
    fs::write(&source, "fn main() {\n    staged();\n}\n")?;
    git(&root, &["add", "."])?;
    fs::write(&source, "fn main() {\n    working();\n}\n")?;
    Ok(Repo { root, source })
}

/// Open a staged or working review over the daemon, check its snapshots, and close it.
fn review_snapshot(
    h: &Harness,
    (p, repo): (&Value, &Repo),
    (staged, left, right): (bool, &str, &str),
) -> Result<()> {
    let Repo { root, source } = repo;
    let s = h
        .rpc(json!({"CreateReview":{"project":id(p),"cwd":root,"path":source,"staged":staged}}))?
        .get("Created")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("missing Created"))?;
    ensure!(
        s.get("review")
            .is_some_and(|review| review.as_bool() == Some(true))
            && s.get("kind").is_some_and(|kind| kind == "editor"),
        "Wrong review identity"
    );
    verify(
        h,
        &s,
        Expected {
            left: json!(["fn main() {", format!("    {left}();"), "}"]),
            right: json!(["fn main() {", format!("    {right}();"), "}"]),
        },
    )?;
    let mut stream = h.attach(&s)?;
    h.write(&mut stream, "t")?;
    thread::sleep(Duration::from_millis(300));
    ensure!(
        inspect(h, &s)?
            .get("buffers")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow::anyhow!("missing review buffers"))?
            .iter()
            .all(|b| b["modifiable"] == 0 && b["readonly"] == 1),
        "Layout toggle made review writable"
    );
    h.write(&mut stream, "q")?;
    h.wait(|st| session_closed(st, id(&s)), 5)?;
    h.wait(
        |_| !h.root.join(format!("run/review-{}", id(&s))).exists(),
        5,
    )?;
    Ok(())
}

/// Open, reopen and close a Neovim review from the Git sidebar.
fn gui_review(
    h: &Harness,
    o: &Options,
    (p, root, shell): (&Value, &PathBuf, &Value),
) -> Result<()> {
    let gui_source = root.join("review.rs");
    fs::write(
        &gui_source,
        "fn main() {\n    working();\n}\n// שלום עולם\n",
    )?;
    h.layout(p, std::slice::from_ref(shell))?;
    save_prefs(
        h,
        &json!({"version":1,"tool":"Git","visible":true,"typography_migrated":true}),
    )?;
    plain(CaptureInput {
        h,
        opts: o,
        name: "review",
        actions: json!([{"at_ms":1200,"target":"git-file-review.rs","right_click":true},{"at_ms":1600,"target":"Open changes"}]),
        after: 3500,
    })?;
    let state = h.state()?;
    let review = sessions(&state)
        .iter()
        .rev()
        .find(|s| s["review"] == true && s["lifecycle"] == "running")
        .context("Native review not created")?;
    reopen_review(h, o, (&state, review))?;
    plain(CaptureInput {
        h,
        opts: o,
        name: "review-closed",
        actions: json!([{"at_ms":1200,"target":"workspace-close:Diff: review.rs"}]),
        after: 3000,
    })?;
    h.wait(|s| session_closed(s, id(review)), 5)?;
    ensure!(
        session_ids(require_first_layout(&h.state()?)?) == [id(shell)],
        "Closing review affected shell"
    );
    Ok(())
}

/// Reopening the same file must reuse the running review session.
fn reopen_review(h: &Harness, o: &Options, (state, review): (&Value, &Value)) -> Result<()> {
    let review_id = id(review).to_string();
    let session_count = sessions(state).len();
    plain(CaptureInput {
        h,
        opts: o,
        name: "review-reopen",
        actions: json!([
            {"at_ms":1200,"target":"git-file-review.rs","right_click":true},
            {"at_ms":1600,"target":"Open changes"}
        ]),
        after: 3200,
    })?;
    ensure!(
        sessions(&h.state()?).len() == session_count,
        "Reopening the same Neovim review created another session"
    );
    ensure!(
        sessions(&h.state()?)
            .iter()
            .any(|session| id(session) == review_id && session["lifecycle"] == "running"),
        "Reopening a review replaced its existing session"
    );
    verify(
        h,
        review,
        Expected {
            left: json!([""]),
            right: json!(["fn main() {", "    working();", "}", "// שלום עולם"]),
        },
    )?;
    ensure!(
        state
            .get("projects")
            .and_then(|projects| projects.get(0))
            .and_then(|project| project.get("layout"))
            .and_then(|layout| layout.get("tabs"))
            .ok_or_else(|| anyhow::anyhow!("missing projects[0].layout.tabs"))?
            .as_array()
            .unwrap()
            .len()
            == 2,
        "Review not in top-level tab"
    );
    Ok(())
}

fn native_side_by_side(h: &Harness, o: &Options) -> Result<()> {
    let mut settings = h
        .state()?
        .get("settings")
        .cloned()
        .context("Missing settings")?;
    settings
        .as_object_mut()
        .context("Settings are not an object")?
        .insert("review_mode".into(), json!("native"));
    h.rpc(json!({"Settings":settings}))?;
    let before_native = sessions(&h.state()?).len();
    plain(CaptureInput {
        h,
        opts: o,
        name: "native-side-by-side",
        actions: json!([
            {"at_ms":1200,"target":"git-file-review.rs","right_click":true},
            {"at_ms":1600,"target":"Open changes"},
            {"at_ms":2200,"target":"diff-side-by-side"}
        ]),
        after: 3500,
    })?;
    ensure!(
        sessions(&h.state()?).len() == before_native,
        "Native diff allocated a PTY"
    );
    Ok(())
}
