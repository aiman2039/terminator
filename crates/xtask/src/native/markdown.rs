use super::{
    Context, Harness, Options, PathBuf, Result, Value, capture, ensure, fs, id, json, output,
    plain, prefs, session_closed, session_ids, sessions,
};
use terminator_core::{Paths, transport, ui_control};

fn gui(h: &Harness) -> Result<Value> {
    ui_control::rpc(&Paths::at(h.root.clone()), ui_control::Request::Snapshot)
}
fn expression(h: &Harness, editor: &Value, expression: &str) -> Result<String> {
    let mut command = std::process::Command::new("nvim");
    command
        .arg("--server")
        .arg(transport::nvim_listen_arg(
            &Paths::at(h.root.clone()).editor_socket(id(editor)),
        ))
        .args(["--remote-expr", expression]);
    Ok(String::from_utf8(output(command)?)?.trim().into())
}
fn view(h: &Harness, editor: &Value, text: &str, attached: bool) -> bool {
    gui(h).is_ok_and(|snapshot| {
        let Some(markdown) = snapshot
            .get("markdown")
            .and_then(|markdown| markdown.get(id(editor)))
        else {
            return false;
        };
        markdown["visible"] == true
            && markdown["error"].is_null()
            && markdown["text"].as_str().is_some_and(|t| t.contains(text))
            && snapshot
                .get("visible_terminals")
                .and_then(Value::as_array)
                .is_some_and(|s| s.contains(&editor["id"]) == attached)
    })
}

fn check_header(h: &Harness) -> Result<()> {
    let snapshot = gui(h)?;
    let header = snapshot
        .get("markdown_header")
        .ok_or_else(|| anyhow::anyhow!("missing markdown header"))?;
    let rects = ["title", "edit", "preview", "split", "refresh"].map(|key| &header[key]);
    for rect in &rects {
        ensure!(rect.is_array(), "Missing Markdown header control");
    }
    for pair in rects.windows(2) {
        let left = pair
            .first()
            .ok_or_else(|| anyhow::anyhow!("missing header control"))?;
        let right = pair
            .get(1)
            .ok_or_else(|| anyhow::anyhow!("missing header control"))?;
        ensure!(
            (left[1].as_f64().unwrap() - right[1].as_f64().unwrap()).abs() < 1.0,
            "Markdown header wrapped onto a second row"
        );
        ensure!(
            left[0].as_f64().unwrap() + left[2].as_f64().unwrap()
                <= right[0].as_f64().unwrap() + 1.0,
            "Markdown header controls overlap"
        );
    }
    Ok(())
}

