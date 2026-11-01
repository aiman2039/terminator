use super::super::CaptureInput;
use super::super::{
    Context, Harness, Options, Path, PathBuf, Result, Value, capture, ensure, fs, git, json,
    save_prefs, sessions, waiting,
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
    let staged = h.root.join("fixture-diff-hunks-staged");
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "diff-hunks",
            actions: hunk_actions(source, &staged, texts),
            after: 60000,
        },
        |_| {
            // The index is not a file the GUI can watch: tell it once the
            // stage reached Git. Each later step waits for the file state
            // the step before it must leave.
            h.wait(
                |_| {
                    git(root, &["show", ":hunks.txt"])
                        .is_ok_and(|bytes| bytes == texts.middle.as_bytes())
                },
                30,
            )?;
            fs::write(&staged, "")?;
            Ok(())
        },
    )
}

/// The diff-hunks script. Each action waits for its target to hold still,
/// and for the file state its previous action must leave.
fn hunk_actions(source: &Path, staged: &Path, texts: &Texts) -> Value {
    let Texts {
        changed,
        discarded,
        edited,
        ..
    } = texts;
    let is = |text: &str| json!({"file":{"path":source,"equals":text}});
    let mut actions = json!([
        {"target":"git-file-hunks.txt","right_click":true},
        {"target":"Open changes","wait":{"retry_previous_ms":1500}},
        {"target":"diff-next"},
        {"target":"diff-hunk-action-1"},
        {"target":"diff-previous","wait":{"file":{"path":staged}}},
        // The view reloads after the stage reached Git; let it settle.
        {"target":"diff-hunk-discard-0","wait":{"settle_ms":600}},
        {"target":"diff-confirm-discard","wait":{"retry_previous_ms":1500}},
        {"target":"diff-undo","wait":is(discarded)},
        {"target":"diff-side-by-side","wait":is(changed)},
        {"target":"diff-marker-1"},
        {"target":"diff-marker-0"},
        {"target":"diff-menu"},
        {"target":"Collapse unchanged lines","wait":{"retry_previous_ms":1500}}
    ]);
    let rest = json!([
        {"target":"diff-edit-save","wait":key_gap()},
        {"target":"diff-undo","wait":is(edited)},
        {"target":"diff-unified","wait":is(changed)},
        {"target":"diff-edit-split"},
        {"target":"diff-right-editor"},
        {"target":"diff-right-editor","key":"I","wait":key_gap()},
        {"target":"diff-right-editor","input":"unified edit","wait":key_gap()},
        {"target":"diff-right-editor","key":"Escape","wait":key_gap()},
        {"target":"diff-edit-save","wait":key_gap()},
        {"target":"fixture-done","check":true,"capture":true,
         "wait":{"file":{"path":source,"contains":"unified edit"}}}
    ]);
    if let (Some(all), Some(edit), Some(rest)) = (
        actions.as_array_mut(),
        buffer_edit(edited).as_array(),
        rest.as_array(),
    ) {
        all.extend(edit.iter().chain(rest).cloned());
    }
    waiting(actions)
}

/// The Vim keys that replace the split editor's buffer with `edited`
/// (`ggdG`, then insert). No state shows that the editor handled a key, so
/// key steps keep the timed script's spacing (`key_gap`).
fn buffer_edit(edited: &str) -> Value {
    json!([
        {"target":"diff-right-editor"},
        {"target":"diff-right-editor","key":"G","wait":key_gap()},
        {"target":"diff-right-editor","key":"G","wait":key_gap()},
        {"target":"diff-right-editor","key":"D","wait":key_gap()},
        {"target":"diff-right-editor","key":"G","shift":true,"wait":key_gap()},
        {"target":"diff-right-editor","key":"I","wait":key_gap()},
        {"target":"diff-right-editor","input":edited,"wait":key_gap()},
        {"target":"diff-right-editor","key":"Escape","wait":key_gap()}
    ])
}

/// Spacing between editor key steps (the old timeline used 100 ms).
fn key_gap() -> Value {
    json!({"settle_ms":150})
}
