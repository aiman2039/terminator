//! Strict SFTP file transfer shared by VNC sessions and SSH projects.
//!
//! Uploads write a create-new temporary file, fsync it, and publish it with
//! a hard link; downloads write a local temporary file and publish it with
//! a no-clobber rename. Completed destinations are never overwritten.
use anyhow::{Context, Result, ensure};
use openssh_sftp_client::{Sftp, SftpOptions, file::TokioCompatFile};
use std::{path::Path, process::Stdio};
use terminator_core::vnc::MAX_TRANSFER_BYTES;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Observes a running transfer. `check` runs before every read so the owner
/// can stop it; `advance` reports the bytes copied so far.
pub(crate) trait Progress {
    fn check(&self) -> Result<()> {
        Ok(())
    }
    /// The source size, once known.
    fn size(&self, _bytes: u64) {}
    fn advance(&self, bytes: u64);
}

/// Transfer IDs become part of remote temporary file names.
pub(crate) fn validate_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c)),
        "Invalid transfer ID"
    );
    Ok(())
}

/// The create-new remote file an upload writes before it is published.
pub(crate) fn partial_path(remote: &str, id: &str) -> String {
    format!("{remote}.terminator-{id}.partial")
}

pub(crate) async fn copy<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    mut input: R,
    mut output: W,
    progress: &dyn Progress,
) -> Result<u64> {
    let mut buffer = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        progress.check()?;
        let read = input.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(u64::try_from(read)?)
            .context("Transfer size overflow")?;
        ensure!(total <= MAX_TRANSFER_BYTES, "Transfer exceeds 1 GiB");
        output
            .write_all(buffer.get(..read).context("Invalid transfer chunk")?)
            .await?;
        progress.advance(total);
    }
    output.flush().await?;
    Ok(total)
}

/// Runs `body` against an SFTP session over `ssh -s sftp`, riding the
/// OpenSSH master at `control` when one is alive. SSH failures come back
/// classified.
pub(crate) async fn with_sftp<T>(
    ssh: &terminator_core::ssh::SshOptions,
    control: Option<&Path>,
    body: impl AsyncFnOnce(&Sftp) -> Result<T>,
) -> Result<T> {
    let mut command = tokio::process::Command::new("ssh");
    command
        .args(terminator_core::ssh::arguments(ssh, false, control)?)
        .kill_on_drop(true);
    for key in terminator_core::ssh::private_environment_keys() {
        command.env_remove(key);
    }
    command
        .arg("-s")
        .arg("--")
        .arg(&ssh.host)
        .arg("sftp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().context("Cannot start SSH SFTP subsystem")?;
    let stderr = super::capture_stderr(&mut child);
    let input = child.stdin.take().context("SFTP stdin unavailable")?;
    let output = child.stdout.take().context("SFTP stdout unavailable")?;
    let result = async {
        let sftp = Sftp::new(input, output, SftpOptions::default())
            .await
            .context("SFTP negotiation failed; verify SSH host key and authentication")?;
        let value = body(&sftp).await?;
        sftp.close().await?;
        Ok::<T, anyhow::Error>(value)
    }
    .await;
    let _ = child.start_kill();
    let _ = child.wait().await;
    match result {
        Ok(value) => Ok(value),
        Err(error) => Err(super::sftp_failure(error, stderr).await),
    }
}

/// Uploads the regular file `local` to the absolute remote path `remote`.
/// Fails, without replacing anything, when `remote` already exists.
pub(crate) struct UploadInput<'a> {
    pub sftp: &'a Sftp,
    pub local: &'a Path,
    pub remote: &'a str,
    pub id: &'a str,
    pub progress: &'a dyn Progress,
}
pub(crate) async fn upload(input: UploadInput<'_>) -> Result<u64> {
    let UploadInput {
        sftp,
        local,
        remote,
        id,
        progress,
    } = input;
    ensure!(
        sftp.support_hardlink() && sftp.support_fsync(),
        "Server must support hard-link publication and fsync for strict SFTP uploads"
    );
    let source = tokio::fs::File::open(local).await?;
    let metadata = source.metadata().await?;
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_TRANSFER_BYTES,
        "Upload source must be a regular file no larger than 1 GiB"
    );
    progress.size(metadata.len());
    // Sftp::close waits for every SftpHandle; this Fs is dropped on return.
    let mut fs = sftp.fs();
    let temporary = partial_path(remote, id);
    let file = sftp
        .options()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .await?;
    fs.set_permissions(
        &temporary,
        openssh_sftp_client::metadata::Permissions::from(0o600),
    )
    .await?;
    let mut file = Box::pin(TokioCompatFile::from(file));
    let result = async {
        let bytes = copy(source, file.as_mut(), progress).await?;
        ensure!(
            bytes == metadata.len(),
            "Upload source changed during transfer"
        );
        file.as_mut().shutdown().await?;
        file.as_mut().as_mut_file().sync_all().await?;
        drop(file);
        fs.hard_link(&temporary, remote).await.context(
            "Remote destination already exists or cannot be published; it was not overwritten",
        )?;
        Ok::<u64, anyhow::Error>(bytes)
    }
    .await;
    let _ = fs.remove_file(&temporary).await;
    result
}

