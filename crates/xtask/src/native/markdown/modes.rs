use super::super::{Result, Value, capture, ensure, fs, id, json, plain, sessions};
use super::{CaptureInput, Md, TEXT, expression, gui, stored_mode_is_default, view};

impl Md<'_> {
    pub(super) fn split_unsaved(&self) -> Result<()> {
        let Self { h, o, editor, .. } = *self;
        capture(
            CaptureInput {
                h,
                opts: o,
                name: "split-unsaved",
                actions: json!([
                    {"at_ms":800,"target":"markdown-mode:Split"},
                    {"at_ms":1900,"target":"editor-terminal","input":"gg0C# Unsaved preview\u{1b}"},
                    {"at_ms":2800,"target":"markdown-preview"},
                    {"at_ms":3100,"target":"markdown-preview","input":"x"}
                ]),
                after: 4500,
            },
            |_| {
                h.wait(
                    |_| {
                        view(ViewInput {
                            h,
                            editor,
                            text: "# Unsaved preview",
                            attached: true,
                        })
                    },
                    8,
                )?;
                self.check_split_layout()
            },
        )?;
        ensure!(
            fs::read_to_string(self.path)? == TEXT,
            "Preview saved the unsaved buffer"
        );
        ensure!(
            expression(h, editor, "getline(1)")? == "# Unsaved preview",
            "GUI typing did not reach Neovim"
        );
        h.assert_pids(&[self.shell.clone(), editor.clone()])
    }
    fn check_split_layout(&self) -> Result<()> {
        let snapshot = gui(self.h)?;
        let editor = self.editor;
        ensure!(
            snapshot
                .get("markdown")
                .and_then(|markdown| markdown.get(id(editor)))
                .and_then(|markdown| markdown.get("status"))
                .is_some_and(|status| status == "Unsaved changes"),
            "Live preview did not mark unsaved edits"
        );
        let preview = snapshot
            .get("markdown")
            .and_then(|markdown| markdown.get(id(editor)))
            .and_then(|markdown| markdown.get("rect"))
            .ok_or_else(|| anyhow::anyhow!("missing markdown rect"))?;
        let terminal = snapshot
            .get("editor_rect")
            .ok_or_else(|| anyhow::anyhow!("missing editor rect"))?;
        ensure!(
            preview[2].as_f64().unwrap() > 80.0 && terminal[2].as_f64().unwrap() > 80.0,
            "Split side is too narrow"
        );
        ensure!(
            terminal[0].as_f64().unwrap() + terminal[2].as_f64().unwrap()
                <= preview[0].as_f64().unwrap() + 2.0,
            "Editor and preview overlap"
        );
        Ok(())
    }
    pub(super) fn preview(&self) -> Result<()> {
        let Self { h, o, editor, .. } = *self;
        capture(
            CaptureInput {
                h,
                opts: o,
                name: "preview",
                actions: json!([
                    {"at_ms":900,"target":"markdown-mode:Preview"},
                    {"at_ms":1800,"target":"markdown-preview","input":"x"}
                ]),
                after: 3500,
            },
            |_| {
                h.wait(
                    |_| {
                        view(ViewInput {
                            h,
                            editor,
                            text: "# Unsaved preview",
                            attached: false,
                        })
                    },
                    8,
                )?;
                Ok(())
            },
        )?;
        ensure!(
            expression(h, editor, "getline(1)")? == "# Unsaved preview",
            "Preview forwarded typing to the hidden editor"
        );
        ensure!(
            stored_mode_is_default(h, editor)?,
            "Preview selection was not persisted"
        );
        Ok(())
    }
    pub(super) fn restored_preview(&self) -> Result<()> {
        let Self { h, o, editor, .. } = *self;
        capture(
            CaptureInput {
                h,
                opts: o,
                name: "restored-preview",
                actions: json!([]),
                after: 3500,
            },
            |_| {
                h.wait(
                    |_| {
                        view(ViewInput {
                            h,
                            editor,
                            text: "# Unsaved preview",
                            attached: false,
                        })
                    },
                    8,
                )?;
                self.preview_follows_tab_file()
            },
        )?;
        h.assert_pids(&[self.shell.clone(), editor.clone()])?;
        ensure!(
            sessions(&h.state()?).len() == 2,
            "Mode switching or restart created another editor"
        );
        Ok(())
    }
    // Preview follows this tab's file even if Neovim is showing another buffer.
    fn preview_follows_tab_file(&self) -> Result<()> {
        let Self { h, editor, .. } = *self;
        let original = expression(
            h,
            editor,
            "luaeval('(function() local original = vim.api.nvim_get_current_buf(); vim.api.nvim_buf_set_lines(original, 0, 1, false, {\"# Original buffer preview\"}); local other = vim.api.nvim_create_buf(false, true); vim.api.nvim_buf_set_lines(other, 0, -1, false, {\"# Wrong buffer\"}); vim.bo[other].modified = false; vim.api.nvim_set_current_buf(other); return original end)()')",
        )?;
        h.wait(
            |_| {
                view(ViewInput {
                    h,
                    editor,
                    text: "# Original buffer preview",
                    attached: false,
                })
            },
            8,
        )?;
        expression(
            h,
            editor,
            &format!(
                "luaeval('vim.api.nvim_set_current_buf({})')",
                original.parse::<u64>()?
            ),
        )?;
        Ok(())
    }
    pub(super) fn back_to_edit(&self) -> Result<()> {
        let Self { h, o, editor, .. } = *self;
        plain(CaptureInput {
            h,
            opts: o,
            name: "back-to-edit",
            actions: json!([
                {"at_ms":800,"target":"markdown-mode:Edit"},
                {"at_ms":1800,"target":"editor-terminal","input":"gg0C# Back in editor\u{1b}"}
            ]),
            after: 3200,
        })?;
        ensure!(
            expression(h, editor, "getline(1)")? == "# Back in editor",
            "Edit mode did not reattach the original editor"
        );
        ensure!(
            fs::read_to_string(self.path)? == TEXT,
            "Mode switching changed the file on disk"
        );
        Ok(())
    }
    pub(super) fn restored_edit(&self) -> Result<()> {
        let Self { h, o, editor, .. } = *self;
        capture(
            CaptureInput {
                h,
                opts: o,
                name: "restored-edit",
                actions: json!([]),
                after: 2200,
            },
            |_| {
                h.wait(
                    |_| {
                        gui(h).is_ok_and(|snapshot| {
                            snapshot
                                .get("markdown_modes")
                                .and_then(|modes| modes.get(id(editor)))
                                .is_some_and(|mode| mode == "Edit")
                                && snapshot
                                    .get("visible_terminals")
                                    .and_then(Value::as_array)
                                    .is_some_and(|ids| {
                                        editor.get("id").is_some_and(|id| ids.contains(id))
                                    })
                                && snapshot
                                    .get("markdown")
                                    .and_then(|markdown| markdown.get(id(editor)))
                                    .and_then(|markdown| markdown.get("visible"))
                                    .is_some_and(|visible| visible.as_bool() == Some(false))
                        })
                    },
                    8,
                )?;
                Ok(())
            },
        )?;
        Ok(())
    }
    pub(super) fn paused_unsaved_refresh(&self) -> Result<()> {
        let Self { h, o, editor, .. } = *self;
        capture(
            CaptureInput {
                h,
                opts: o,
                name: "paused-unsaved-refresh",
                actions: json!([
                    {"at_ms":700,"target":"markdown-mode:Preview"},
                    {"at_ms":2500,"target":"markdown-refresh"}
                ]),
                after: 4000,
            },
            |_| {
                h.wait(
                    |_| {
                        view(ViewInput {
                            h,
                            editor,
                            text: "# Back in editor",
                            attached: false,
                        })
                    },
                    8,
                )?;
                h.write(
                    &mut h.attach(editor)?,
                    ":echo join(range(1, 200), \"\\n\")\r",
                )?;
                self.wait_paused_status()
            },
        )?;
        // Only the fixture answers its own prompt; the preview itself never sends input.
        h.write(&mut h.attach(editor)?, "q\r")?;
        ensure!(
            expression(h, editor, "getline(1)")? == "# Back in editor",
            "Paused preview lost unsaved text"
        );
        Ok(())
    }
    fn wait_paused_status(&self) -> Result<()> {
        let Self { h, editor, .. } = *self;
        h.wait(
            |_| {
                gui(h).is_ok_and(|snapshot| {
                    let markdown = snapshot
                        .get("markdown")
                        .and_then(|markdown| markdown.get(id(editor)));
                    markdown
                        .and_then(|markdown| markdown.get("status"))
                        .is_some_and(|status| status == "Unsaved preview · live updates paused")
                        && markdown
                            .and_then(|markdown| markdown.get("text"))
                            .and_then(Value::as_str)
                            .is_some_and(|t| t.contains("# Back in editor"))
                        && markdown
                            .and_then(|markdown| markdown.get("error"))
                            .is_none_or(Value::is_null)
                })
            },
            8,
        )?;
        Ok(())
    }
}
