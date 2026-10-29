use super::super::CaptureInput;
use super::super::{
    Context, Harness, Options, PathBuf, Result, Value, capture, ensure, fs, git, json, save_prefs,
    sessions,
};
use super::{disable_signing_and_hooks, init_repo};

/// The file contents each hunk action should leave on disk or in the index.
struct Texts {
    changed: String,
    middle: String,
    discarded: String,
    edited: &'static str,
}

/// Real-window regression for explicit native hunk mutations and navigation.
pub fn hunks(o: &Options) -> Result<()> {
    let mut h = Harness::new()?;
    h.env
        .insert("TERMINATOR_TEST_CAPTURE_COPY".into(), "1".into());
    h.setup()?;
    let p = h.project("diff-hunks")?;
    let root = PathBuf::from(
        p.get("path")
            .and_then(Value::as_str)
            .context("Missing project path")?,
    );
    init_repo(&root)?;
    disable_signing_and_hooks(&root)?;
    let source = root.join("hunks.txt");
    let texts = commit_and_change(&root, &source)?;
    let shell = h.shell(&p)?;
    h.layout(&p, std::slice::from_ref(&shell))?;
    save_prefs(
        &h,
        &json!({"version":1,"tool":"Git","visible":true,"typography_migrated":true}),
    )?;
    let before = sessions(&h.state()?).len();
    drive_hunks(&h, o, (&root, &source, &texts))?;
    ensure!(
        git(&root, &["show", ":hunks.txt"])? == texts.middle.as_bytes(),
        "Hunk stage changed the wrong lines"
    );
    ensure!(
        fs::read_to_string(&source)?.contains("unified edit"),
        "Edit from unified, then save, did not update the working file"
    );
    ensure!(
        sessions(&h.state()?).len() == before,
        "Native diff allocated a PTY"
    );
    h.assert_pids(&[shell])
}

/// Commit 45 numbered lines, then change three separate hunks in the working tree.
fn commit_and_change(root: &std::path::Path, source: &std::path::Path) -> Result<Texts> {
    use std::fmt::Write;
    let mut original = String::new();
    for line in 0..45 {
        writeln!(original, "line {line}")?;
    }
    let changed = original
        .replace(
            "line 2\n",
            "first change\nextra inserted line\nanother inserted line\n",
        )
        .replace("line 20\n", "middle change\n")
        .replace("line 38\n", "");
    let middle = original.replace("line 20\n", "middle change\n");
    let discarded = changed.replace(
        "first change\nextra inserted line\nanother inserted line\n",
        "line 2\n",
    );
    let edited = "edited in the diff viewer\n";
    fs::write(source, &original)?;
    git(root, &["add", "hunks.txt"])?;
    git(root, &["commit", "-qm", "fixture"])?;
    fs::write(source, &changed)?;
    Ok(Texts {
        changed,
        middle,
        discarded,
        edited,
    })
}

fn drive_hunks(
    h: &Harness,
    o: &Options,
    (root, source, texts): (&PathBuf, &PathBuf, &Texts),
) -> Result<String> {
    let Texts {
        changed,
        middle,
        discarded,
        edited,
    } = texts;
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "diff-hunks",
            actions: with_buffer_edit(
                json!([
                {"at_ms":1200,"target":"git-file-hunks.txt","right_click":true},
                {"at_ms":1600,"target":"Open changes"},
                {"at_ms":2200,"target":"diff-next"},
                {"at_ms":2600,"target":"diff-hunk-action-1"},
                {"at_ms":3700,"target":"diff-previous"},
                {"at_ms":4100,"target":"diff-hunk-discard-0"},
                {"at_ms":4500,"target":"diff-confirm-discard"},
                {"at_ms":5900,"target":"diff-undo"},
                {"at_ms":6500,"target":"diff-side-by-side"},
                {"at_ms":7100,"target":"diff-marker-1"},
                {"at_ms":8200,"target":"diff-marker-0"},
                {"at_ms":8600,"target":"diff-menu"},
                {"at_ms":9000,"target":"Collapse unchanged lines"},
                {"at_ms":10700,"target":"diff-edit-save"},
                {"at_ms":11700,"target":"diff-undo"},
                {"at_ms":12700,"target":"diff-unified"},
                {"at_ms":13000,"target":"diff-edit-split"},
                {"at_ms":13400,"target":"diff-right-editor"},
                {"at_ms":13700,"target":"diff-right-editor","key":"I"},
                {"at_ms":14000,"target":"diff-right-editor","input":"unified edit"},
                {"at_ms":14300,"target":"diff-right-editor","key":"Escape"},
                {"at_ms":16000,"target":"diff-edit-save"}
                ]),
                edited,
            ),
            after: 17100,
        },
        |_| {
            h.wait(
                |_| {
                    git(root, &["show", ":hunks.txt"]).is_ok_and(|bytes| bytes == middle.as_bytes())
                },
                8,
            )?;
            h.wait(
                |_| fs::read_to_string(source).is_ok_and(|text| text == *discarded),
                5,
            )?;
            h.wait(
                |_| fs::read_to_string(source).is_ok_and(|text| text == *changed),
                5,
            )?;
            h.wait(
                |_| fs::read_to_string(source).is_ok_and(|text| text == *edited),
                8,
            )?;
            h.wait(
                |_| fs::read_to_string(source).is_ok_and(|text| text == *changed),
                5,
            )?;
            h.wait(
                |_| fs::read_to_string(source).is_ok_and(|text| text.contains("unified edit")),
                8,
            )?;
            Ok(())
        },
    )
}

/// `actions` plus the Vim keys that replace the split editor's buffer with
/// `edited` (`ggdG`, then insert), in time order.
fn with_buffer_edit(mut actions: Value, edited: &str) -> Value {
    let edit = json!([
        {"at_ms":9700,"target":"diff-right-editor"},
        {"at_ms":9850,"target":"diff-right-editor","key":"G"},
        {"at_ms":9950,"target":"diff-right-editor","key":"G"},
        {"at_ms":10050,"target":"diff-right-editor","key":"D"},
        {"at_ms":10150,"target":"diff-right-editor","key":"G","shift":true},
        {"at_ms":10250,"target":"diff-right-editor","key":"I"},
        {"at_ms":10350,"target":"diff-right-editor","input":edited},
        {"at_ms":10450,"target":"diff-right-editor","key":"Escape"}
    ]);
    if let (Some(all), Some(edit)) = (actions.as_array_mut(), edit.as_array()) {
        all.extend(edit.iter().cloned());
        all.sort_by_key(|action| action["at_ms"].as_u64().unwrap_or_default());
    }
    actions
}
