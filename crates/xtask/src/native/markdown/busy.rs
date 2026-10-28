use super::super::{
    Context, Harness, Options, Result, Value, capture, ensure, fs, id, json, sessions,
};
use super::{CaptureInput, gui, launch, view};

pub fn busy(o: &Options) -> Result<()> {
    let (h, shell, root) = launch("markdown-busy")?;
    let path = root.join("README.md");
    let text = "# Markdown while Neovim waits\n\nThe saved document remains readable.\n";
    fs::write(&path, text)?;
    write_blocking_init(&h)?;
    capture_waiting(&h, o)?;
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
    capture_resumed(&h, o, &editor)?;
    h.assert_pids(&[shell, editor])
}

/// Make Neovim open on a pager prompt so the editor is busy.
fn write_blocking_init(h: &Harness) -> Result<()> {
    let init = h.root.join("config/nvim/init.lua");
    fs::create_dir_all(init.parent().unwrap())?;
    fs::write(
        init,
        "vim.api.nvim_create_autocmd('VimEnter', { callback = function() vim.api.nvim_echo({{'Fixture pager\\n' .. table.concat(vim.fn.range(1, 200), '\\n')}}, true, {}) end })\n",
    )?;
    Ok(())
}

fn has_status(h: &Harness, editor: &Value, expected: &str) -> bool {
    gui(h).is_ok_and(|s| {
        s.get("markdown")
            .and_then(|markdown| markdown.get(id(editor)))
            .and_then(|markdown| markdown.get("status"))
            .is_some_and(|status| status == expected)
    })
}

fn capture_waiting(h: &Harness, o: &Options) -> Result<()> {
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "waiting",
            actions: json!([
                {"at_ms":800,"target":"explorer-file:README.md"},
                {"at_ms":1500,"target":"markdown-mode:Preview"}
            ]),
            after: 4000,
        },
        |_| {
            h.wait(
                |state| {
                    sessions(state)
                        .iter()
                        .find(|s| s["kind"] == "editor")
                        .is_some_and(|editor| {
                            view(h, editor, "# Markdown while Neovim waits", false)
                                && has_status(h, editor, "Saved file · live preview paused")
                        })
                },
                8,
            )?;
            Ok(())
        },
    )?;
    Ok(())
}

fn capture_resumed(h: &Harness, o: &Options, editor: &Value) -> Result<()> {
    capture(
        CaptureInput {
            h,
            opts: o,
            name: "resumed",
            actions: json!([]),
            after: 3000,
        },
        |_| {
            h.wait(
                |_| {
                    view(h, editor, "# Markdown while Neovim waits", false)
                        && has_status(h, editor, "Live preview")
                },
                8,
            )?;
            Ok(())
        },
    )?;
    Ok(())
}
