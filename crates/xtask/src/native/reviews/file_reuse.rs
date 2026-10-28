use super::super::CaptureInput;
use super::super::{
    Context, Harness, Options, PathBuf, Result, Value, ensure, fs, git, id, json, plain,
    save_prefs, sessions,
};
use super::{disable_signing_and_hooks, init_repo};

pub fn file_reuse(o: &Options) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let p = h.project("file-reuse")?;
    let root = PathBuf::from(
        p.get("path")
            .and_then(Value::as_str)
            .context("Missing project path")?,
    );
    init_repo(&root)?;
    disable_signing_and_hooks(&root)?;
    fs::write(root.join("file.txt"), "original\n")?;
    git(&root, &["add", "file.txt"])?;
    git(&root, &["commit", "-qm", "fixture"])?;
    fs::write(root.join("file.txt"), "changed\n")?;
    let mut settings = h
        .state()?
        .get("settings")
        .cloned()
        .context("Missing settings")?;
    let settings_object = settings
        .as_object_mut()
        .context("Settings are not an object")?;
    settings_object.insert("editor_mode".into(), json!("Native"));
    settings_object.insert("review_mode".into(), json!("native"));
    h.rpc(json!({"Settings":settings}))?;
    let shell = h.shell(&p)?;
    h.layout(&p, std::slice::from_ref(&shell))?;
    save_prefs(
        &h,
        &json!({"version":1,"tool":"Explorer","visible":true,"typography_migrated":true}),
    )?;
    plain(CaptureInput {
        h: &h,
        opts: o,
        name: "file-reuse",
        actions: json!([
            {"at_ms":1000,"target":"explorer-file:file.txt"},
            {"at_ms":1800,"target":"explorer-file:file.txt"},
            {"at_ms":2600,"target":"explorer-file:file.txt"},
            {"at_ms":3300,"target":"tool-Git"},
            {"at_ms":4000,"target":"git-file-file.txt"},
            {"at_ms":4600,"target":"git-file-file.txt"},
            {"at_ms":5200,"target":"git-file-file.txt"},
            {"at_ms":5800,"target":"tool-Explorer"},
            {"at_ms":6400,"target":"explorer-file:file.txt"}
        ]),
        after: 7400,
    })?;
    check_single_views(&h.state()?, &p)?;
    h.assert_pids(&[shell])
}

fn count(value: &Value, kind: &str) -> usize {
    match value {
        Value::Object(values) if values.contains_key(kind) => 1,
        Value::Object(values) => values.values().map(|value| count(value, kind)).sum(),
        Value::Array(values) => values.iter().map(|value| count(value, kind)).sum(),
        _ => 0,
    }
}

/// Repeated opens reuse one editor and one diff tab beside the shell.
fn check_single_views(state: &Value, p: &Value) -> Result<()> {
    let layout = state
        .get("projects")
        .and_then(Value::as_array)
        .and_then(|projects| {
            projects
                .iter()
                .find(|project| project.get("id").and_then(Value::as_str) == Some(id(p)))
        })
        .and_then(|project| project.get("layout"))
        .context("Missing saved layout")?;
    let groups = layout
        .get("tabs")
        .and_then(Value::as_array)
        .context("Missing top-level tabs")?;
    for kind in ["NativeEditor", "Diff"] {
        ensure!(
            groups
                .iter()
                .filter_map(|group| group.get("layout"))
                .map(|layout| count(layout, kind))
                .sum::<usize>()
                == 1,
            "Repeated file opens duplicated {kind}: {layout}"
        );
    }
    ensure!(
        groups.len() == 3,
        "Repeated opens created extra top-level tabs"
    );
    ensure!(
        sessions(state).len() == 1,
        "Repeated opens created extra sessions"
    );
    Ok(())
}
