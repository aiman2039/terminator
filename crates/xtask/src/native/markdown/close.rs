use super::super::{
    Context, Path, Result, capture, ensure, fs, id, json, plain, require_first_layout,
    session_closed, session_ids, sessions,
};
use super::{CaptureInput, Md, TEXT};

impl Md<'_> {
    /// Cancel, then save, closing the dirty preview.
    pub(super) fn close_dirty(&self) -> Result<()> {
        let Self {
            h,
            o,
            shell,
            editor,
            path,
        } = *self;
        let close_target = if o.narrow {
            format!("pane-close:{}", id(editor))
        } else {
            "workspace-close:read me ' 日本.md".into()
        };
        plain(CaptureInput {
            h,
            opts: o,
            name: "cancel-dirty-preview-close",
            actions: json!([
                {"at_ms":800,"target":"markdown-mode:Preview"},
                {"at_ms":1700,"target":close_target},
                {"at_ms":2600,"target":"Cancel"}
            ]),
            after: 3500,
        })?;
        h.assert_pids(&[shell.clone(), editor.clone()])?;
        ensure!(
            fs::read_to_string(path)? == TEXT,
            "Cancelling preview close saved or discarded changes"
        );
        plain(CaptureInput {
            h,
            opts: o,
            name: "save-dirty-preview-close",
            actions: json!([
                {"at_ms":1000,"target":close_target},
                {"at_ms":1900,"target":"Save and close"}
            ]),
            after: 3300,
        })?;
        h.wait(|s| session_closed(s, id(editor)), 5)?;
        ensure!(
            fs::read_to_string(path)?.starts_with("# Back in editor\n"),
            "Save and close lost Markdown edits"
        );
        Ok(())
    }
    /// Open a second document, dirty it, and discard on close.
    pub(super) fn discard(&self, root: &Path) -> Result<()> {
        let Self { h, o, shell, .. } = *self;
        let discard_path = root.join("discard.md");
        let discard_text = "# Keep on disk\n";
        fs::write(&discard_path, discard_text)?;
        capture(
            CaptureInput {
                h,
                opts: o,
                name: "discard-dirty-preview-open",
                actions: json!([{"at_ms":1000,"target":"explorer-file:discard.md"}]),
                after: 3200,
            },
            |_| h.type_into_running_editor(Some("discard.md"), "iDiscarded\n", 8),
        )?;
        let discard_editor = sessions(&h.state()?)
            .iter()
            .find(|s| {
                s["kind"] == "editor" && s["lifecycle"] == "running" && s["label"] == "discard.md"
            })
            .context("Discard Markdown editor was not opened")?
            .clone();
        let discard_close = if o.narrow {
            format!("pane-close:{}", id(&discard_editor))
        } else {
            "workspace-close:discard.md".into()
        };
        plain(CaptureInput {
            h,
            opts: o,
            name: "discard-dirty-preview-close",
            actions: json!([
                {"at_ms":800,"target":discard_close},
                {"at_ms":2000,"target":"Discard changes"}
            ]),
            after: 3500,
        })?;
        h.wait(|s| session_closed(s, id(&discard_editor)), 5)?;
        ensure!(
            fs::read_to_string(&discard_path)? == discard_text,
            "Discard wrote Markdown edits"
        );
        ensure!(
            session_ids(require_first_layout(&h.state()?)?) == [id(shell)],
            "Closing Markdown changed the shell layout"
        );
        Ok(())
    }
}
