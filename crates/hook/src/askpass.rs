//! `SSH_ASKPASS` mode (`ssh-askpass-v1`). OpenSSH runs this binary with the
//! prompt as its only argument when the daemon's sign-in sets
//! `TERMINATOR_ASKPASS_SOCKET`. The prompt goes to that private socket; the
//! answer typed in the GUI comes back and is printed for OpenSSH. A cancel
//! exits non-zero, so OpenSSH stops. Nothing is saved or logged.
use anyhow::{Context, Result, bail};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    time::Duration,
};
use terminator_core::ssh::sign_in::{AskpassAnswer, AskpassQuestion, MAX_ASKPASS_TEXT};

/// Longer than the daemon's five-minute sign-in limit.
const WAIT: Duration = Duration::from_secs(310);

/// The reply line: a JSON-escaped answer of at most `MAX_ASKPASS_TEXT`
/// bytes plus its frame.
const REPLY_LIMIT: u64 = 2 * MAX_ASKPASS_TEXT as u64 + 64;

/// Askpass mode when the daemon's sign-in named its socket in the
/// environment (OpenSSH passes only the prompt); `None` otherwise.
pub(crate) fn from_env(args: &[String]) -> Option<Result<()>> {
    let socket = std::env::var_os(terminator_core::ssh::sign_in::ASKPASS_SOCKET_ENV)?;
    let prompt = args.first().map_or("", String::as_str);
    Some(run(Path::new(&socket), prompt))
}

#[cfg(unix)]
fn run(socket: &Path, prompt: &str) -> Result<()> {
    let stream = std::os::unix::net::UnixStream::connect(socket)
        .context("Terminator's SSH sign-in is no longer running")?;
    stream.set_read_timeout(Some(WAIT))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let prompt: String = prompt.chars().take(MAX_ASKPASS_TEXT).collect();
    let mut question = serde_json::to_vec(&AskpassQuestion { prompt })?;
    question.push(b'\n');
    (&stream).write_all(&question)?;
    let mut line = String::new();
    BufReader::new((&stream).take(REPLY_LIMIT)).read_line(&mut line)?;
    let reply: AskpassAnswer = serde_json::from_str(&line).context("Invalid sign-in reply")?;
    let Some(answer) = reply.answer else {
        bail!("Sign-in cancelled");
    };
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(answer.expose().as_bytes())?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

#[cfg(not(unix))]
fn run(_socket: &Path, _prompt: &str) -> Result<()> {
    bail!("SSH sign-in in the app needs macOS or Linux")
}
