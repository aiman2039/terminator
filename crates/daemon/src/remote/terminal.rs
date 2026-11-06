use super::*;
use portable_pty::CommandBuilder;
use std::path::{Path, PathBuf};
use terminator_core::remote::RemoteSession;

fn resolve_cwd(project: &RemoteProject, cwd: Option<&Path>) -> Result<String> {
    if let Some(cwd) = cwd.filter(|p| *p != project.identity_path()) {
        let value = cwd.to_str().context("Remote cwd must be UTF-8")?;
        if value.starts_with('/') {
            let relative = value
                .strip_prefix(project.root.trim_end_matches('/'))
                .filter(|tail| tail.is_empty() || tail.starts_with('/'))
                .context("Remote cwd is outside the project root")?;
            Ok(project.absolute(relative.trim_start_matches('/'))?)
        } else {
            Ok(project.absolute(value)?)
        }
    } else {
        Ok(project.root.clone())
    }
}
fn resolve_file(project: &RemoteProject, file: Option<&Path>) -> Result<Option<String>> {
    file.map(|path| -> Result<String> {
        let value = path.to_str().context("Remote file path must be UTF-8")?;
        if value.starts_with('/') {
            let relative = value
                .strip_prefix(project.root.trim_end_matches('/'))
                .filter(|tail| tail.starts_with('/'))
                .context("Remote file is outside the project root")?;
            Ok(project.absolute(relative.trim_start_matches('/'))?)
        } else {
            Ok(project.absolute(value)?)
        }
    })
    .transpose()
}
pub(crate) struct PrepareInput<'a> {
    pub project: &'a RemoteProject,
    pub control: Option<&'a Path>,
    /// `terminator-hook`, which starts Mosh sessions (`RemoteProject::mosh`).
    pub helper: &'a Path,
    pub sid: &'a str,
    pub cwd: Option<&'a Path>,
    pub file: Option<&'a Path>,
    pub line: Option<u32>,
    pub column: Option<u32>,
}
/// `editor` is the file and its Neovim socket. `shell` is the host's saved terminal command; editors ignore it.
fn remote_script(
    cwd: &str,
    editor: Option<(&str, &str)>,
    cursor: Option<(u32, Option<u32>)>,
    shell: Option<&str>,
) -> Result<String> {
    let mut script = format!(
        "set -eu; cd -- {}; export COLORTERM=truecolor; ",
        terminator_core::quote(cwd)
    );
    if let Some((file, socket)) = editor {
        script.push_str("umask 077; d=\"$HOME/.cache/terminator/e\"; test ! -L \"$HOME/.cache/terminator\"; test ! -L \"$d\"; mkdir -p \"$d\"; chmod 700 \"$HOME/.cache/terminator\" \"$d\"; exec nvim --listen \"$HOME/");
        script.push_str(socket);
        script.push_str("\" -c ");
        script.push_str(&terminator_core::quote(&format!(
            "lua assert(loadstring({}))()",
            serde_json::to_string(include_str!("../editor.lua"))?
        )));
        if let Some((line, column)) = cursor {
            script.push(' ');
            script.push_str(&terminator_core::quote(&format!(
                "+call cursor({},{})",
                line.max(1),
                column.unwrap_or(1).max(1)
            )));
        }
        script.push_str(" -- ");
        script.push_str(&terminator_core::quote(file));
    } else if let Some(shell) = shell {
        // A user command must not inherit our strict mode.
        script.push_str("set +eu; ");
        script.push_str(shell);
    } else {
        script.push_str(terminator_core::remote::DEFAULT_SHELL_COMMAND);
    }
    Ok(script)
}
pub(crate) fn prepare(
    input: PrepareInput<'_>,
) -> Result<(CommandBuilder, RemoteSession, Option<PathBuf>)> {
    let PrepareInput {
        project,
        control,
        helper,
        sid,
        cwd,
        file,
        line,
        column,
    } = input;
    project.validate()?;
    let cwd = resolve_cwd(project, cwd)?;
    let file = resolve_file(project, file)?;
    let socket = file
        .as_ref()
        .map(|_| format!(".cache/terminator/e/{}.sock", sid.replace('-', "")));
    let script = remote_script(
        &cwd,
        file.as_deref().zip(socket.as_deref()),
        line.map(|line| (line, column)),
        project.shell_command.as_deref(),
    )?;
    let (program, remote) = if project.mosh {
        // The helper runs this over SSH, then becomes `mosh-client`.
        (
            helper.to_path_buf(),
            terminator_core::remote::mosh::server_command(project.mosh_command.as_deref(), &script),
        )
    } else {
        (
            terminator_core::find_executable("ssh").context("OpenSSH client is not installed")?,
            format!("exec sh -c {}", terminator_core::quote(&script)),
        )
    };
    let mut command = CommandBuilder::new(program);
    if project.mosh {
        command.arg("mosh");
    }
    command.args(terminator_core::ssh::arguments(
        &project.ssh,
        true,
        control,
    )?);
    command.arg("--");
    command.arg(&project.ssh.host);
    command.arg(remote);
    for key in terminator_core::ssh::private_environment_keys() {
        command.env_remove(key);
    }
    Ok((
        command,
        RemoteSession {
            ssh: project.ssh.clone(),
            root: project.root.clone(),
            cwd,
            editor_socket: socket,
        },
        file.map(PathBuf::from),
    ))
}
pub(crate) fn editor_rpc(
    session: &RemoteSession,
    control: Option<&Path>,
    expression: &str,
) -> Result<Vec<u8>> {
    let socket = session
        .editor_socket
        .as_deref()
        .context("Remote session is not an editor")?;
    let identity = socket
        .strip_prefix(".cache/terminator/e/")
        .and_then(|s| s.strip_suffix(".sock"))
        .context("Invalid remote editor socket")?;
    ensure!(
        identity.len() == 32 && identity.chars().all(|c| c.is_ascii_hexdigit()),
        "Invalid remote editor identity"
    );
    let mut command = super::command(&session.ssh, control)?;
    command.arg(format!(
        "exec sh -c {}",
        terminator_core::quote(&format!(
            "exec nvim --clean --headless --server \"$HOME/{socket}\" --remote-expr {}",
            terminator_core::quote(expression)
        ))
    ));
    Ok(run_command(
        command,
        CommandOptions {
            timeout: Duration::from_secs(2),
            stdout_limit: 1024 * 1024,
            stderr_limit: 16 * 1024,
            ..CommandOptions::default()
        },
    )?
    .stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(root: &str, mosh: bool) -> RemoteProject {
        RemoteProject {
            id: "fixture".into(),
            name: "Remote".into(),
            root: root.into(),
            ssh: terminator_core::ssh::SshOptions {
                host: "fixture".into(),
                ..terminator_core::ssh::SshOptions::default()
            },
            layout: serde_json::Value::Null,
            group: None,
            forwards: Vec::new(),
            last_connected: 0,
            command_timeout_seconds: None,
            mosh,
            shell_command: None,
            mosh_command: None,
        }
    }

    fn shell<'a>(project: &'a RemoteProject, cwd: Option<&'a Path>) -> PrepareInput<'a> {
        PrepareInput {
            project,
            control: None,
            helper: Path::new("/fixture/terminator-hook"),
            sid: "00000000-0000-0000-0000-000000000000",
            cwd,
            file: None,
            line: None,
            column: None,
        }
    }

    #[test]
    fn ssh_terminals_quote_remote_roots_and_do_not_start_a_local_shell() {
        let p = project("/workspace/path with 'quotes'", false);
        let (cmd, meta, _) = prepare(shell(&p, None)).unwrap();
        assert!(cmd.get_argv().first().is_some_and(|s| {
            Path::new(s)
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case(terminator_core::exe_name("ssh")))
        }));
        assert_eq!(meta.cwd, p.root);
        assert!(meta.editor_socket.is_none());
        let script = cmd
            .get_argv()
            .last()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(script.contains("csh|tcsh) exec \"$s\" -l;;"), "{script}");
        assert!(prepare(shell(&p, Some(Path::new("/tmp/local")))).is_err());
    }

    #[test]
    fn mosh_hosts_start_terminals_through_the_helper() {
        let p = project("/workspace", true);
        let (cmd, _, _) = prepare(shell(&p, None)).unwrap();
        let argv: Vec<String> = cmd
            .get_argv()
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            argv.first().map(String::as_str),
            Some("/fixture/terminator-hook")
        );
        assert_eq!(argv.get(1).map(String::as_str), Some("mosh"));
        let split = argv.iter().position(|a| a == "--").unwrap();
        assert_eq!(argv.get(split + 1).map(String::as_str), Some("fixture"));
        let remote = argv.get(split + 2).unwrap();
        assert!(remote.contains("mosh-server new"), "{remote}");
        assert!(remote.contains("/workspace"), "{remote}");
        assert_eq!(argv.len(), split + 3);
    }

    #[test]
    fn saved_host_commands_start_shells_but_not_editors() {
        let mut p = project("/workspace", true);
        p.shell_command = Some("tmux new -A -s main".into());
        p.mosh_command = Some("mosh-server new -p 60001".into());
        let (cmd, _, _) = prepare(shell(&p, None)).unwrap();
        let remote = cmd
            .get_argv()
            .last()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(
            remote.contains("mosh-server new -p 60001 -- sh -c"),
            "{remote}"
        );
        assert!(remote.contains("tmux new -A -s main"), "{remote}");
        assert!(!remote.contains("exec \"$s\" -il"), "{remote}");
        let mut editor = shell(&p, None);
        editor.file = Some(Path::new("a.txt"));
        let (cmd, _, _) = prepare(editor).unwrap();
        let remote = cmd
            .get_argv()
            .last()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(!remote.contains("tmux"), "{remote}");
        assert!(remote.contains("mosh-server new -p 60001"), "{remote}");
    }
}