pub(crate) struct DownloadInput<'a> {
    pub(crate) sftp: &'a Sftp,
    pub(crate) remote: &'a str,
    pub(crate) local: &'a Path,
    pub(crate) progress: &'a dyn Progress,
}

/// Downloads the regular remote file `remote` to the absolute local path
/// `local`. Fails, without replacing anything, when `local` already exists.
pub(crate) async fn download(input: DownloadInput<'_>) -> Result<u64> {
    let DownloadInput {
        sftp,
        remote,
        local,
        progress,
    } = input;
    let mut file = sftp.open(remote).await?;
    let metadata = file.metadata().await?;
    ensure!(
        metadata.file_type().is_some_and(|t| t.is_file())
            && metadata.len().is_some_and(|n| n <= MAX_TRANSFER_BYTES),
        "Download source must be a regular file no larger than 1 GiB"
    );
    if let Some(size) = metadata.len() {
        progress.size(size);
    }
    let parent = local
        .parent()
        .context("Download destination has no parent")?;
    let temporary = tempfile::NamedTempFile::new_in(parent)?;
    let mut output = tokio::fs::File::from_std(temporary.reopen()?);
    let bytes = copy(Box::pin(TokioCompatFile::from(file)), &mut output, progress).await?;
    ensure!(
        Some(bytes) == metadata.len(),
        "Remote file changed during download"
    );
    output.sync_all().await?;
    drop(output);
    publish_local(temporary, local)?;
    Ok(bytes)
}

/// Moves a finished download into place. Never replaces `local`.
pub(crate) fn publish_local(temporary: tempfile::NamedTempFile, local: &Path) -> Result<()> {
    temporary
        .persist_noclobber(local)
        .map_err(|e| e.error)
        .context(
            "Local destination already exists or cannot be published; it was not overwritten",
        )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    #[derive(Default)]
    struct Recorder {
        seen: Mutex<Vec<u64>>,
        stop_after: Option<usize>,
    }
    impl Progress for Recorder {
        fn check(&self) -> Result<()> {
            let seen = self.seen.lock().unwrap().len();
            ensure!(self.stop_after.is_none_or(|n| seen < n), "stopped");
            Ok(())
        }
        fn advance(&self, bytes: u64) {
            self.seen.lock().unwrap().push(bytes);
        }
    }
    #[tokio::test]
    async fn copy_reports_cumulative_progress_and_flushes_everything() {
        let source = vec![7u8; 150 * 1024];
        let mut output = Vec::new();
        let progress = Recorder::default();
        let bytes = copy(source.as_slice(), &mut output, &progress)
            .await
            .unwrap();
        assert_eq!(bytes, source.len() as u64);
        assert_eq!(output, source);
        let seen = progress.seen.lock().unwrap().clone();
        assert!(seen.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(seen.last(), Some(&bytes));
    }
    #[tokio::test]
    async fn copy_stops_when_the_owner_says_so() {
        let source = vec![1u8; 200 * 1024];
        let progress = Recorder {
            stop_after: Some(1),
            ..Recorder::default()
        };
        let mut output = Vec::new();
        assert!(
            copy(source.as_slice(), &mut output, &progress)
                .await
                .is_err()
        );
        assert!(output.len() < source.len());
    }
    #[test]
    fn transfer_ids_are_safe_in_remote_temporary_names() {
        assert!(validate_id("abc-DEF_123").is_ok());
        for id in ["", "../x", "a/b", "a b", "a.partial"] {
            assert!(validate_id(id).is_err(), "{id:?}");
        }
        assert!(validate_id(&"a".repeat(129)).is_err());
        assert_eq!(
            partial_path("/srv/file.txt", "id1"),
            "/srv/file.txt.terminator-id1.partial"
        );
    }
    #[test]
    fn finished_downloads_never_replace_an_existing_local_file() {
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("existing.txt");
        std::fs::write(&local, b"keep").unwrap();
        let temporary = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
        std::fs::write(temporary.path(), b"new").unwrap();
        assert!(publish_local(temporary, &local).is_err());
        assert_eq!(std::fs::read(&local).unwrap(), b"keep");
        let fresh = dir.path().join("fresh.txt");
        let temporary = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
        std::fs::write(temporary.path(), b"new").unwrap();
        publish_local(temporary, &fresh).unwrap();
        assert_eq!(std::fs::read(&fresh).unwrap(), b"new");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}
