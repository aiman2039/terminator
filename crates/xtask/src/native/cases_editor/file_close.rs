use super::super::super::harness::{Harness, require_first_layout, session_ids, sessions};
use super::super::dispatch::{CaptureInput, Options, capture, setup};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{fs, path::Path};

pub(crate) fn file_close(o: &Options) -> Result<()> {
    let (h, _, shells, root) = setup("file-close")?;
    let source = root.join("source.rs");
    fs::write(&source, "fn main() {}\n")?;
    // A native runner cannot guarantee the time between rendered clicks.
    // Rapid-repeat suppression has a separate unit regression; this fixture
    // checks that closing one clean editor preserves the shell layout.
    clean_open(&h, o)?;
    clean_close(&h, o)?;
    save_and_close(&h, o, &source)?;
    discard_and_close(&h, o, &root)?;
    unsaved_close_bar(&h, o, &root)?;
    h.assert_pids(&shells)
}

fn clean_open(h: &Harness, o: &Options) -> Result<()> {
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "clean-file-open",
            actions: json!([{"at_ms":1100,"target":"explorer-file:source.rs"}]),
            after: 6000,
        },
        |_| {
            let state = h.wait(
                |state| {
                    sessions(state).iter().any(|session| {
                        session.get("kind").is_some_and(|kind| kind == "editor")
                            && session
                                .get("lifecycle")
                                .is_some_and(|lifecycle| lifecycle == "running")
                    })
                },
                6,
            )?;
            ensure!(
                sessions(&state)
                    .iter()
                    .filter(|s| s["kind"] == "editor")
                    .count()
                    == 1,
                "Clean open did not create exactly one editor"
            );
            Ok(())
        },
    )?;
    Ok(())
}

fn clean_close(h: &Harness, o: &Options) -> Result<()> {
    let close_log = capture(
        CaptureInput {
            h,
            opts: o,
            name: "clean-file-close",
            actions: json!([{"at_ms":1100,"target":"workspace-close:source.rs"}]),
            after: 2600,
        },
        |_| Ok(()),
    )?;
    let state = h.state()?;
    let editors = sessions(&state)
        .iter()
        .filter(|s| s["kind"] == "editor")
        .collect::<Vec<_>>();
    ensure!(
        editors.iter().all(|editor| editor["lifecycle"] == "ended"),
        "Clean close failed: editor lifecycles {:?}; {close_log}",
        editors
            .iter()
            .map(|editor| &editor["lifecycle"])
            .collect::<Vec<_>>()
    );
    Ok(())
}

fn save_and_close(h: &Harness, o: &Options, source: &Path) -> Result<()> {
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "dirty-file-close",
            actions: json!([{"at_ms":1100,"target":"explorer-file:source.rs"},{"at_ms":2700,"target":"workspace-close:source.rs"},{"at_ms":3400,"target":"Cancel"},{"at_ms":4100,"target":"workspace-close:source.rs"},{"at_ms":4700,"target":"Save and close"}]),
            after: 6500,
        },
        |_| h.type_into_running_editor(None, "iUnsaved change "),
    )?;
    ensure!(
        fs::read_to_string(source)?.contains("Unsaved change"),
        "Save-and-close lost buffer"
    );
    let state = h.state()?;
    assert_editors_closed(&state, "Editor did not close", "Close removed shells")
}

fn discard_and_close(h: &Harness, o: &Options, root: &Path) -> Result<()> {
    let keep = root.join("keep.rs");
    fs::write(&keep, "fn keep() {}\n")?;
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "dirty-file-discard",
            actions: json!([
                {"at_ms":1100,"target":"explorer-file:keep.rs"},
                {"at_ms":2700,"target":"workspace-close:keep.rs"},
                {"at_ms":3600,"target":"Discard changes"}
            ]),
            after: 5500,
        },
        |_| h.type_into_running_editor(Some("keep.rs"), "iDiscarded change "),
    )?;
    ensure!(
        fs::read_to_string(&keep)? == "fn keep() {}\n",
        "Discard wrote the buffer"
    );
    let state = h.state()?;
    assert_editors_closed(
        &state,
        "Discard did not close the editor",
        "Discard close removed shells",
    )
}

fn unsaved_close_bar(h: &Harness, o: &Options, root: &Path) -> Result<()> {
    let bar = root.join("bar.rs");
    fs::write(&bar, "fn bar() {}\n")?;
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "unsaved-close-bar",
            actions: json!([
                {"at_ms":1100,"target":"explorer-file:bar.rs"},
                {"at_ms":2700,"target":"workspace-close:bar.rs"}
            ]),
            after: 3400,
        },
        |_| h.type_into_running_editor(Some("bar.rs"), "iBar change "),
    )?;
    ensure!(
        fs::read_to_string(&bar)? == "fn bar() {}\n",
        "Bar capture wrote the buffer"
    );
    Ok(())
}

/// Every editor ended and the six shell panes remain in the first layout.
fn assert_editors_closed(state: &Value, editor_message: &str, shells_message: &str) -> Result<()> {
    ensure!(
        sessions(state)
            .iter()
            .filter(|s| s["kind"] == "editor")
            .all(|s| s["lifecycle"] == "ended"),
        "{editor_message}"
    );
    ensure!(
        session_ids(require_first_layout(state)?).len() == 6,
        "{shells_message}"
    );
    Ok(())
}