pub fn run(o: &Options) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let project = h.project("markdown")?;
    let shell = h.shell(&project)?;
    h.layout(&project, std::slice::from_ref(&shell))?;
    let root = PathBuf::from(
        project
            .get("path")
            .ok_or_else(|| anyhow::anyhow!("missing project path"))?
            .as_str()
            .unwrap(),
    );
    let path = root.join("read me ' 日本.md");
    let text = "# Native Markdown\n\nA **native** preview with *emphasis* and [a local file](next.md).\n\n## Features\n\n- [x] Neovim editing\n- [ ] More notes\n\n| Mode | Purpose |\n| --- | --- |\n| Edit | Neovim |\n| Preview | Reading |\n\n```rust\nfn main() { println!(\"Hello\"); }\n```\n\n![Local illustration](<picture space.png>)\n";
    fs::write(&path, text)?;
    fs::write(root.join("next.md"), "# Linked document\n")?;
    image::RgbaImage::from_fn(180, 48, |x, y| {
        let red =
            60u8.saturating_add(u8::try_from(x.checked_div(3).unwrap_or(0)).unwrap_or(u8::MAX));
        let green = 120u8.saturating_add(u8::try_from(y).unwrap_or(u8::MAX));
        image::Rgba([red, green, 210, 255])
    })
    .save(root.join("picture space.png"))?;

    capture(
        &h,
        o,
        "default-preview",
        json!([
            {"at_ms":1000,"target":"explorer-file:read me ' 日本.md"}
        ]),
        3200,
        |_| {
            h.wait(
                |state| {
                    sessions(state)
                        .iter()
                        .find(|s| s["kind"] == "editor")
                        .is_some_and(|editor| view(&h, editor, "# Native Markdown", false))
                },
                8,
            )?;
            check_header(&h)
        },
    )?;
    let editor = sessions(&h.state()?)
        .iter()
        .find(|s| s["kind"] == "editor")
        .context("Markdown editor was not opened")?
        .clone();
    ensure!(
        sessions(&h.state()?).len() == 2,
        "Markdown opening duplicated sessions"
    );
    ensure!(
        prefs(&h)?
            .get("markdown_modes")
            .ok_or_else(|| anyhow::anyhow!("missing markdown_modes"))?
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("markdown_modes is not an object"))?
            .get(id(&editor))
            .is_none_or(Value::is_null),
        "A newly opened Markdown file must use the default Preview mode"
    );

    capture(
        &h,
        o,
        "split-unsaved",
        json!([
            {"at_ms":800,"target":"markdown-mode:Split"},
            {"at_ms":1900,"target":"editor-terminal","input":"gg0C# Unsaved preview\u{1b}"},
            {"at_ms":2800,"target":"markdown-preview"},
            {"at_ms":3100,"target":"markdown-preview","input":"x"}
        ]),
        4500,
        |_| {
            h.wait(|_| view(&h, &editor, "# Unsaved preview", true), 8)?;
            let snapshot = gui(&h)?;
            ensure!(
                snapshot
                    .get("markdown")
                    .and_then(|markdown| markdown.get(id(&editor)))
                    .and_then(|markdown| markdown.get("status"))
                    .is_some_and(|status| status == "Unsaved changes"),
                "Live preview did not mark unsaved edits"
            );
            let preview = snapshot
                .get("markdown")
                .and_then(|markdown| markdown.get(id(&editor)))
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
        },
    )?;
    ensure!(
        fs::read_to_string(&path)? == text,
        "Preview saved the unsaved buffer"
    );
    ensure!(
        expression(&h, &editor, "getline(1)")? == "# Unsaved preview",
        "GUI typing did not reach Neovim"
    );
    h.assert_pids(&[shell.clone(), editor.clone()])?;

    capture(
        &h,
        o,
        "preview",
        json!([
            {"at_ms":900,"target":"markdown-mode:Preview"},
            {"at_ms":1800,"target":"markdown-preview","input":"x"}
        ]),
        3500,
        |_| {
            h.wait(|_| view(&h, &editor, "# Unsaved preview", false), 8)?;
            Ok(())
        },
    )?;
    ensure!(
        expression(&h, &editor, "getline(1)")? == "# Unsaved preview",
        "Preview forwarded typing to the hidden editor"
    );
    ensure!(
        prefs(&h)?
            .get("markdown_modes")
            .ok_or_else(|| anyhow::anyhow!("missing markdown_modes"))?
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("markdown_modes is not an object"))?
            .get(id(&editor))
            .is_none_or(Value::is_null),
        "Preview selection was not persisted"
    );

    capture(&h, o, "restored-preview", json!([]), 3500, |_| {
        h.wait(|_| view(&h, &editor, "# Unsaved preview", false), 8)?;
        // Preview follows this tab's file even if Neovim is showing another buffer.
        let original = expression(
            &h,
            &editor,
            "luaeval('(function() local original = vim.api.nvim_get_current_buf(); vim.api.nvim_buf_set_lines(original, 0, 1, false, {\"# Original buffer preview\"}); local other = vim.api.nvim_create_buf(false, true); vim.api.nvim_buf_set_lines(other, 0, -1, false, {\"# Wrong buffer\"}); vim.bo[other].modified = false; vim.api.nvim_set_current_buf(other); return original end)()')",
        )?;
        h.wait(|_| view(&h, &editor, "# Original buffer preview", false), 8)?;
        expression(
            &h,
            &editor,
            &format!(
                "luaeval('vim.api.nvim_set_current_buf({})')",
                original.parse::<u64>()?
            ),
        )?;
        Ok(())
    })?;
    h.assert_pids(&[shell.clone(), editor.clone()])?;
    ensure!(
        sessions(&h.state()?).len() == 2,
        "Mode switching or restart created another editor"
    );

    plain(
        &h,
        o,
        "back-to-edit",
        json!([
            {"at_ms":800,"target":"markdown-mode:Edit"},
            {"at_ms":1800,"target":"editor-terminal","input":"gg0C# Back in editor\u{1b}"}
        ]),
        3200,
    )?;
    ensure!(
        expression(&h, &editor, "getline(1)")? == "# Back in editor",
        "Edit mode did not reattach the original editor"
    );
    ensure!(
        fs::read_to_string(&path)? == text,
        "Mode switching changed the file on disk"
    );

    capture(&h, o, "restored-edit", json!([]), 2200, |_| {
        h.wait(
            |_| {
                gui(&h).is_ok_and(|snapshot| {
                    snapshot
                        .get("markdown_modes")
                        .and_then(|modes| modes.get(id(&editor)))
                        .is_some_and(|mode| mode == "Edit")
                        && snapshot
                            .get("visible_terminals")
                            .and_then(Value::as_array)
                            .is_some_and(|ids| editor.get("id").is_some_and(|id| ids.contains(id)))
                        && snapshot
                            .get("markdown")
                            .and_then(|markdown| markdown.get(id(&editor)))
                            .and_then(|markdown| markdown.get("visible"))
                            .is_some_and(|visible| visible.as_bool() == Some(false))
                })
            },
            8,
        )?;
        Ok(())
    })?;

    capture(
        &h,
        o,
        "paused-unsaved-refresh",
        json!([
            {"at_ms":700,"target":"markdown-mode:Preview"},
            {"at_ms":2500,"target":"markdown-refresh"}
        ]),
        4000,
        |_| {
            h.wait(|_| view(&h, &editor, "# Back in editor", false), 8)?;
            h.write(
                &mut h.attach(&editor)?,
                ":echo join(range(1, 200), \"\\n\")\r",
            )?;
            h.wait(
                |_| {
                    gui(&h).is_ok_and(|snapshot| {
                        let markdown = snapshot
                            .get("markdown")
                            .and_then(|markdown| markdown.get(id(&editor)));
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
        },
    )?;
    // Only the fixture answers its own prompt; the preview itself never sends input.
    h.write(&mut h.attach(&editor)?, "q\r")?;
    ensure!(
        expression(&h, &editor, "getline(1)")? == "# Back in editor",
        "Paused preview lost unsaved text"
    );

    let close_target = if o.narrow {
        format!("pane-close:{}", id(&editor))
    } else {
        "workspace-close:read me ' 日本.md".into()
    };
    plain(
        &h,
        o,
        "cancel-dirty-preview-close",
        json!([
            {"at_ms":800,"target":"markdown-mode:Preview"},
            {"at_ms":1700,"target":close_target},
            {"at_ms":2600,"target":"Cancel"}
        ]),
        3500,
    )?;
    h.assert_pids(&[shell.clone(), editor.clone()])?;
    ensure!(
        fs::read_to_string(&path)? == text,
        "Cancelling preview close saved or discarded changes"
    );
    plain(
        &h,
        o,
        "save-dirty-preview-close",
        json!([
            {"at_ms":1000,"target":close_target},
            {"at_ms":1900,"target":"Save and close"}
        ]),
        3300,
    )?;
    h.wait(|s| session_closed(s, id(&editor)), 5)?;
    ensure!(
        fs::read_to_string(&path)?.starts_with("# Back in editor\n"),
        "Save and close lost Markdown edits"
    );
    let discard_path = root.join("discard.md");
    let discard_text = "# Keep on disk\n";
    fs::write(&discard_path, discard_text)?;
    capture(
        &h,
        o,
        "discard-dirty-preview-open",
        json!([{"at_ms":1000,"target":"explorer-file:discard.md"}]),
        3200,
        |_| {
            let state = h.wait(
                |st| {
                    sessions(st).iter().any(|s| {
                        s["kind"] == "editor"
                            && s["lifecycle"] == "running"
                            && s["label"] == "discard.md"
                    })
                },
                8,
            )?;
            let editor = sessions(&state)
                .iter()
                .find(|s| {
                    s["kind"] == "editor"
                        && s["lifecycle"] == "running"
                        && s["label"] == "discard.md"
                })
                .unwrap()
                .clone();
            h.write(&mut h.attach(&editor)?, "iDiscarded\n")?;
            h.wait(
                |_| {
                    h.rpc(json!({"EditorStatus":{"session":id(&editor)}}))
                        .is_ok_and(|r| {
                            r.get("Text")
                                .and_then(Value::as_str)
                                .and_then(|s| s.trim().parse::<u32>().ok())
                                .is_some_and(|n| n > 0)
                        })
                },
                3,
            )?;
            Ok(())
        },
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
    plain(
        &h,
        o,
        "discard-dirty-preview-close",
        json!([
            {"at_ms":800,"target":discard_close},
            {"at_ms":2000,"target":"Discard changes"}
        ]),
        3500,
    )?;
    h.wait(|s| session_closed(s, id(&discard_editor)), 5)?;
    ensure!(
        fs::read_to_string(&discard_path)? == discard_text,
        "Discard wrote Markdown edits"
    );
    ensure!(
        session_ids(
            h.state()?
                .get("projects")
                .and_then(|projects| projects.get(0))
                .and_then(|project| project.get("layout"))
                .ok_or_else(|| anyhow::anyhow!("missing projects[0].layout"))?,
        ) == [id(&shell)],
        "Closing Markdown changed the shell layout"
    );
    h.assert_pids(std::slice::from_ref(&shell))
}

pub fn busy(o: &Options) -> Result<()> {
    let h = Harness::new()?;
    h.setup()?;
    let project = h.project("markdown-busy")?;
    let shell = h.shell(&project)?;
    h.layout(&project, std::slice::from_ref(&shell))?;
    let root = PathBuf::from(
        project
            .get("path")
            .ok_or_else(|| anyhow::anyhow!("missing project path"))?
            .as_str()
            .unwrap(),
    );
    let path = root.join("README.md");
    let text = "# Markdown while Neovim waits\n\nThe saved document remains readable.\n";
    fs::write(&path, text)?;
    let init = h.root.join("config/nvim/init.lua");
    fs::create_dir_all(init.parent().unwrap())?;
    fs::write(
        init,
        "vim.api.nvim_create_autocmd('VimEnter', { callback = function() vim.api.nvim_echo({{'Fixture pager\\n' .. table.concat(vim.fn.range(1, 200), '\\n')}}, true, {}) end })\n",
    )?;
    capture(
        &h,
        o,
        "waiting",
        json!([
            {"at_ms":800,"target":"explorer-file:README.md"},
            {"at_ms":1500,"target":"markdown-mode:Preview"}
        ]),
        4000,
        |_| {
            h.wait(
                |state| {
                    sessions(state)
                        .iter()
                        .find(|s| s["kind"] == "editor")
                        .is_some_and(|editor| {
                            view(&h, editor, "# Markdown while Neovim waits", false)
                                && gui(&h).is_ok_and(|s| {
                                    s.get("markdown")
                                        .and_then(|markdown| markdown.get(id(editor)))
                                        .and_then(|markdown| markdown.get("status"))
                                        .is_some_and(|status| {
                                            status == "Saved file · live preview paused"
                                        })
                                })
                        })
                },
                8,
            )?;
            Ok(())
        },
    )?;
    let editor = sessions(&h.state()?)
        .iter()
        .find(|s| s["kind"] == "editor")
        .context("Missing fixture editor")?
        .clone();
    ensure!(
        sessions(&h.state()?).len() == 2,
        "Busy preview duplicated the editor"
    );
    ensure!(
        h.history(id(&editor))?.contains("Fixture pager"),
        "Fixture did not enter a blocking prompt"
    );
    ensure!(
        fs::read_to_string(&path)? == text,
        "Preview changed the saved file"
    );
    h.assert_pids(&[shell.clone(), editor.clone()])?;
    h.write(&mut h.attach(&editor)?, "q\r")?;
    capture(&h, o, "resumed", json!([]), 3000, |_| {
        h.wait(
            |_| {
                view(&h, &editor, "# Markdown while Neovim waits", false)
                    && gui(&h).is_ok_and(|s| {
                        s.get("markdown")
                            .and_then(|markdown| markdown.get(id(&editor)))
                            .and_then(|markdown| markdown.get("status"))
                            .is_some_and(|status| status == "Live preview")
                    })
            },
            8,
        )?;
        Ok(())
    })?;
    h.assert_pids(&[shell, editor])
}
