use anyhow::{Context, Result};
use portable_pty::CommandBuilder;
use std::{ffi::OsString, path::Path};
use terminator_core::*;

pub fn prepare(
    paths: &Paths,
    sid: &str,
    settings: &Settings,
    file: &Path,
    line: Option<u32>,
    column: Option<u32>,
) -> Result<CommandBuilder> {
    let editor = find_executable(&settings.editor_program).context(
        "Editor executable not found; install Neovim or select another editor in Settings",
    )?;
    let mut command = CommandBuilder::new(&editor);
    if settings.editor_mode == EditorMode::Embedded {
        command.arg("--listen");
        command.arg(transport::nvim_listen_arg(&paths.editor_socket(sid)));
        let script = paths.data.join("editor.lua");
        atomic_write(&script, include_bytes!("editor.lua"))?;
        command.args([
            "-c",
            &format!(
                "lua dofile({})",
                serde_json::to_string(&script.to_string_lossy())?
            ),
        ]);
        command.args([
            "--cmd",
            "set autoread",
            "-c",
            "autocmd FocusGained,BufEnter,CursorHold * checktime",
        ]);
        command.args(vim_position(line, column));
        command.arg("--");
        command.arg(file);
    } else {
        command.args(terminal_args(&editor, file, line, column));
    }
    Ok(command)
}
fn vim_position(line: Option<u32>, column: Option<u32>) -> Vec<OsString> {
    line.map(|line| match column {
        Some(column) => format!("+call cursor({},{})", line.max(1), column.max(1)),
        None => format!("+{}", line.max(1)),
    })
    .into_iter()
    .map(OsString::from)
    .collect()
}
fn terminal_args(
    editor: &Path,
    file: &Path,
    line: Option<u32>,
    column: Option<u32>,
) -> Vec<OsString> {
    let name = editor
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let mut args = match name {
        "nvim" | "vim" => vim_position(line, column),
        // +line also works with macOS's nano (Pico), whose column syntax differs.
        "vi" | "nano" | "pico" => line
            .map(|n| format!("+{}", n.max(1)))
            .into_iter()
            .map(OsString::from)
            .collect(),
        "emacs" | "emacsclient" => line
            .map(|n| format!("+{}:{}", n.max(1), column.unwrap_or(1).saturating_sub(1)))
            .into_iter()
            .map(OsString::from)
            .collect(),
        // Custom terminal editors receive only the absolute file operand.
        _ => vec![],
    };
    args.push(file.as_os_str().into());
    args
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_editors_receive_supported_positions_and_a_literal_file() {
        let file = Path::new("/tmp/space - file.rs");
        for (program, prefix) in [
            ("nano", vec!["+12"]),
            ("pico", vec!["+12"]),
            ("vim", vec!["+call cursor(12,3)"]),
            ("vi", vec!["+12"]),
            ("emacs", vec!["+12:2"]),
            ("custom-editor", vec![]),
        ] {
            let mut expected: Vec<OsString> = prefix.into_iter().map(OsString::from).collect();
            expected.push(file.into());
            assert_eq!(
                terminal_args(Path::new(program), file, Some(12), Some(3)),
                expected
            );
            assert_eq!(
                terminal_args(Path::new(program), file, None, None),
                vec![file.as_os_str()]
            );
        }
    }
}
