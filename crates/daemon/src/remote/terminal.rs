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
    pub sid: &'a str,
    pub cwd: Option<&'a Path>,
    pub file: Option<&'a Path>,
    pub line: Option<u32>,
    pub column: Option<u32>,
}
pub(crate) fn prepare(
    input: PrepareInput<'_>,
) -> Result<(CommandBuilder, RemoteSession, Option<PathBuf>)> {
    let PrepareInput {
        project,
        control,
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
    let mut script = format!(
        "set -eu; cd -- {}; export COLORTERM=truecolor; ",
        terminator_core::quote(&cwd)
    );
    if let Some(file) = &file {
        let socket = socket.as_deref().context("Remote editor socket missing")?;
        script.push_str("umask 077; d=\"$HOME/.cache/terminator/e\"; test ! -L \"$HOME/.cache/terminator\"; test ! -L \"$d\"; mkdir -p \"$d\"; chmod 700 \"$HOME/.cache/terminator\" \"$d\"; exec nvim --listen \"$HOME/");
        script.push_str(socket);
        script.push_str("\" -c ");
        script.push_str(&terminator_core::quote(&format!(
            "lua assert(loadstring({}))()",
            serde_json::to_string(include_str!("../editor.lua"))?
        )));
        if let Some(line) = line {
            script.push(' ');
            script.push_str(&terminator_core::quote(&format!(
                "+call cursor({},{})",
                line.max(1),
                column.unwrap_or(1).max(1)
            )));
        }
        script.push_str(" -- ");
        script.push_str(&terminator_core::quote(file));
    } else {
        script.push_str("exec \"${SHELL:-/bin/sh}\" -il");
    }
    let ssh = terminator_core::find_executable("ssh").context("OpenSSH client is not installed")?;
    let mut command = CommandBuilder::new(ssh);
    command.args(terminator_core::ssh::arguments(
        &project.ssh,
        true,
        control,
    )?);
    command.arg("--");
    command.arg(&project.ssh.host);
    command.arg(format!("exec sh -c {}", terminator_core::quote(&script)));
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
    #[test]
    fn ssh_terminals_quote_remote_roots_and_do_not_start_a_local_shell() {
        let p = RemoteProject {
            id: "fixture".into(),
            name: "Remote".into(),
            root: "/workspace/path with 'quotes'".into(),
            ssh: terminator_core::ssh::SshOptions {
                host: "fixture".into(),
                ..terminator_core::ssh::SshOptions::default()
            },
            layout: serde_json::Value::Null,
            group: None,
            forwards: Vec::new(),
        };
        let (cmd, meta, _) = prepare(PrepareInput {
            project: &p,
            control: None,
            sid: "00000000-0000-0000-0000-000000000000",
            cwd: None,
            file: None,
            line: None,
            column: None,
        })
        .unwrap();
        assert!(cmd.get_argv().first().is_some_and(|s| {
            Path::new(s)
                .file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case(terminator_core::exe_name("ssh")))
        }));
        assert_eq!(meta.cwd, p.root);
        assert!(meta.editor_socket.is_none());
        assert!(
            prepare(PrepareInput {
                project: &p,
                control: None,
                sid: "fixture",
                cwd: Some(Path::new("/tmp/local")),
                file: None,
                line: None,
                column: None,
            })
            .is_err()
        );
    }
}
