//! SSH work around a tunneled VNC connection: the optional start command
//! that runs on the host first, and bounded stderr capture so a failed
//! tunnel reports why (unknown host key, rejected key, unreachable host).
use super::transport::ssh_command;
use std::{path::Path, process::Stdio, time::Duration};
use terminator_core::vnc::{VncErrorKind, VncOptions};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    task::JoinHandle,
};

/// Keep at most this much stderr; the rest is read and dropped so OpenSSH
/// never blocks on a full pipe.
const MAX_STDERR: usize = 8 * 1024;
/// How long a failed tunnel waits for OpenSSH to finish writing stderr.
const STDERR_GRACE: Duration = Duration::from_millis(500);
const START_TIMEOUT: Duration = Duration::from_secs(15);

/// Read `stderr` to its end in the background, keeping the first
/// [`MAX_STDERR`] bytes.
pub(super) fn capture<R: AsyncRead + Unpin + Send + 'static>(mut stderr: R) -> JoinHandle<Vec<u8>> {
    tokio::spawn(async move {
        let mut kept = Vec::new();
        let mut chunk = [0u8; 1024];
        while let Ok(read) = stderr.read(&mut chunk).await {
            if read == 0 {
                break;
            }
            let room = MAX_STDERR.saturating_sub(kept.len());
            kept.extend(chunk.iter().take(read.min(room)));
        }
        kept
    })
}

/// The captured stderr of a tunnel that just failed, if OpenSSH wrote any.
pub(super) async fn collected(handle: JoinHandle<Vec<u8>>) -> Option<String> {
    let bytes = tokio::time::timeout(STDERR_GRACE, handle)
        .await
        .ok()?
        .ok()?;
    let text = String::from_utf8_lossy(&bytes).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

/// A classified SSH failure from OpenSSH stderr, with a short message.
pub(super) fn classify(stderr: &str) -> (VncErrorKind, String) {
    let failure = terminator_core::ssh::classify(stderr);
    let kind = VncErrorKind::from_ssh(&failure);
    let detail = stderr.lines().last().unwrap_or(stderr).trim();
    let message = match kind {
        VncErrorKind::SshHostKey => format!(
            "SSH host key is not trusted: {detail}. Open an SSH terminal to the host once to accept it."
        ),
        VncErrorKind::SshAuth => {
            format!("SSH rejected the key or password: {detail}")
        }
        VncErrorKind::Unreachable => format!("SSH host is unreachable: {detail}"),
        _ => format!("SSH tunnel failed: {detail}"),
    };
    (kind, message)
}

/// Run the host's start command over SSH (bounded time and output).
/// `Err` holds the kind and a message with the command's last stderr line.
pub(super) async fn run_start_command(
    options: &VncOptions,
    control: Option<&Path>,
) -> Result<(), (VncErrorKind, String)> {
    let command_text = options.ssh_start_command.trim();
    if command_text.is_empty() {
        return Ok(());
    }
    let failed = |message: String| (VncErrorKind::StartCommand, message);
    let mut command = ssh_command(&options.ssh, control)
        .map_err(|e| failed(format!("Start command could not run: {e}")))?;
    command
        .arg("--")
        .arg(&options.ssh.host)
        .arg(command_text)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|e| failed(format!("Start command could not run: {e}")))?;
    let stderr = child.stderr.take().map(capture);
    let status = tokio::time::timeout(START_TIMEOUT, child.wait())
        .await
        .map_err(|_| failed("Start command did not finish within 15 seconds".into()))?
        .map_err(|e| failed(format!("Start command failed: {e}")))?;
    if status.success() {
        return Ok(());
    }
    let stderr = match stderr {
        Some(handle) => collected(handle).await.unwrap_or_default(),
        None => String::new(),
    };
    // OpenSSH itself failing (exit 255) is an SSH problem, not the command's.
    if status.code() == Some(255) && !stderr.is_empty() {
        return Err(classify(&stderr));
    }
    let detail = stderr.lines().last().unwrap_or("no error output").trim();
    Err(failed(format!(
        "Start command failed on the host ({status}): {detail}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stderr_capture_keeps_only_the_first_bytes() {
        let big = vec![b'x'; MAX_STDERR + 4096];
        let kept = capture(std::io::Cursor::new(big)).await.unwrap();
        assert_eq!(kept.len(), MAX_STDERR);
    }

    #[test]
    fn ssh_stderr_becomes_a_fixable_kind() {
        let (kind, message) = classify("user@h: Permission denied (publickey).");
        assert_eq!(kind, VncErrorKind::SshAuth);
        assert!(message.contains("Permission denied"));
        let (kind, _) = classify("Host key verification failed.");
        assert_eq!(kind, VncErrorKind::SshHostKey);
    }
}
